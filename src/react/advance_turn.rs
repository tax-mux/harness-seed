//! 外側 advance ループ。

use crate::action::TurnTrace;
use crate::advance::{
    build_phase_note, count_substantive_ok_observations, evidence_deepening_subtask,
    prepare_phase_recalled, prior_evidence_is_thin, restore_base_recalled, AdvancePhaseSummary,
    AdvanceProgress,
};
use crate::brain::AgentBrain;
use crate::context_metrics::TurnContextSummary;
use crate::layer::run_plan_layer;
use crate::plan::{
    format_plan_for_display, is_replan_subtask, PlanProgress, PlanQueue, Subtask,
};
use crate::tasks::TaskRegistry;

use crate::lifecycle::SubtaskOutcome;
use super::{append_trace, ReActError, ReActLoop, SubtaskExecResult, TurnResult};

impl<E: AgentBrain> ReActLoop<E> {
    /// 計画層 → フェーズ逐次実行。各フェーズ前に `recalled` へ進捗を載せ、必要なら session をクリア。
    pub(super) fn run_turn_advance(&mut self, user_input: &str) -> Result<TurnResult, ReActError> {
        let advance = self.config.advance.clone();
        let base_recalled = self.blocks.recalled.clone();

        if self.config.verbose {
            eprintln!("[advance] planning for: {user_input}");
        }
        let turn_refs = self.take_pending_reference_info_for_plan();
        let (mut harness, plan_trace, plan_steps) = run_plan_layer(
            &mut self.plan_brain,
            &mut self.tools,
            &mut self.blocks,
            &self.session,
            user_input,
            self.config.max_steps_plan,
            self.config.verbose,
            self.config.show_prompt,
            self.config.show_tool_output,
            true,
            self.turn_observer.as_ref(),
            self.stop_requested.as_deref(),
            Some(self.memory.as_ref()),
            self.config.memory.recall_max_rounds,
            &self.task_registry,
            self.config.plan_candidate_selection,
            self.config.plan_catalog_max_entries,
            self.config.plan_catalog_max_chars,
        )?;
        Self::merge_turn_reference_info(&mut harness, turn_refs);
        self.apply_harness_from_plan(&mut harness, user_input);
        let plan = harness.plan.clone();
        self.notify_plan_artifact(&plan);
        self.emit_plan_finished(user_input, &plan);
        if self.config.show_plan {
            println!("{}", format_plan_for_display(&plan, &self.task_registry));
        }

        if !plan.needs_execution() {
            restore_base_recalled(&mut self.blocks, &base_recalled);
            let result = self.finish_skip_execution(
                user_input,
                plan,
                harness,
                plan_trace,
                plan_steps,
            )?;
            return Ok(result);
        }

        let budget = advance.max_phases;
        let initial: Vec<Subtask> = plan.subtasks.iter().take(budget).cloned().collect();
        let mut plan_queue = PlanQueue::from_plan(&initial, budget);
        let mut advance_progress = AdvanceProgress::new(user_input, plan.summary.clone());
        let mut plan_progress = PlanProgress::default();
        let mut subtask_results = Vec::new();
        let mut advance_phases = Vec::new();
        let mut combined_trace = plan_trace;
        let mut total_steps = plan_steps;
        let mut final_answer = String::new();
        let mut phase_index = 0usize;
        let mut substantive_ok_obs = 0usize;
        let mut evidence_boost_used = false;
        let min_substantive = advance.min_substantive_obs;

        while let Some(subtask) = plan_queue.pop_next() {
            if self.is_stop_requested() {
                return Err(ReActError::Cancelled);
            }
            // Keep previous-turn memory for the first phase of a turn.
            // When enabled, clear only between phases in the same turn.
            if advance.clear_session_each_phase && phase_index > 0 {
                self.session.clear();
            }
            prepare_phase_recalled(
                &mut self.blocks,
                &base_recalled,
                &advance_progress,
                &plan,
                &subtask,
                &advance,
            );

            if advance.show_phases {
                println!(
                    "--- Advance phase {} (budget {}/{}) ---",
                    subtask.id,
                    plan_queue.consumed_count(),
                    plan_queue.total_budget()
                );
                println!("  goal: {}", subtask.goal);
            }
            if self.config.verbose {
                eprintln!("[advance] phase {}: {}", subtask.id, subtask.goal);
            }

            self.emit_subtask_started(user_input, &plan, &subtask, phase_index);

            if is_replan_subtask(&subtask) {
                let (new_subs, replan_steps, replan_trace) =
                    self.run_replan_subtask(user_input, &subtask)?;
                total_steps += replan_steps;
                append_trace(&mut combined_trace, &replan_trace);
                let note = if new_subs.is_empty()
                    && prior_evidence_is_thin(substantive_ok_obs, min_substantive)
                    && !evidence_boost_used
                    && plan_queue.consumed_count() + plan_queue.pending_len() + 1
                        <= plan_queue.total_budget()
                {
                    evidence_boost_used = true;
                    match plan_queue.splice_from_replan(
                        vec![evidence_deepening_subtask(0)],
                        subtask.id,
                    ) {
                        Ok(n) => {
                            eprintln!(
                                "[advance] replan returned empty with thin evidence — inserted evidence-deepening ({n})"
                            );
                            format!("replan empty; inserted evidence-deepening ({n})")
                        }
                        Err(err) => {
                            eprintln!("[advance] evidence-deepening splice failed: {err}");
                            format!("replan: inserted 0 subtask(s); deepen failed: {err}")
                        }
                    }
                } else {
                    match plan_queue.splice_from_replan(new_subs, subtask.id) {
                        Ok(n) => {
                            if self.config.verbose || self.config.show_task_execution {
                                eprintln!("[replan] spliced {n} subtask(s) after {}", subtask.id);
                            }
                            format!("replan: inserted {n} subtask(s)")
                        }
                        Err(err) => {
                            eprintln!("[replan] {err}");
                            format!("replan failed: {err}")
                        }
                    }
                };
                self.emit_subtask_finished(
                    user_input,
                    &plan,
                    &subtask,
                    &SubtaskOutcome::completed(&note, replan_steps),
                );
                advance_progress.push(subtask.id, subtask.goal.clone(), note.clone());
                plan_progress.push(subtask.id, note.clone());
                subtask_results.push(SubtaskExecResult {
                    id: subtask.id,
                    answer: note.clone(),
                    steps_used: replan_steps,
                    used_step_driver: false,
                });
                advance_phases.push(AdvancePhaseSummary {
                    id: subtask.id,
                    goal: subtask.goal.clone(),
                    answer: note.clone(),
                    steps_used: replan_steps,
                });
                final_answer = note;
                phase_index += 1;
                continue;
            }

            // 先行フェーズがあり実質証拠が薄いとき、結論フェーズの前に一度だけ証拠深化を差し込む。
            if !advance_progress.steps.is_empty()
                && prior_evidence_is_thin(substantive_ok_obs, min_substantive)
                && !evidence_boost_used
                && plan_queue.consumed_count() + plan_queue.pending_len() + 1
                    <= plan_queue.total_budget()
            {
                evidence_boost_used = true;
                let boost = evidence_deepening_subtask(subtask.id.saturating_add(1000));
                if advance.show_phases || self.config.show_task_execution {
                    eprintln!(
                        "[advance] thin prior evidence ({substantive_ok_obs} substantive ok obs, min {min_substantive}) — running evidence-deepening before phase {}",
                        subtask.id
                    );
                }
                prepare_phase_recalled(
                    &mut self.blocks,
                    &base_recalled,
                    &advance_progress,
                    &plan,
                    &boost,
                    &advance,
                );
                if advance.show_phases {
                    println!(
                        "--- Advance phase {} (evidence-deepening) ---",
                        boost.id
                    );
                    println!("  goal: {}", boost.goal);
                }
                self.emit_subtask_started(user_input, &plan, &boost, phase_index);
                if self.config.show_task_execution {
                    println!("--- Exec subtask {} (evidence-deepening) ---", boost.id);
                    println!(
                        "{}",
                        self.task_registry
                            .format_subtask_execution_for_display(&boost)
                    );
                }
                self.prepare_harness_for_subtask(&mut harness, &boost);
                let (boost_exec, boost_driver) =
                    self.run_subtask_exec_audited(user_input, &plan, &boost, &plan_progress)?;
                harness.advance_after_subtask(boost.id);
                self.sync_harness_step_to_blocks(&harness);
                substantive_ok_obs += count_substantive_ok_observations(&boost_exec.trace);
                if self.config.show_task_execution {
                    let mode = if boost_driver { "step-driver" } else { "ReAct" };
                    println!(
                        "  completed via {mode}: {}",
                        TaskRegistry::format_trace_tools_used(&boost_exec.trace)
                    );
                }
                self.emit_subtask_finished(
                    user_input,
                    &plan,
                    &boost,
                    &SubtaskOutcome::completed(&boost_exec.answer, boost_exec.steps_used),
                );
                let boost_note = build_phase_note(
                    boost.id,
                    boost.goal.clone(),
                    boost_exec.answer.clone(),
                    Some(&boost_exec.trace),
                );
                let boost_carry = boost_note.format_structured(advance.max_note_chars);
                advance_progress.push_note(boost_note);
                plan_progress.push(boost.id, boost_carry.clone());
                subtask_results.push(SubtaskExecResult {
                    id: boost.id,
                    answer: boost_carry.clone(),
                    steps_used: boost_exec.steps_used,
                    used_step_driver: boost_driver,
                });
                advance_phases.push(AdvancePhaseSummary {
                    id: boost.id,
                    goal: boost.goal.clone(),
                    answer: boost_carry,
                    steps_used: boost_exec.steps_used,
                });
                total_steps += boost_exec.steps_used;
                append_trace(&mut combined_trace, &boost_exec.trace);
                phase_index += 1;
                // 本命サブタスク用に recalled を作り直す
                if advance.clear_session_each_phase {
                    self.session.clear();
                }
                prepare_phase_recalled(
                    &mut self.blocks,
                    &base_recalled,
                    &advance_progress,
                    &plan,
                    &subtask,
                    &advance,
                );
                if advance.show_phases {
                    println!(
                        "--- Advance phase {} (budget {}/{}) ---",
                        subtask.id,
                        plan_queue.consumed_count(),
                        plan_queue.total_budget()
                    );
                    println!("  goal: {}", subtask.goal);
                }
                self.emit_subtask_started(user_input, &plan, &subtask, phase_index);
            }

            if self.config.show_task_execution {
                println!("--- Exec subtask {} ---", subtask.id);
                println!(
                    "{}",
                    self.task_registry
                        .format_subtask_execution_for_display(&subtask)
                );
            }

            self.prepare_harness_for_subtask(&mut harness, &subtask);
            let (exec, used_driver) =
                self.run_subtask_exec_audited(user_input, &plan, &subtask, &plan_progress)?;
            harness.advance_after_subtask(subtask.id);
            self.sync_harness_step_to_blocks(&harness);
            substantive_ok_obs += count_substantive_ok_observations(&exec.trace);

            if self.config.show_task_execution {
                let mode = if used_driver { "step-driver" } else { "ReAct" };
                println!(
                    "  completed via {mode}: {}",
                    TaskRegistry::format_trace_tools_used(&exec.trace)
                );
            }

            self.emit_subtask_finished(
                user_input,
                &plan,
                &subtask,
                &SubtaskOutcome::completed(&exec.answer, exec.steps_used),
            );
            let phase_note = build_phase_note(
                subtask.id,
                subtask.goal.clone(),
                exec.answer.clone(),
                Some(&exec.trace),
            );
            let phase_carry = phase_note.format_structured(advance.max_note_chars);
            advance_progress.push_note(phase_note);
            plan_progress.push(subtask.id, phase_carry.clone());
            subtask_results.push(SubtaskExecResult {
                id: subtask.id,
                answer: phase_carry.clone(),
                steps_used: exec.steps_used,
                used_step_driver: used_driver,
            });
            advance_phases.push(AdvancePhaseSummary {
                id: subtask.id,
                goal: subtask.goal.clone(),
                answer: phase_carry,
                steps_used: exec.steps_used,
            });
            total_steps += exec.steps_used;
            final_answer = exec.answer;
            append_trace(&mut combined_trace, &exec.trace);
            phase_index += 1;
        }

        let multi_phase = subtask_results.len() >= 2;
        self.maybe_synthesize_advance_answer(
            user_input,
            &plan,
            &subtask_results,
            &mut final_answer,
            &mut combined_trace,
            &mut total_steps,
        )?;
        // 多フェーズ合成済みなら step-driver 向け合成は重ねない
        if !multi_phase {
            self.maybe_synthesize_user_answer(
                user_input,
                &plan,
                &subtask_results,
                &mut final_answer,
                &mut combined_trace,
                &mut total_steps,
            )?;
        }

        restore_base_recalled(&mut self.blocks, &base_recalled);
        self.clear_harness_prompt_blocks();

        let result = TurnResult {
            answer: final_answer,
            context: TurnContextSummary::from_usages(&combined_trace.context_usages),
            trace: combined_trace,
            steps_used: total_steps,
            plan: Some(plan),
            harness: Some(harness),
            subtask_results,
            advance_phases,
        };
        self.finish_turn(user_input, &result);
        Ok(result)
    }

    /// `task: "replan"` — 計画層を再実行し、新しい subtask 列を返す（ネスト replan は落とす）。
    pub(super) fn run_replan_subtask(
        &mut self,
        user_input: &str,
        subtask: &Subtask,
    ) -> Result<(Vec<Subtask>, usize, TurnTrace), ReActError> {
        let replan_input = if subtask.goal.trim().is_empty() {
            format!("{user_input}\n\nReplan: revise remaining work based on completed phases in Recalled context.")
        } else {
            format!("{user_input}\n\nReplan directive: {}", subtask.goal)
        };
        if self.config.verbose {
            eprintln!("[replan] planning: {}", subtask.goal);
        }
        let (harness, trace, steps) = run_plan_layer(
            &mut self.plan_brain,
            &mut self.tools,
            &mut self.blocks,
            &self.session,
            &replan_input,
            self.config.max_steps_plan,
            self.config.verbose,
            self.config.show_prompt,
            self.config.show_tool_output,
            false,
            self.turn_observer.as_ref(),
            self.stop_requested.as_deref(),
            Some(self.memory.as_ref()),
            self.config.memory.recall_max_rounds,
            &self.task_registry,
            self.config.plan_candidate_selection,
            self.config.plan_catalog_max_entries,
            self.config.plan_catalog_max_chars,
        )?;
        let new_subs: Vec<Subtask> = harness
            .plan
            .subtasks
            .into_iter()
            .filter(|s| !is_replan_subtask(s))
            .collect();
        Ok((new_subs, steps, trace))
    }

}
