//! `skip_execution` 完了経路。

use crate::action::TurnTrace;
use crate::brain::AgentBrain;
use crate::context_metrics::TurnContextSummary;
use crate::harness::HarnessState;
use crate::plan::PlanArtifact;

use super::{append_trace, ReActError, ReActLoop, TurnResult};

impl<E: AgentBrain> ReActLoop<E> {
    /// `skip_execution` 計画: exec LLM を呼ばず plan の output/summary を最終回答にする。
    pub(super) fn finish_skip_execution(
        &mut self,
        user_input: &str,
        plan: PlanArtifact,
        harness: HarnessState,
        plan_trace: TurnTrace,
        plan_steps: usize,
    ) -> Result<TurnResult, ReActError> {
        if let Some(answer) = plan.direct_reply(&harness.work_instructions) {
            if self.config.verbose {
                eprintln!("[plan] skip_execution — direct reply (no exec LLM)");
            }
            let result = TurnResult {
                answer,
                context: TurnContextSummary::from_usages(&plan_trace.context_usages),
                trace: plan_trace,
                steps_used: plan_steps,
                plan: Some(plan),
                harness: Some(harness),
                subtask_results: vec![],
                advance_phases: vec![],
            };
            self.clear_harness_prompt_blocks();
            self.finish_turn(user_input, &result);
            return Ok(result);
        }
        // plan に使える本文が無いときだけ従来どおり exec にフォールバック
        if self.config.verbose {
            eprintln!("[plan] skip_execution — no direct reply, fallback to exec LLM");
        }
        self.blocks.work_instructions_text =
            Some(harness.format_work_instructions_for_prompt());
        let mut result =
            self.run_turn_single(user_input, true, Some(plan), vec![])?;
        append_trace(&mut result.trace, &plan_trace);
        result.context = TurnContextSummary::from_usages(&result.trace.context_usages);
        result.steps_used += plan_steps;
        result.harness = Some(harness);
        result.advance_phases.clear();
        self.clear_harness_prompt_blocks();
        Ok(result)
    }

}
