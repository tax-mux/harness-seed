//! 計画層・実行層で共有する ReAct ループ部品。

use crate::action::{Action, AgentStep, Observation, TurnTrace};
use crate::brain::AgentBrain;
use crate::context::{
    eprintln_step_prompt, format_plan_rule_prompt_preview, format_prompt_messages,
    TurnPromptContext,
};
use crate::context_metrics::TurnContextSummary;
use crate::harness::HarnessState;
use crate::plan::PlanArtifact;
use crate::react::{ReActError, SubtaskExecResult, TurnResult};
use crate::turn_observer::{emit_llm_step, emit_observation_step, emit_phase_started, TurnObserver};
use crate::session::SessionMemory;
use crate::tool::{execute_action, ToolRuntime};
use crate::tool_display::eprintln_tool_execution;
use std::sync::atomic::{AtomicBool, Ordering};

/// 1 ループ（計画層・サブタスク実行）あたり許容する `thought` の上限。
pub const DEFAULT_MAX_THOUGHTS: usize = 1;

const THOUGHT_LIMIT_TOOL: &str = "__thought_limit";
const THOUGHT_LIMIT_MSG: &str = "Only one thought step is allowed in this run. \
Do not emit another thought. Return {\"step\":\"action\",...} or {\"step\":\"answer\",...}.";

/// 層ごとのループ設定。
#[derive(Debug, Clone, Copy)]
pub struct LayerLoopOptions {
    pub max_steps: usize,
    pub max_thoughts: usize,
    pub tools_enabled: bool,
    pub context_label: &'static str,
}

impl LayerLoopOptions {
    pub const fn plan(max_steps: usize) -> Self {
        Self {
            max_steps,
            max_thoughts: DEFAULT_MAX_THOUGHTS,
            tools_enabled: false,
            context_label: "plan",
        }
    }

    pub const fn exec(max_steps: usize) -> Self {
        Self {
            max_steps,
            max_thoughts: DEFAULT_MAX_THOUGHTS,
            tools_enabled: true,
            context_label: "step",
        }
    }
}

/// 計画層・実行層共通の ReAct ループ。
pub fn run_layer_loop<B: AgentBrain>(
    brain: &mut B,
    tools: &mut ToolRuntime,
    blocks: &crate::context::PromptBlocks,
    session: &SessionMemory,
    user_input: &str,
    opts: LayerLoopOptions,
    verbose: bool,
    show_prompt: bool,
    show_tool_output: bool,
    plan: Option<PlanArtifact>,
    subtask_results: Vec<SubtaskExecResult>,
    turn_observer: Option<&TurnObserver>,
    stop_requested: Option<&AtomicBool>,
) -> Result<TurnResult, ReActError> {
    let mut trace = TurnTrace::default();

    for steps_used in 1..=opts.max_steps {
        if stop_requested
            .map(|t| t.load(Ordering::Relaxed))
            .unwrap_or(false)
        {
            return Err(ReActError::Cancelled);
        }
        if steps_used == 1 {
            let label = match opts.context_label {
                "plan" => "計画を開始しています…",
                _ => "推論を開始しています…",
            };
            emit_phase_started(turn_observer, opts.context_label, label);
        }
        let prompt_ctx = TurnPromptContext::new(blocks, user_input, &trace, session);
        let step = brain.decide(&prompt_ctx);
        if stop_requested
            .map(|t| t.load(Ordering::Relaxed))
            .unwrap_or(false)
        {
            return Err(ReActError::Cancelled);
        }
        if let Some(usage) = brain.poll_context_usage() {
            if show_prompt {
                eprintln_step_prompt(opts.context_label, steps_used, &usage.prompt_body);
            }
            eprintln!("[context {}] {usage}", opts.context_label);
            emit_llm_step(turn_observer, opts.context_label, steps_used, &usage, &step);
            trace.push_context_usage(usage);
        } else if show_prompt {
            let body = if opts.context_label == "plan" {
                format_plan_rule_prompt_preview(&prompt_ctx)
            } else {
                format_prompt_messages(&prompt_ctx.render())
            };
            eprintln_step_prompt(opts.context_label, steps_used, &body);
        }
        if verbose {
            eprintln!("[{}] {step:?}", opts.context_label);
        }

        match step {
            AgentStep::Thought(thought) => {
                if trace.thoughts.len() < opts.max_thoughts {
                    trace.push_thought(thought);
                } else {
                    let id = tools.allocate_invoke_id();
                    trace.push_action(Action::new(id, THOUGHT_LIMIT_TOOL, serde_json::json!({})));
                    let observation = Observation::failure(id, THOUGHT_LIMIT_MSG);
                    emit_observation_step(
                        turn_observer,
                        opts.context_label,
                        steps_used,
                        THOUGHT_LIMIT_TOOL,
                        &observation,
                    );
                    if verbose {
                        eprintln!("[{}] thought rejected (limit {})", opts.context_label, opts.max_thoughts);
                    }
                    trace.push_observation(observation);
                }
            }
            AgentStep::Action(action) => {
                if opts.tools_enabled {
                    let tool_name = action.tool.clone();
                    if stop_requested
                        .map(|t| t.load(Ordering::Relaxed))
                        .unwrap_or(false)
                    {
                        return Err(ReActError::Cancelled);
                    }
                    let observation = execute_action(tools, &action);
                    if stop_requested
                        .map(|t| t.load(Ordering::Relaxed))
                        .unwrap_or(false)
                    {
                        return Err(ReActError::Cancelled);
                    }
                    emit_observation_step(
                        turn_observer,
                        opts.context_label,
                        steps_used,
                        &tool_name,
                        &observation,
                    );
                    if show_tool_output {
                        eprintln_tool_execution(&action, &observation);
                    } else if verbose {
                        eprintln!("{observation:?}");
                    }
                    trace.push_action(action);
                    trace.push_observation(observation);
                } else {
                    let id = action.invoke_id;
                    trace.push_action(action);
                    trace.push_observation(crate::action::Observation::failure(
                        id,
                        "plan layer: tools are not available",
                    ));
                }
            }
            AgentStep::Answer(answer) => {
                if stop_requested
                    .map(|t| t.load(Ordering::Relaxed))
                    .unwrap_or(false)
                {
                    return Err(ReActError::Cancelled);
                }
                let context = TurnContextSummary::from_usages(&trace.context_usages);
                return Ok(TurnResult {
                    answer,
                    trace,
                    steps_used,
                    context,
                    plan,
                    harness: None,
                    subtask_results,
                    advance_phases: vec![],
                });
            }
        }
    }

    Err(ReActError::MaxStepsExceeded {
        limit: opts.max_steps,
    })
}

/// 計画層ループ → Harness パース → [`HarnessState`]。挨拶等（skip_execution）のみ LLM を呼ばない。
pub fn run_plan_layer<B: AgentBrain>(
    brain: &mut B,
    tools: &mut ToolRuntime,
    blocks: &crate::context::PromptBlocks,
    session: &SessionMemory,
    user_input: &str,
    max_steps: usize,
    verbose: bool,
    show_prompt: bool,
    show_tool_output: bool,
    echo_harness_parsed: bool,
    turn_observer: Option<&TurnObserver>,
    stop_requested: Option<&AtomicBool>,
) -> Result<(HarnessState, crate::action::TurnTrace, usize), ReActError> {
    if let Some(contract) = &blocks.plan_data_contract {
        if contract.skip_plan_layer() {
            if verbose {
                eprintln!("[plan] trivial chat — skip plan LLM");
            }
            let plan = PlanArtifact {
                summary: "direct chat".into(),
                skip_execution: true,
                subtasks: vec![],
            };
            let harness = HarnessState::new("(trivial chat — plan layer skipped)", plan);
            if echo_harness_parsed {
                harness.eprintln_parsed();
            }
            return Ok((harness, crate::action::TurnTrace::default(), 0));
        }
    }
    let turn = run_layer_loop(
        brain,
        tools,
        blocks,
        session,
        user_input,
        LayerLoopOptions::plan(max_steps),
        verbose,
        show_prompt,
        show_tool_output,
        None,
        vec![],
        turn_observer,
        stop_requested,
    )?;
    let harness = match crate::harness::parse_harness_strict(&turn.answer, user_input) {
        Ok(harness) => harness,
        Err(err) => {
            return Err(ReActError::PlanParseFailed {
                message: err.to_string(),
            });
        }
    };
    if echo_harness_parsed {
        harness.eprintln_parsed();
    }
    Ok((harness, turn.trace, turn.steps_used))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brain::AgentBrain;
    use crate::context::PromptBlocks;
    use crate::session::SessionMemory;

    struct SeqBrain {
        steps: Vec<AgentStep>,
        index: usize,
    }

    impl AgentBrain for SeqBrain {
        fn decide(&mut self, _ctx: &TurnPromptContext<'_>) -> AgentStep {
            let step = self
                .steps
                .get(self.index)
                .cloned()
                .unwrap_or_else(|| AgentStep::Answer("fallback".into()));
            self.index += 1;
            step
        }
    }

    #[test]
    fn rejects_second_thought_with_loop_guard_observation() {
        let mut brain = SeqBrain {
            steps: vec![
                AgentStep::Thought("first".into()),
                AgentStep::Thought("second".into()),
                AgentStep::Answer("done".into()),
            ],
            index: 0,
        };
        let mut tools = ToolRuntime::from_registry(
            crate::runtime::RuntimeEnvironment::detect(),
            None,
            crate::tool::full_builtin_registry(false),
        );
        let blocks = PromptBlocks::default();
        let session = SessionMemory::default();

        let turn = run_layer_loop(
            &mut brain,
            &mut tools,
            &blocks,
            &session,
            "test",
            LayerLoopOptions::exec(8),
            false,
            false,
            false,
            None,
            vec![],
            None,
            None,
        )
        .unwrap();

        assert_eq!(turn.answer, "done");
        assert_eq!(turn.trace.thoughts.len(), 1);
        assert_eq!(turn.trace.thoughts[0], "first");
        assert_eq!(turn.trace.actions.len(), 1);
        assert_eq!(turn.trace.actions[0].tool, THOUGHT_LIMIT_TOOL);
        assert!(
            turn.trace
                .observations
                .iter()
                .any(|o| !o.ok && o.output.contains("Only one thought"))
        );
    }

    #[test]
    fn plain_text_plan_output_falls_back_generically() {
        let mut brain = SeqBrain {
            steps: vec![AgentStep::Answer("自己紹介します。私はハーネスの案内役です。".into())],
            index: 0,
        };
        let mut tools = ToolRuntime::from_registry(
            crate::runtime::RuntimeEnvironment::detect(),
            None,
            crate::tool::full_builtin_registry(false),
        );
        let blocks = PromptBlocks::default();
        let session = SessionMemory::default();

        let (harness, trace, steps_used) = run_plan_layer(
            &mut brain,
            &mut tools,
            &blocks,
            &session,
            "自己紹介して",
            4,
            false,
            false,
            false,
            false,
            None,
            None,
        )
        .unwrap();

        assert!(harness.plan.skip_execution);
        assert_eq!(harness.plan.subtasks.len(), 0);
        assert_eq!(steps_used, 1);
        assert!(trace.thoughts.is_empty());
    }
}
