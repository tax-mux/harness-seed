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
use crate::turn_observer::{emit_llm_step, emit_observation_step, emit_phase_started, TurnObserver};
use crate::session::SessionMemory;
use crate::tool::{execute_action, ToolRuntime};
use crate::tool_display::eprintln_tool_execution;
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
                    let observation = Observation::failure(id, &thought_limit_message(opts.max_thoughts));
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
        let mut blocks = PromptBlocks::default();
        let session = SessionMemory::default();

        let turn = run_layer_loop(
            &mut brain,
            &mut tools,
            &mut blocks,
            &session,
            "test",
            LayerLoopOptions::exec(8, 1),
            false,
            false,
            false,
            None,
            vec![],
            None,
            None,
            None,
            0,
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
                .any(|o| !o.ok && o.output.contains("Thought limit reached"))
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
        let mut blocks = PromptBlocks::default();
        let session = SessionMemory::default();

        let (harness, trace, steps_used) = run_plan_layer(
            &mut brain,
            &mut tools,
            &mut blocks,
            &session,
            "自己紹介して",
            4,
            false,
            false,
            false,
            false,
            None,
            None,
            None,
            0,
            &crate::tasks::TaskRegistry::builtin(),
            false,
            40,
            8000,
        )
        .unwrap();

        assert!(harness.plan.skip_execution);
        assert_eq!(harness.plan.subtasks.len(), 0);
        assert_eq!(steps_used, 1);
        assert!(trace.thoughts.is_empty());
    }

    #[test]
    fn plan_recall_injects_memory_hits() {
        use crate::memory::{DiaryEntry, LocalDiaryBridge};

        let mut memory = LocalDiaryBridge::new();
        memory
            .diary(&DiaryEntry {
                user_input: "ファルモ導入".into(),
                summary: "事例メモ".into(),
                answer: "導入成功".into(),
                phases: vec![],
            })
            .unwrap();

        let mut brain = SeqBrain {
            steps: vec![
                AgentStep::Recall("ファルモ".into()),
                AgentStep::Answer(
                    r#"{"summary":"ok","skip_execution":true,"knowledge_sufficient":true,"subtasks":[]}"#.into(),
                ),
            ],
            index: 0,
        };
        let mut tools = ToolRuntime::from_registry(
            crate::runtime::RuntimeEnvironment::detect(),
            None,
            crate::tool::full_builtin_registry(false),
        );
        let mut blocks = PromptBlocks::default();
        let session = SessionMemory::default();

        let (harness, trace, steps_used) = run_plan_layer(
            &mut brain,
            &mut tools,
            &mut blocks,
            &session,
            "続き",
            4,
            false,
            false,
            false,
            false,
            None,
            None,
            Some(&memory),
            2,
            &crate::tasks::TaskRegistry::builtin(),
            false,
            40,
            8000,
        )
        .unwrap();

        assert!(harness.plan.skip_execution);
        assert_eq!(steps_used, 2);
        assert!(blocks
            .recalled
            .iter()
            .any(|c| c.contains("plan recall") && c.contains("ファルモ")));
        assert!(trace
            .thoughts
            .iter()
            .any(|t| t.contains("recall[1/2]") && t.contains("hits=1")));
    }

    struct AlwaysThought;

    impl AgentBrain for AlwaysThought {
        fn decide(&mut self, _ctx: &TurnPromptContext<'_>) -> AgentStep {
            AgentStep::Thought("still exploring in plan layer".into())
        }
    }

    #[test]
    fn plan_loop_without_answer_requests_mandatory_plan_then_freeform_fallback() {
        let mut brain = AlwaysThought;
        let mut tools = ToolRuntime::from_registry(
            crate::runtime::RuntimeEnvironment::detect(),
            None,
            crate::tool::full_builtin_registry(false),
        );
        let mut blocks = PromptBlocks::default();
        let session = SessionMemory::default();

        let (harness, trace, steps_used) = run_plan_layer(
            &mut brain,
            &mut tools,
            &mut blocks,
            &session,
            "どういう改造が計画されているの？",
            4,
            false,
            false,
            false,
            false,
            None,
            None,
            None,
            2,
            &crate::tasks::TaskRegistry::builtin(),
            false,
            40,
            8000,
        )
        .unwrap();

        // 4 ステップ + 強制 answer 要求の 1 回
        assert_eq!(steps_used, 5);
        assert!(!harness.plan.skip_execution);
        assert_eq!(harness.plan.knowledge_sufficient, Some(false));
        assert_eq!(harness.plan.subtasks.len(), 1);
        assert!(harness.plan.subtasks[0].task.is_none());
        assert_eq!(
            harness.plan.subtasks[0].goal,
            "どういう改造が計画されているの？"
        );
        assert!(trace
            .thoughts
            .iter()
            .any(|t| t.contains("Emit") && t.contains("answer")));
        assert!(trace
            .thoughts
            .iter()
            .any(|t| t.contains("mandatory answer not produced")));
    }

    struct FinalizeAnswerBrain {
        calls: usize,
    }

    impl AgentBrain for FinalizeAnswerBrain {
        fn decide(&mut self, _ctx: &TurnPromptContext<'_>) -> AgentStep {
            self.calls += 1;
            if self.calls <= 4 {
                return AgentStep::Thought("not yet".into());
            }
            AgentStep::Answer(
                r#"{"summary":"ok","skip_execution":true,"knowledge_sufficient":true,"subtasks":[],"output":"done"}"#
                    .into(),
            )
        }
    }

    #[test]
    fn plan_loop_mandatory_answer_can_skip_when_sufficient() {
        let mut brain = FinalizeAnswerBrain { calls: 0 };
        let mut tools = ToolRuntime::from_registry(
            crate::runtime::RuntimeEnvironment::detect(),
            None,
            crate::tool::full_builtin_registry(false),
        );
        let mut blocks = PromptBlocks::default();
        let session = SessionMemory::default();

        let (harness, _, steps_used) = run_plan_layer(
            &mut brain,
            &mut tools,
            &mut blocks,
            &session,
            "hello",
            4,
            false,
            false,
            false,
            false,
            None,
            None,
            None,
            2,
            &crate::tasks::TaskRegistry::builtin(),
            false,
            40,
            8000,
        )
        .unwrap();

        assert_eq!(steps_used, 5);
        assert!(harness.plan.skip_execution);
        assert_eq!(harness.plan.knowledge_sufficient, Some(true));
        assert!(harness.plan.subtasks.is_empty());
    }

    #[test]
    fn exec_loop_finalizes_instead_of_max_steps_error() {
        let mut brain = SeqBrain {
            steps: vec![
                AgentStep::Action(Action::new(
                    1,
                    "echo",
                    serde_json::json!({ "message": "one" }),
                )),
                AgentStep::Action(Action::new(
                    2,
                    "echo",
                    serde_json::json!({ "message": "two" }),
                )),
                AgentStep::Action(Action::new(
                    3,
                    "echo",
                    serde_json::json!({ "message": "three" }),
                )),
                // finalize decide still refuses to answer → trace fallback
                AgentStep::Action(Action::new(
                    4,
                    "echo",
                    serde_json::json!({ "message": "four" }),
                )),
            ],
            index: 0,
        };
        let mut tools = ToolRuntime::from_registry(
            crate::runtime::RuntimeEnvironment::detect(),
            None,
            crate::tool::full_builtin_registry(false),
        );
        let mut blocks = PromptBlocks::default();
        let session = SessionMemory::default();

        let turn = run_layer_loop(
            &mut brain,
            &mut tools,
            &mut blocks,
            &session,
            "summarize evidence",
            LayerLoopOptions::exec(3, 1),
            false,
            false,
            false,
            None,
            vec![],
            None,
            None,
            None,
            0,
        )
        .expect("exec should finalize, not MaxStepsExceeded");

        assert!(
            turn.answer.contains("step limit") || turn.answer.contains("Evidence"),
            "got: {}",
            turn.answer
        );
        assert!(turn.answer.contains("summarize evidence"));
        assert_eq!(turn.steps_used, 4);
        assert!(!turn.trace.observations.is_empty());
    }
}
