//! 計画層・実行層で共有する ReAct ループ部品。

use crate::action::{AgentStep, TurnTrace};
use crate::brain::AgentBrain;
use crate::context::{
    eprintln_step_prompt, format_plan_rule_prompt_preview, format_prompt_messages,
    TurnPromptContext,
};
use crate::context_metrics::TurnContextSummary;
use crate::plan::artifact_from_plan_turn;
use crate::plan::PlanArtifact;
use crate::react::{ReActError, SubtaskExecResult, TurnResult};
use crate::session::SessionMemory;
use crate::tool::{execute_action, ToolRuntime};
use crate::tool_display::eprintln_tool_execution;

/// 層ごとのループ設定。
#[derive(Debug, Clone, Copy)]
pub struct LayerLoopOptions {
    pub max_steps: usize,
    pub tools_enabled: bool,
    pub context_label: &'static str,
}

impl LayerLoopOptions {
    pub const fn plan(max_steps: usize) -> Self {
        Self {
            max_steps,
            tools_enabled: false,
            context_label: "plan",
        }
    }

    pub const fn exec(max_steps: usize) -> Self {
        Self {
            max_steps,
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
) -> Result<TurnResult, ReActError> {
    let mut trace = TurnTrace::default();

    for steps_used in 1..=opts.max_steps {
        let prompt_ctx = TurnPromptContext::new(blocks, user_input, &trace, session);
        let step = brain.decide(&prompt_ctx);
        if let Some(usage) = brain.poll_context_usage() {
            if show_prompt {
                eprintln_step_prompt(opts.context_label, steps_used, &usage.prompt_body);
            }
            eprintln!("[context {}] {usage}", opts.context_label);
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
            AgentStep::Thought(thought) => trace.push_thought(thought),
            AgentStep::Action(action) => {
                if opts.tools_enabled {
                    let observation = execute_action(tools, &action);
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
                let context = TurnContextSummary::from_usages(&trace.context_usages);
                return Ok(TurnResult {
                    answer,
                    trace,
                    steps_used,
                    context,
                    plan,
                    subtask_results,
                });
            }
        }
    }

    Err(ReActError::MaxStepsExceeded {
        limit: opts.max_steps,
    })
}

/// 計画層ループ → [`PlanArtifact`]。
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
) -> Result<(PlanArtifact, usize), ReActError> {
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
    )?;
    let artifact = artifact_from_plan_turn(&turn.answer, user_input);
    Ok((artifact, turn.steps_used))
}
