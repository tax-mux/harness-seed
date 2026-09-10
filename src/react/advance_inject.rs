//! advance の挿入フェーズ（証拠深化・主張監査・replan）。
use crate::action::TurnTrace;
use crate::advance::{
    build_phase_note, count_substantive_ok_observations, count_substantive_tool_attempts,
    prepare_phase_recalled, AdvanceConfig, AdvancePhaseSummary, AdvanceProgress,
};
use crate::brain::AgentBrain;
use crate::harness::HarnessState;
use crate::layer::run_plan_layer;
use crate::plan::{is_replan_subtask, PlanArtifact, PlanProgress, Subtask};

use super::{append_trace, ReActError, ReActLoop, SubtaskExecResult};
use crate::lifecycle::SubtaskOutcome;
use crate::tasks::TaskRegistry;

impl<E: AgentBrain> ReActLoop<E> {
    pub(super) fn run_injected_advance_phase(
        &mut self,
        user_input: &str,
        plan: &PlanArtifact,
        harness: &mut HarnessState,
        advance: &AdvanceConfig,
        base_recalled: &[String],
        advance_progress: &mut AdvanceProgress,
        plan_progress: &mut PlanProgress,
        subtask_results: &mut Vec<SubtaskExecResult>,
        advance_phases: &mut Vec<AdvancePhaseSummary>,
        combined_trace: &mut TurnTrace,
        total_steps: &mut usize,
        phase_index: &mut usize,
        substantive_ok_obs: &mut usize,
        boost: &Subtask,
        label: &str,
    ) -> Result<(usize, usize), ReActError> {
        prepare_phase_recalled(
            &mut self.blocks,
            base_recalled,
            advance_progress,
            plan,
            boost,
            advance,
        );
        if advance.show_phases {
            println!("--- Advance phase {} ({label}) ---", boost.id);
            println!("  goal: {}", boost.goal);
        }
        self.emit_subtask_started(user_input, plan, boost, *phase_index);
        if self.config.show_task_execution {
            println!("--- Exec subtask {} ({label}) ---", boost.id);
            println!(
                "{}",
                self.task_registry
                    .format_subtask_execution_for_display(boost)
            );
        }
        self.prepare_harness_for_subtask(harness, boost);
        let is_claim_audit = label.starts_with("claim-falsification");
        let (boost_exec, boost_driver) = if is_claim_audit {
            let max_steps = advance
                .claim_check_max_steps
                .min(self.config.max_steps)
                .max(2);
            let sterile = Some(advance.claim_check_sterile_run_cmd_limit);
            self.run_subtask_exec_with_opts(
                user_input,
                plan,
                boost,
                plan_progress,
                max_steps,
                sterile,
            )?
        } else {
            self.run_subtask_exec_audited(user_input, plan, boost, plan_progress)?
        };
        harness.advance_after_subtask(boost.id);
        self.sync_harness_step_to_blocks(harness);
        let gained = count_substantive_ok_observations(&boost_exec.trace);
        let tool_attempts = count_substantive_tool_attempts(&boost_exec.trace);
        *substantive_ok_obs += gained;
        if self.config.show_task_execution {
            let mode = if boost_driver { "step-driver" } else { "ReAct" };
            println!(
                "  completed via {mode}: {}",
                TaskRegistry::format_trace_tools_used(&boost_exec.trace)
            );
        }
        self.emit_subtask_finished(
            user_input,
            plan,
            boost,
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
        *total_steps += boost_exec.steps_used;
        append_trace(combined_trace, &boost_exec.trace);
        *phase_index += 1;
        // 返り値は「中身のある証拠」数。再試行判定は呼び出し側で tool_attempts も見る。
        let _ = tool_attempts;
        Ok((gained, tool_attempts))
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
            self.config.show_thinking,
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
