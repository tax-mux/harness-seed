//! 契約順ステップドライバと依存波の実行。
use crate::action::TurnTrace;
use crate::brain::AgentBrain;
use crate::context_metrics::TurnContextSummary;
use crate::harness::HarnessState;
use crate::plan::{format_mission, PlanArtifact, PlanProgress, Subtask};
use crate::tasks::TaskRegistry;
use crate::tool::ToolRuntime;

use super::{
    append_trace, ReActError, ReActLoop, SubtaskExecResult, TurnResult, SUBTASK_AUDIT_MAX_ATTEMPTS,
};
use crate::lifecycle::SubtaskOutcome;

impl<E: AgentBrain> ReActLoop<E> {
    /// 1 依存波を実行する。`parallel_subtasks` 時はステップドライバ契約タスクを並列化。
    pub(super) fn run_subtask_wave(
        &mut self,
        user_input: &str,
        plan: &PlanArtifact,
        wave: &[Subtask],
        index: &mut usize,
        progress: &mut PlanProgress,
        harness: &mut HarnessState,
        subtask_results: &mut Vec<SubtaskExecResult>,
        total_steps: &mut usize,
        final_answer: &mut String,
        combined_trace: &mut TurnTrace,
    ) -> Result<(), ReActError> {
        let parallel_drivers = self.config.parallel_subtasks
            && wave.len() > 1
            && wave
                .iter()
                .any(|st| self.config.use_step_driver && self.task_registry.use_step_driver(st));

        if parallel_drivers {
            let mut drivers = Vec::new();
            let mut reacts = Vec::new();
            for st in wave {
                let idx = *index;
                *index += 1;
                if self.config.use_step_driver && self.task_registry.use_step_driver(st) {
                    drivers.push((idx, st.clone()));
                } else {
                    reacts.push((idx, st.clone()));
                }
            }

            for (idx, st) in &drivers {
                self.emit_subtask_started(user_input, plan, st, *idx);
                if self.config.show_task_execution {
                    println!("--- Exec subtask {} (parallel driver) ---", st.id);
                }
            }

            let wave_progress = progress.clone();
            let registry = self.task_registry.clone();
            let verbose = self.config.verbose;
            let show_tool_output = self.config.show_tool_output;
            let show_thinking = self.config.show_thinking;
            let arg_mode = self.config.arg_audit_mode;
            let env = self.tools.environment().clone();
            let brave = self.brave_search.clone();
            let packs = self.tool_packs.clone();

            let driver_outcomes: Vec<(Subtask, Result<(TurnResult, bool), String>)> =
                std::thread::scope(|scope| {
                    let mut handles = Vec::with_capacity(drivers.len());
                    let join_fallback: Vec<Subtask> =
                        drivers.iter().map(|(_, st)| st.clone()).collect();
                    for (_idx, st) in drivers {
                        let registry = registry.clone();
                        let packs = packs.clone();
                        let brave = brave.clone();
                        let env = env.clone();
                        handles.push(scope.spawn(move || {
                            let st_id = st.id;
                            let outcome =
                                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    let mut tools = ToolRuntime::with_packs(env, brave, &packs);
                                    registry
                                        .run_subtask_driver(
                                            &st,
                                            &mut tools,
                                            verbose,
                                            show_tool_output,
                                            show_thinking,
                                            arg_mode,
                                        )
                                        .map(|drv| {
                                            (
                                                TurnResult {
                                                    answer: drv.answer,
                                                    context: TurnContextSummary::default(),
                                                    trace: drv.trace,
                                                    steps_used: drv.steps_used,
                                                    plan: None,
                                                    harness: None,
                                                    subtask_results: vec![],
                                                    advance_phases: vec![],
                                                },
                                                true,
                                            )
                                        })
                                        .map_err(|e| e.to_string())
                                }))
                                .unwrap_or_else(|_| {
                                    Err(format!("parallel driver panicked (subtask {st_id})"))
                                });
                            (st, outcome)
                        }));
                    }
                    handles
                        .into_iter()
                        .enumerate()
                        .map(|(i, h)| {
                            h.join().unwrap_or_else(|_| {
                                (
                                    join_fallback[i].clone(),
                                    Err(format!(
                                        "parallel driver thread aborted (subtask {})",
                                        join_fallback[i].id
                                    )),
                                )
                            })
                        })
                        .collect()
                });

            for (st, outcome) in driver_outcomes {
                let (exec, used_driver) = match outcome {
                    Ok(v) => v,
                    Err(err) => {
                        // ドライバ失敗・スレッドパニック時はメインで ReAct にフォールバック
                        if self.config.verbose
                            || err.contains("panicked")
                            || err.contains("aborted")
                        {
                            eprintln!(
                                "[driver] subtask {} failed ({err}); falling back to ReAct",
                                st.id
                            );
                        }
                        self.run_subtask_exec_audited(user_input, plan, &st, &wave_progress)?
                    }
                };
                self.finish_subtask_outcome(
                    user_input,
                    plan,
                    &st,
                    exec,
                    used_driver,
                    progress,
                    harness,
                    subtask_results,
                    total_steps,
                    final_answer,
                    combined_trace,
                );
            }

            for (idx, st) in reacts {
                self.run_one_subtask_serial(
                    user_input,
                    plan,
                    &st,
                    idx,
                    progress,
                    harness,
                    subtask_results,
                    total_steps,
                    final_answer,
                    combined_trace,
                )?;
            }
            return Ok(());
        }

        for st in wave {
            let idx = *index;
            *index += 1;
            self.run_one_subtask_serial(
                user_input,
                plan,
                st,
                idx,
                progress,
                harness,
                subtask_results,
                total_steps,
                final_answer,
                combined_trace,
            )?;
        }
        Ok(())
    }

    pub(super) fn run_one_subtask_serial(
        &mut self,
        user_input: &str,
        plan: &PlanArtifact,
        subtask: &Subtask,
        idx: usize,
        progress: &mut PlanProgress,
        harness: &mut HarnessState,
        subtask_results: &mut Vec<SubtaskExecResult>,
        total_steps: &mut usize,
        final_answer: &mut String,
        combined_trace: &mut TurnTrace,
    ) -> Result<(), ReActError> {
        if self.is_stop_requested() {
            return Err(ReActError::Cancelled);
        }
        self.emit_subtask_started(user_input, plan, subtask, idx);
        if self.config.show_task_execution {
            println!("--- Exec subtask {} ---", subtask.id);
            println!(
                "{}",
                self.task_registry
                    .format_subtask_execution_for_display(subtask)
            );
        }
        if self.config.verbose {
            eprintln!("[exec] subtask {}: {}", subtask.id, subtask.goal);
        }
        self.prepare_harness_for_subtask(harness, subtask);
        let (exec, used_driver) =
            self.run_subtask_exec_audited(user_input, plan, subtask, progress)?;
        self.finish_subtask_outcome(
            user_input,
            plan,
            subtask,
            exec,
            used_driver,
            progress,
            harness,
            subtask_results,
            total_steps,
            final_answer,
            combined_trace,
        );
        Ok(())
    }

    pub(super) fn finish_subtask_outcome(
        &mut self,
        user_input: &str,
        plan: &PlanArtifact,
        subtask: &Subtask,
        exec: TurnResult,
        used_driver: bool,
        progress: &mut PlanProgress,
        harness: &mut HarnessState,
        subtask_results: &mut Vec<SubtaskExecResult>,
        total_steps: &mut usize,
        final_answer: &mut String,
        combined_trace: &mut TurnTrace,
    ) {
        harness.advance_after_subtask(subtask.id);
        self.sync_harness_step_to_blocks(harness);
        if self.config.show_task_execution {
            let mode = if used_driver { "step-driver" } else { "ReAct" };
            println!(
                "  completed via {mode}: {}",
                TaskRegistry::format_trace_tools_used(&exec.trace)
            );
        }
        self.emit_subtask_finished(
            user_input,
            plan,
            subtask,
            &SubtaskOutcome::completed(&exec.answer, exec.steps_used),
        );
        *total_steps += exec.steps_used;
        progress.push(subtask.id, exec.answer.clone());
        subtask_results.push(SubtaskExecResult {
            id: subtask.id,
            answer: exec.answer.clone(),
            steps_used: exec.steps_used,
            used_step_driver: used_driver,
        });
        *final_answer = exec.answer;
        append_trace(combined_trace, &exec.trace);
    }

    /// サブタスク 1 件を実行し、タスク契約の監査で完了を検証する（未達なら同一サブタスクを再実行）。
    pub(super) fn run_subtask_exec_audited(
        &mut self,
        user_input: &str,
        plan: &PlanArtifact,
        subtask: &Subtask,
        progress: &PlanProgress,
    ) -> Result<(TurnResult, bool), ReActError> {
        let mut last: Option<(TurnResult, bool)> = None;
        let mut audit_msg = String::new();

        for attempt in 1..=SUBTASK_AUDIT_MAX_ATTEMPTS {
            if self.is_stop_requested() {
                return Err(ReActError::Cancelled);
            }
            let (exec, used_driver) = if audit_msg.is_empty() {
                self.run_subtask_exec(user_input, plan, subtask, progress)?
            } else {
                let base = format_mission(&self.task_registry, user_input, plan, subtask, progress);
                let mission = format!(
                    "{base}\n\n## Subtask audit (retry {attempt})\n\
                     The previous run did NOT satisfy the task execution contract.\n\
                     {audit_msg}\n\
                     Call every required tool in order before emitting answer.\n"
                );
                let exec = self.run_turn_single(&mission, false, None, vec![])?;
                (exec, false)
            };

            let audit = self.task_registry.audit_subtask_with_mode(
                subtask,
                &exec.trace,
                self.config.arg_audit_mode,
            );
            let complete = audit.as_ref().map(|a| a.complete).unwrap_or(true);
            if self.config.verbose {
                if let Some(a) = &audit {
                    eprintln!(
                        "[tasks] subtask {} audit (attempt {attempt}): complete={} — {}",
                        subtask.id, a.complete, a.message
                    );
                }
            }
            if complete {
                return Ok((exec, used_driver));
            }
            audit_msg = audit
                .map(|a| a.message)
                .unwrap_or_else(|| "contract not satisfied".into());
            last = Some((exec, used_driver));
        }

        Ok(last.expect("subtask exec attempts"))
    }

    /// サブタスク 1 件: 契約ありならステップドライバ、それ以外は実行層 ReAct。
    pub(super) fn run_subtask_exec(
        &mut self,
        user_input: &str,
        plan: &PlanArtifact,
        subtask: &Subtask,
        progress: &PlanProgress,
    ) -> Result<(TurnResult, bool), ReActError> {
        if self.config.use_step_driver && self.task_registry.use_step_driver(subtask) {
            match self.task_registry.run_subtask_driver(
                subtask,
                &mut self.tools,
                self.config.verbose,
                self.config.show_tool_output,
                self.config.show_thinking,
                self.config.arg_audit_mode,
            ) {
                Ok(drv) => {
                    if self.config.verbose {
                        eprintln!(
                            "[driver] subtask {} task={} steps={} audit_ok={}",
                            subtask.id, drv.task_id, drv.steps_used, drv.audit.complete
                        );
                    }
                    return Ok((
                        TurnResult {
                            answer: drv.answer,
                            context: TurnContextSummary::default(),
                            trace: drv.trace,
                            steps_used: drv.steps_used,
                            plan: None,
                            harness: None,
                            subtask_results: vec![],
                            advance_phases: vec![],
                        },
                        true,
                    ));
                }
                Err(err) => {
                    if self.is_stop_requested() {
                        return Err(ReActError::Cancelled);
                    }
                    if self.config.verbose {
                        eprintln!(
                            "[driver] subtask {} failed ({err}); falling back to ReAct",
                            subtask.id
                        );
                    }
                }
            }
        }
        let mut mission = format_mission(&self.task_registry, user_input, plan, subtask, progress);
        if let Some(task_id) = &subtask.task {
            if let Some(def) = self.task_registry.get(task_id) {
                if let Some(spec) = &def.context_manifest {
                    if let Some(manifest_path) = self.blocks.context_manifest_path.clone() {
                        if let Some(params) = self.task_registry.merged_subtask_params(subtask) {
                            match crate::context_manifest::apply_scoped_entry(
                                &manifest_path,
                                spec,
                                &params,
                                &mut self.blocks,
                            ) {
                                Ok(n) if self.config.verbose && n > 0 => {
                                    eprintln!(
                                        "[context-manifest] task {task_id}: {n} image(s) + recalled file(s)"
                                    );
                                }
                                Err(e) => {
                                    eprintln!("[context-manifest] task {task_id}: {e}");
                                    mission.push_str(
                                        &crate::context_manifest::format_apply_error_hint(&e, spec),
                                    );
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
        }
        let saved_catalog = self.blocks.tool_catalog.clone();
        let available: std::collections::HashSet<String> =
            self.tools.registry().names().into_iter().collect();
        let policy = self
            .task_registry
            .tool_policy_for_subtask_with_tools(subtask, Some(&available));
        if let Some(ref p) = policy {
            self.blocks.tool_catalog = self.tools.format_catalog_filtered(Some(p));
            self.tools.set_exec_policy(Some(p.clone()));
        } else {
            self.tools.set_exec_policy(None);
        }
        let exec_result = self.run_turn_single(&mission, false, None, vec![]);
        self.blocks.tool_catalog = saved_catalog;
        self.tools.set_exec_policy(None);
        let exec = exec_result?;
        self.blocks.clear_vision_attachments();
        Ok((exec, false))
    }
}
