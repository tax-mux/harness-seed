//! 計画層を 1 回走らせ、実行経路へ渡す準備。

use crate::action::TurnTrace;
use crate::brain::AgentBrain;
use crate::layer::run_plan_layer;
use crate::plan::{format_plan_for_display, PlanArtifact};

use super::{ReActError, ReActLoop};

pub(super) struct PreparedPlan {
    pub harness: crate::harness::HarnessState,
    pub plan: PlanArtifact,
    pub plan_trace: TurnTrace,
    pub plan_steps: usize,
}

impl<E: AgentBrain> ReActLoop<E> {
    /// 計画層を 1 回実行し、表示と Harness 反映まで済ませる。
    pub(super) fn prepare_plan_turn(
        &mut self,
        user_input: &str,
    ) -> Result<PreparedPlan, ReActError> {
        if self.config.verbose {
            eprintln!("[plan] layer loop for: {user_input}");
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
            self.config.show_thinking,
            self.config.verbose,
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
        if self.config.show_thinking {
            eprintln!("[plan] {}", plan.summary);
            for st in &plan.subtasks {
                eprintln!("[plan] #{} {}", st.id, st.goal);
            }
        }
        if self.config.show_plan {
            println!("{}", format_plan_for_display(&plan, &self.task_registry));
        }
        if self.config.verbose {
            eprintln!(
                "[plan] summary={} skip={} subtasks={}",
                plan.summary,
                plan.skip_execution,
                plan.subtasks.len()
            );
        }
        Ok(PreparedPlan {
            harness,
            plan,
            plan_trace,
            plan_steps,
        })
    }
}
