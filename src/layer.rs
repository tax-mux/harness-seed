//! 計画層・実行層で共有する ReAct ループ部品。

use crate::action::{Action, AgentStep, Observation, TurnTrace};
use crate::brain::AgentBrain;
use crate::context::{
    eprintln_step_prompt, format_plan_rule_prompt_preview, format_prompt_messages,
    TurnPromptContext,
};
use crate::context_metrics::TurnContextSummary;
use crate::harness::HarnessState;
use crate::memory::{format_recalled_block, MemoryBridge};
use crate::plan::PlanArtifact;
use crate::react::{ReActError, SubtaskExecResult, TurnResult};
use crate::session::SessionMemory;
use crate::tool::{execute_action, ToolRuntime};
use crate::tool_display::eprintln_tool_execution;
use crate::turn_observer::{
    emit_llm_step, emit_observation_step, emit_phase_started, TurnObserver,
};
use std::sync::atomic::{AtomicBool, Ordering};

/// 計画層の `recall` ステップ既定上限。
pub const DEFAULT_MAX_RECALL_ROUNDS: usize = 2;

/// 1 ループ（計画層・サブタスク実行）あたり許容する `thought` の上限。
pub const DEFAULT_MAX_THOUGHTS: usize = 1;

const THOUGHT_LIMIT_TOOL: &str = "__thought_limit";

fn thought_limit_message(max_thoughts: usize) -> String {
    format!(
        "Thought limit reached ({max_thoughts} per run). \
         Do not emit another thought. Return {{\"step\":\"action\",...}} or {{\"step\":\"answer\",...}}."
    )
}

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

    pub const fn exec(max_steps: usize, max_thoughts: usize) -> Self {
        Self {
            max_steps,
            max_thoughts,
            tools_enabled: true,
            context_label: "step",
        }
    }
}

/// 計画層・実行層共通の ReAct ループ。
///
/// `memory` / `max_recall_rounds` は計画層の [`AgentStep::Recall`] 用（実行層は `max_recall_rounds=0`）。
pub fn run_layer_loop<B: AgentBrain>(
    brain: &mut B,
    tools: &mut ToolRuntime,
    blocks: &mut crate::context::PromptBlocks,
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
    memory: Option<&dyn MemoryBridge>,
    max_recall_rounds: usize,
) -> Result<TurnResult, ReActError> {
    let mut trace = TurnTrace::default();
    let mut recall_rounds = 0usize;

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
        let prompt_ctx = TurnPromptContext::new(blocks, user_input, &trace, session)
            .with_step_budget(steps_used, opts.max_steps);
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
        // ビジョン画像は初回 LLM 呼び出しのみ（以降のステップで base64 を再送しない）
        if steps_used == 1 {
            blocks.clear_vision_attachments();
        }

        match step {
            AgentStep::Thought(thought) => {
                if trace.thoughts.len() < opts.max_thoughts {
                    trace.push_thought(thought);
                } else {
                    let id = tools.allocate_invoke_id();
                    trace.push_action(Action::new(id, THOUGHT_LIMIT_TOOL, serde_json::json!({})));
                    let observation =
                        Observation::failure(id, &thought_limit_message(opts.max_thoughts));
                    emit_observation_step(
                        turn_observer,
                        opts.context_label,
                        steps_used,
                        THOUGHT_LIMIT_TOOL,
                        &observation,
                    );
                    if verbose {
                        eprintln!(
                            "[{}] thought rejected (limit {})",
                            opts.context_label, opts.max_thoughts
                        );
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
            AgentStep::Recall(query) => {
                let query = query.trim().to_string();
                if query.is_empty() {
                    trace.push_thought("recall ignored: empty query".into());
                    continue;
                }
                if max_recall_rounds == 0 || memory.is_none() {
                    trace.push_thought(format!(
                        "recall not available (query={query}); continue without memory search"
                    ));
                    continue;
                }
                if recall_rounds >= max_recall_rounds {
                    trace.push_thought(format!(
                        "recall limit reached ({max_recall_rounds}); plan with current Recalled context"
                    ));
                    continue;
                }
                let Some(mem) = memory else { continue };
                // 計画層 recall は知識チャネルのみ（作業ログ分岐は通さない）
                let hits = crate::memory::recall_knowledge(mem, 5, &query);
                recall_rounds += 1;
                if hits.is_empty() {
                    trace.push_thought(format!(
                        "recall[{recall_rounds}/{max_recall_rounds}] query={query} hits=0"
                    ));
                } else {
                    let block = format_recalled_block("plan recall", &hits, 3200);
                    blocks.push_recalled(block);
                    trace.push_thought(format!(
                        "recall[{recall_rounds}/{max_recall_rounds}] query={query} hits={}",
                        hits.len()
                    ));
                    if verbose {
                        eprintln!(
                            "[{}] recall query={query:?} hits={}",
                            opts.context_label,
                            hits.len()
                        );
                    }
                }
            }
        }
    }

    // 計画層: 長く探索する場所ではない。answer 未達なら「課題解決に妥当な計画」を一度だけ強制する。
    if opts.context_label == "plan" {
        return finalize_plan_without_answer(
            brain,
            blocks,
            session,
            user_input,
            &mut trace,
            opts.max_steps,
            plan,
            subtask_results,
            turn_observer,
            show_prompt,
            verbose,
        );
    }

    // 実行層: 上限到達でも硬失敗せず、trace 根拠で一度だけ answer を強制する。
    finalize_exec_without_answer(
        brain,
        blocks,
        session,
        user_input,
        &mut trace,
        opts.max_steps,
        plan,
        subtask_results,
        turn_observer,
        show_prompt,
        verbose,
    )
}

const PLAN_FINALIZE_DIRECTIVE: &str = "\
Plan step limit reached. Emit {\"step\":\"answer\",\"content\":...} now \
with a plan that appropriately solves the user request. Do not emit thought or recall.";

const EXEC_FINALIZE_DIRECTIVE: &str = "\
Exec step limit reached. Emit {\"step\":\"answer\",\"content\":...} now \
using evidence already in the turn trace. Do not emit thought or action.";

fn fallback_answer_from_trace(user_input: &str, trace: &TurnTrace) -> String {
    let mut out = String::from(
        "Reached the step limit before a dedicated final answer. \
Evidence gathered so far (may be incomplete):\n\n",
    );
    let mut budget = 2_400usize;
    let ok_obs: Vec<_> = trace.observations.iter().filter(|o| o.ok).collect();
    if ok_obs.is_empty() {
        out.push_str("(No successful tool observations were recorded.)\n");
        out.push_str(&format!("\nUser request was: {user_input}\n"));
        return out;
    }
    for obs in ok_obs.iter().rev().take(4).rev() {
        if budget == 0 {
            break;
        }
        let snippet: String = obs.output.chars().take(budget.min(600)).collect();
        let used = snippet.chars().count();
        budget = budget.saturating_sub(used);
        out.push_str(&format!("- {}\n", snippet.replace('\n', " ")));
    }
    out.push_str(&format!("\nUser request was: {user_input}\n"));
    out
}

fn finalize_exec_without_answer<B: AgentBrain>(
    brain: &mut B,
    blocks: &mut crate::context::PromptBlocks,
    session: &SessionMemory,
    user_input: &str,
    trace: &mut TurnTrace,
    max_steps: usize,
    plan: Option<PlanArtifact>,
    subtask_results: Vec<SubtaskExecResult>,
    turn_observer: Option<&TurnObserver>,
    show_prompt: bool,
    verbose: bool,
) -> Result<TurnResult, ReActError> {
    let steps_used = max_steps.saturating_add(1);
    trace.push_thought(EXEC_FINALIZE_DIRECTIVE.into());
    let prompt_ctx = TurnPromptContext::new(blocks, user_input, trace, session)
        .with_step_budget(steps_used, max_steps);
    let step = brain.decide(&prompt_ctx);
    if let Some(usage) = brain.poll_context_usage() {
        if show_prompt {
            eprintln_step_prompt("exec", steps_used, &usage.prompt_body);
        }
        eprintln!("[context exec] {usage}");
        emit_llm_step(turn_observer, "exec", steps_used, &usage, &step);
        trace.push_context_usage(usage);
    }
    if verbose {
        eprintln!("[exec] finalize decide: {step:?}");
    }

    let answer = match step {
        AgentStep::Answer(answer) => {
            eprintln!("[exec] finalized via mandatory answer after step limit");
            answer
        }
        other => {
            let kind = match &other {
                AgentStep::Thought(_) => "thought",
                AgentStep::Action(_) => "action",
                AgentStep::Recall(_) => "recall",
                AgentStep::Answer(_) => "answer",
            };
            eprintln!(
                "[exec] no answer after finalize prompt (got {kind}) — falling back to trace evidence"
            );
            fallback_answer_from_trace(user_input, trace)
        }
    };

    let context = TurnContextSummary::from_usages(&trace.context_usages);
    Ok(TurnResult {
        answer,
        trace: std::mem::take(trace),
        steps_used,
        context,
        plan,
        harness: None,
        subtask_results,
        advance_phases: vec![],
    })
}

fn finalize_plan_without_answer<B: AgentBrain>(
    brain: &mut B,
    blocks: &mut crate::context::PromptBlocks,
    session: &SessionMemory,
    user_input: &str,
    trace: &mut TurnTrace,
    max_steps: usize,
    plan: Option<PlanArtifact>,
    subtask_results: Vec<SubtaskExecResult>,
    turn_observer: Option<&TurnObserver>,
    show_prompt: bool,
    verbose: bool,
) -> Result<TurnResult, ReActError> {
    let steps_used = max_steps.saturating_add(1);
    // Turn trace に載せて decide に渡す（計画層プロンプトの Plan trace に出る）
    trace.push_thought(PLAN_FINALIZE_DIRECTIVE.into());
    let prompt_ctx = TurnPromptContext::new(blocks, user_input, trace, session);
    let step = brain.decide(&prompt_ctx);
    if let Some(usage) = brain.poll_context_usage() {
        if show_prompt {
            eprintln_step_prompt("plan", steps_used, &usage.prompt_body);
        }
        eprintln!("[context plan] {usage}");
        emit_llm_step(turn_observer, "plan", steps_used, &usage, &step);
        trace.push_context_usage(usage);
    }
    if verbose {
        eprintln!("[plan] finalize decide: {step:?}");
    }

    let answer = match step {
        AgentStep::Answer(answer) => {
            eprintln!("[plan] finalized via mandatory answer after step limit");
            answer
        }
        other => {
            let kind = match &other {
                AgentStep::Thought(_) => "thought",
                AgentStep::Action(_) => "action",
                AgentStep::Recall(_) => "recall",
                AgentStep::Answer(_) => "answer",
            };
            trace.push_thought(format!(
                "mandatory answer not produced (got {kind}); freeform exec for user request"
            ));
            eprintln!("[plan] no answer after finalize prompt — freeform exec for user request");
            // 定型ナラティブは載せない。ユーザ要求だけを実行層へ渡す。
            serde_json::to_string(&PlanArtifact::single_subtask(user_input))
                .unwrap_or_else(|_| "{}".into())
        }
    };

    let context = TurnContextSummary::from_usages(&trace.context_usages);
    Ok(TurnResult {
        answer,
        trace: std::mem::take(trace),
        steps_used,
        context,
        plan,
        harness: None,
        subtask_results,
        advance_phases: vec![],
    })
}

/// 計画層ループ → Harness パース → [`HarnessState`]。挨拶等（skip_execution）のみ LLM を呼ばない。
pub fn run_plan_layer<B: AgentBrain>(
    brain: &mut B,
    tools: &mut ToolRuntime,
    blocks: &mut crate::context::PromptBlocks,
    session: &SessionMemory,
    user_input: &str,
    max_steps: usize,
    verbose: bool,
    show_prompt: bool,
    show_tool_output: bool,
    echo_harness_parsed: bool,
    turn_observer: Option<&TurnObserver>,
    stop_requested: Option<&AtomicBool>,
    memory: Option<&dyn MemoryBridge>,
    max_recall_rounds: usize,
    task_registry: &crate::tasks::TaskRegistry,
    plan_candidate_selection: bool,
    plan_catalog_max_entries: usize,
    plan_catalog_max_chars: usize,
) -> Result<(HarnessState, crate::action::TurnTrace, usize), ReActError> {
    if let Some(contract) = &blocks.plan_data_contract {
        if contract.skip_plan_layer() {
            if verbose {
                eprintln!("[plan] trivial chat — skip plan LLM");
            }
            let plan = PlanArtifact::skip_needs_exec("direct chat");
            // WI は内部ラベルのみ。user_reply 無し → exec LLM が雑談応答する
            let harness = HarnessState::new("(trivial chat — plan layer skipped)", plan);
            if echo_harness_parsed {
                harness.eprintln_parsed();
            }
            return Ok((harness, crate::action::TurnTrace::default(), 0));
        }
    }

    // 計画フェーズ内: summary で候補選定 → 詳細カタログをコンテキスト登録
    if plan_candidate_selection {
        let selected = crate::plan::select_and_register_plan_candidates_with_budget(
            brain,
            tools,
            blocks,
            session,
            user_input,
            task_registry,
            verbose,
            show_prompt,
            turn_observer,
            stop_requested,
            plan_catalog_max_entries,
            plan_catalog_max_chars,
        );
        if selected.is_empty() {
            if verbose {
                eprintln!("[plan] candidate selection empty — treat as direct chat");
            }
            let plan = PlanArtifact::skip_needs_exec("direct chat");
            let harness = HarnessState::new("(no task candidates — direct chat)", plan);
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
        memory,
        max_recall_rounds,
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
mod tests;
