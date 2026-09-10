//! ライフサイクル hook 発火と Harness 固定ゾーン同期。
use crate::brain::AgentBrain;
use crate::harness::HarnessState;
use crate::lifecycle::{
    invoke_lifecycle, HostView, RunStatus, SubtaskOutcome, TurnOutcome, WriteScope,
};
use crate::plan::{PlanArtifact, Subtask};

use super::{ReActError, ReActLoop};

impl<E: AgentBrain> ReActLoop<E> {
    pub(super) fn emit_turn_started(&mut self, user_input: &str) {
        let Some(h) = self.lifecycle.clone() else {
            return;
        };
        invoke_lifecycle("on_turn_started", || {
            h.on_turn_started(
                user_input,
                HostView::new(&mut self.host_scratch, WriteScope::Turn),
            );
        });
    }

    pub(super) fn emit_plan_finished(&mut self, user_input: &str, plan: &PlanArtifact) {
        self.lifecycle_plan = Some(plan.clone());
        let Some(h) = self.lifecycle.clone() else {
            return;
        };
        invoke_lifecycle("on_plan_finished", || {
            h.on_plan_finished(
                user_input,
                plan,
                HostView::new(&mut self.host_scratch, WriteScope::Turn),
            );
        });
    }

    pub(super) fn emit_subtask_started(
        &mut self,
        user_input: &str,
        plan: &PlanArtifact,
        subtask: &Subtask,
        index: usize,
    ) {
        self.lifecycle_open_subtasks.push((subtask.clone(), index));
        let Some(h) = self.lifecycle.clone() else {
            return;
        };
        let id = subtask.id;
        invoke_lifecycle("on_subtask_started", || {
            h.on_subtask_started(
                user_input,
                plan,
                subtask,
                index,
                HostView::new(&mut self.host_scratch, WriteScope::Subtask(id)),
            );
        });
    }

    pub(super) fn emit_subtask_finished(
        &mut self,
        user_input: &str,
        plan: &PlanArtifact,
        subtask: &Subtask,
        outcome: &SubtaskOutcome,
    ) {
        self.lifecycle_open_subtasks
            .retain(|(s, _)| s.id != subtask.id);
        let Some(h) = self.lifecycle.clone() else {
            return;
        };
        let id = subtask.id;
        invoke_lifecycle("on_subtask_finished", || {
            h.on_subtask_finished(
                user_input,
                plan,
                subtask,
                outcome,
                HostView::new(&mut self.host_scratch, WriteScope::Subtask(id)),
            );
        });
    }

    pub(super) fn emit_turn_finished(
        &mut self,
        user_input: &str,
        plan: Option<&PlanArtifact>,
        outcome: &TurnOutcome,
    ) {
        self.lifecycle_open_subtasks.clear();
        let Some(h) = self.lifecycle.clone() else {
            return;
        };
        invoke_lifecycle("on_turn_finished", || {
            h.on_turn_finished(
                user_input,
                plan,
                outcome,
                HostView::new(&mut self.host_scratch, WriteScope::Turn),
            );
        });
    }

    /// 開始済み未完了のサブタスクとターンを Failed / Cancelled で閉じる。
    pub(super) fn finalize_lifecycle_on_error(&mut self, user_input: &str, err: &ReActError) {
        let status = match err {
            ReActError::Cancelled => RunStatus::Cancelled,
            _ => RunStatus::Failed,
        };
        let message = err.to_string();
        let plan = self.lifecycle_plan.clone();
        let open: Vec<(Subtask, usize)> = self.lifecycle_open_subtasks.drain(..).collect();
        if let Some(ref plan) = plan {
            let outcome = SubtaskOutcome {
                status,
                message: message.clone(),
                steps_used: 0,
            };
            for (subtask, _) in open {
                // retain 済みのため emit の retain は no-op
                self.emit_subtask_finished(user_input, plan, &subtask, &outcome);
            }
        }
        let turn = TurnOutcome {
            status,
            answer: message,
            steps_used: 0,
        };
        self.emit_turn_finished(user_input, plan.as_ref(), &turn);
    }

    /// 計画フェーズの Harness パース結果をプロンプト固定ゾーンへ反映する。
    pub(super) fn apply_harness_from_plan(&mut self, harness: &mut HarnessState, user_input: &str) {
        self.resolve_plan_for_turn(&mut harness.plan, user_input);
        self.blocks.work_instructions_text = Some(harness.format_work_instructions_for_prompt());
        if harness.total_steps > 0 {
            harness.begin_execution();
        }
        self.sync_harness_step_to_blocks(harness);
        if self.config.verbose {
            eprintln!("[harness] state:\n{}", harness.to_json_pretty());
        }
    }

    pub(super) fn sync_harness_step_to_blocks(&mut self, harness: &HarnessState) {
        self.blocks.current_step_text =
            Some(harness.format_current_step_for_prompt(&self.task_registry));
    }

    pub(super) fn prepare_harness_for_subtask(
        &mut self,
        harness: &mut HarnessState,
        subtask: &Subtask,
    ) {
        harness.current_step = subtask.id;
        let available: std::collections::HashSet<String> =
            self.tools.registry().names().into_iter().collect();
        let policy = self
            .task_registry
            .tool_policy_for_subtask_with_tools(subtask, Some(&available));
        harness.set_tool_set_from_policy(policy.as_ref());
        self.sync_harness_step_to_blocks(harness);
    }

    pub(super) fn clear_harness_prompt_blocks(&mut self) {
        self.blocks.work_instructions_text = None;
        self.blocks.current_step_text = None;
    }
}
