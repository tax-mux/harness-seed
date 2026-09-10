//! 外側 advance ループ。

use crate::action::TurnTrace;
use crate::advance::{
    prepare_phase_recalled, restore_base_recalled, AdvancePhaseSummary, AdvanceProgress,
};
use crate::brain::AgentBrain;
use crate::context_metrics::TurnContextSummary;
use crate::layer::run_plan_layer;
use crate::plan::{
    format_plan_for_display, is_replan_subtask, PlanProgress, PlanQueue, Subtask,
};
use crate::tasks::TaskRegistry;

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
                let note = match plan_queue.splice_from_replan(new_subs, subtask.id) {
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
                };
                self.emit_subtask_finished(
                    user_input,
                    &plan,
                    &subtask,
                    &note,
                    replan_steps,
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
                &exec.answer,
                exec.steps_used,
            );
            advance_progress.push(subtask.id, subtask.goal.clone(), exec.answer.clone());
            plan_progress.push(subtask.id, exec.answer.clone());
            subtask_results.push(SubtaskExecResult {
                id: subtask.id,
                answer: exec.answer.clone(),
                steps_used: exec.steps_used,
                used_step_driver: used_driver,
            });
            advance_phases.push(AdvancePhaseSummary {
                id: subtask.id,
                goal: subtask.goal.clone(),
                answer: exec.answer.clone(),
                steps_used: exec.steps_used,
            });
            total_steps += exec.steps_used;
            final_answer = exec.answer;
            append_trace(&mut combined_trace, &exec.trace);
            phase_index += 1;
        }

        self.maybe_synthesize_user_answer(
            user_input,
            &plan,
            &subtask_results,
            &mut final_answer,
            &mut combined_trace,
            &mut total_steps,
        )?;

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
