//! 計画→実行の two_phase オーケストレーション。

use crate::action::TurnTrace;
use crate::advance::{should_escalate_from_plan, AdvanceMode};
use crate::brain::AgentBrain;
use crate::context_metrics::TurnContextSummary;
use crate::plan::{execution_waves, PlanArtifact, PlanProgress};

use super::plan_turn::PreparedPlan;

use super::synthesis::{
    self, SYNTHESIS_EVIDENCE_ITEM_MAX_CHARS, SYNTHESIS_EVIDENCE_TOTAL_MAX_CHARS,
};
use super::{append_trace, ReActError, ReActLoop, SubtaskExecResult, TurnResult};

impl<E: AgentBrain> ReActLoop<E> {
    /// 計画層 ReAct → 実行層 ReAct（直列）。`from_plan` 時はここで昇格判定する。
    pub(super) fn run_turn_two_phase(
        &mut self,
        user_input: &str,
    ) -> Result<TurnResult, ReActError> {
        let prepared = self.prepare_plan_turn(user_input)?;
        if self.config.advance.mode == AdvanceMode::FromPlan
            && should_escalate_from_plan(&prepared.plan)
        {
            if self.config.verbose {
                eprintln!("[advance] escalate from_plan");
            }
            return self.run_advance_from_prepared(user_input, prepared);
        }
        self.run_two_phase_from_prepared(user_input, prepared)
    }

    pub(super) fn run_two_phase_from_prepared(
        &mut self,
        user_input: &str,
        prepared: PreparedPlan,
    ) -> Result<TurnResult, ReActError> {
        let PreparedPlan {
            mut harness,
            plan,
            plan_trace,
            plan_steps,
        } = prepared;

        if !plan.needs_execution() {
            return self.finish_skip_execution(user_input, plan, harness, plan_trace, plan_steps);
        }

        let waves = execution_waves(&plan.subtasks).map_err(|e| ReActError::ScheduleFailed {
            message: e.to_string(),
        })?;
        let mut progress = PlanProgress::default();
        let mut subtask_results = Vec::new();
        let mut total_steps = plan_steps;
        let mut final_answer = String::new();
        let mut combined_trace = plan_trace;
        let mut index = 0usize;

        for wave in &waves {
            if self.is_stop_requested() {
                return Err(ReActError::Cancelled);
            }
            if self.config.verbose && waves.len() > 1 {
                let ids: Vec<_> = wave.iter().map(|s| s.id).collect();
                eprintln!(
                    "[exec] wave ({} task(s), parallel={}): {ids:?}",
                    wave.len(),
                    self.config.parallel_subtasks
                );
            }
            self.run_subtask_wave(
                user_input,
                &plan,
                wave,
                &mut index,
                &mut progress,
                &mut harness,
                &mut subtask_results,
                &mut total_steps,
                &mut final_answer,
                &mut combined_trace,
            )?;
        }

        self.maybe_synthesize_user_answer(
            user_input,
            &plan,
            &subtask_results,
            &mut final_answer,
            &mut combined_trace,
            &mut total_steps,
        )?;

        self.clear_harness_prompt_blocks();

        let result = TurnResult {
            answer: final_answer,
            context: TurnContextSummary::from_usages(&combined_trace.context_usages),
            trace: combined_trace,
            steps_used: total_steps,
            plan: Some(plan),
            harness: Some(harness),
            subtask_results,
            advance_phases: vec![],
        };
        self.finish_turn(user_input, &result);
        Ok(result)
    }

    /// 最後のサブタスクがステップドライバで、かつ生の answer がユーザー向けでないときだけ合成する。
    pub(super) fn needs_user_answer_synthesis(results: &[SubtaskExecResult]) -> bool {
        let Some(last) = results.last() else {
            return false;
        };
        last.used_step_driver && !synthesis::answer_looks_user_ready(&last.answer)
    }

    pub(super) fn maybe_synthesize_user_answer(
        &mut self,
        user_input: &str,
        plan: &PlanArtifact,
        results: &[SubtaskExecResult],
        final_answer: &mut String,
        combined_trace: &mut TurnTrace,
        total_steps: &mut usize,
    ) -> Result<(), ReActError> {
        if !Self::needs_user_answer_synthesis(results) {
            if self.config.verbose || self.config.show_task_execution {
                if results.last().is_some_and(|r| {
                    r.used_step_driver && synthesis::answer_looks_user_ready(&r.answer)
                }) {
                    eprintln!(
                        "[exec] skipping answer synthesis — step-driver answer already user-ready"
                    );
                }
            }
            return Ok(());
        }
        self.synthesize_grounded_answer(
            user_input,
            plan,
            results,
            final_answer,
            combined_trace,
            total_steps,
            false,
        )
    }

    /// 推進ループでフェーズが 2 以上あるとき、最終回答をフェーズ証拠へ再接地する。
    pub(super) fn maybe_synthesize_advance_answer(
        &mut self,
        user_input: &str,
        plan: &PlanArtifact,
        results: &[SubtaskExecResult],
        final_answer: &mut String,
        combined_trace: &mut TurnTrace,
        total_steps: &mut usize,
    ) -> Result<(), ReActError> {
        if !synthesis::needs_advance_answer_synthesis(results) {
            return Ok(());
        }
        self.synthesize_grounded_answer(
            user_input,
            plan,
            results,
            final_answer,
            combined_trace,
            total_steps,
            true,
        )
    }

    fn synthesize_grounded_answer(
        &mut self,
        user_input: &str,
        plan: &PlanArtifact,
        results: &[SubtaskExecResult],
        final_answer: &mut String,
        combined_trace: &mut TurnTrace,
        total_steps: &mut usize,
        advance_style: bool,
    ) -> Result<(), ReActError> {
        if self.is_stop_requested() {
            return Err(ReActError::Cancelled);
        }
        if self.config.verbose || self.config.show_task_execution {
            if advance_style {
                eprintln!("[advance] synthesizing user-facing answer from multi-phase evidence");
            } else {
                eprintln!("[exec] synthesizing user-facing answer from step-driver evidence");
            }
        }

        let evidence = if advance_style {
            synthesis::build_advance_phase_evidence(
                results,
                SYNTHESIS_EVIDENCE_ITEM_MAX_CHARS,
                SYNTHESIS_EVIDENCE_TOTAL_MAX_CHARS,
            )
        } else {
            synthesis::build_synthesis_evidence(
                results,
                &combined_trace.observations,
                SYNTHESIS_EVIDENCE_ITEM_MAX_CHARS,
                SYNTHESIS_EVIDENCE_TOTAL_MAX_CHARS,
            )
        };

        let grounding = crate::advance::evidence_grounding_rules();
        let claim_audit = crate::advance::claim_audit_rules();
        let mission = if advance_style {
            format!(
                "User request:\n{user_input}\n\nPlan summary: {}\n\n\
Evidence from completed phases (structured Paths / Claims / Open questions when available; do not invent beyond this):\n{evidence}\n\n\
{grounding}\n\
{claim_audit}\n\
Produce the final user-facing reply in clear language based only on the evidence. \
Prefer claims that cite paths or prior-phase findings. \
If a claim-falsification phase labeled items falsified, omit or demote those claims. \
Mark anything not supported as an unverified candidate. \
Prefer {{\"step\":\"answer\",\"content\":\"...\"}} with no tools.",
                plan.summary
            )
        } else {
            format!(
                "User request:\n{user_input}\n\nPlan summary: {}\n\n\
Evidence from completed work (do not invent beyond this):\n{evidence}\n\n\
Reply to the user in clear language based only on the evidence. \
Prefer {{\"step\":\"answer\",\"content\":\"...\"}} with no tools if evidence is sufficient.",
                plan.summary
            )
        };

        let synth = self.run_turn_single(&mission, false, None, vec![])?;
        let mut answer = synth.answer;
        if advance_style {
            if self.config.advance.citation_check {
                let evidence_paths = crate::advance::evidence_paths_from_texts(
                    results.iter().map(|r| r.answer.as_str()),
                );
                answer = crate::advance::apply_citation_gate(&answer, &evidence_paths);
            }
            if self.config.advance.absence_check {
                answer = crate::advance::apply_absence_gate(&answer, combined_trace);
            }
        }
        *final_answer = answer;
        *total_steps += synth.steps_used;
        append_trace(combined_trace, &synth.trace);
        Ok(())
    }
}
