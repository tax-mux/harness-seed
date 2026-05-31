use crate::action::AgentStep;
use crate::brain::AgentBrain;
use crate::context::TurnPromptContext;
use crate::context_metrics::ContextUsage;
use crate::llm::{ChatMessage, LlmBrain, LlmConnector, LlmConnectorKind, MockLlmConnector};
use crate::tasks::TaskRegistry;

use super::parse_step::parse_plan_agent_step;
use super::PlanArtifact;

/// 計画層用 ReAct system（`thought` / `answer` のみ。ツールは不可）。
pub const PLAN_REACT_SYSTEM_CORE: &str = r#"You are a planning agent in a ReAct-style loop. Reply with ONE JSON object only (no markdown).

Schema:
- {"step":"thought","content":"<reasoning>"}
- {"step":"answer","content":"<作業指示書 — structured plan JSON or numbered text steps>"}

The harness parses your answer into internal JSON (HarnessState). Prefer ONE of:
- Structured JSON (input / steps / output) as below, OR
- Numbered work-instruction lines (e.g. "1. ..." / "ステップ1: ...").

Planning schema (inside answer content when using JSON):
{
  "input": ["<fixed INPUT contract lines copied from prompt>"],
  "steps": [
    {"id": 1, "task": "<registered task id>", "params": {}, "goal": "", "done_when": ""}
  ],
  "output": "<fixed OUTPUT contract line copied from prompt>",
  "skip_execution": <bool>,
}

Rules:
- INPUT and OUTPUT boundaries are fixed before you run. Do NOT change read/write sources or storage targets.
- Instruction contract: "Take data ONLY from INPUT. Write result ONLY to OUTPUT. Think ONLY about the in-between procedure."
- Your job is the PROCEDURE in between: emit ordered `steps` that transform INPUT into OUTPUT.
- Prefer emitting `answer` with the work instructions (JSON plan or numbered steps) when clear; use `thought` only for brief decomposition if needed.
- Do NOT emit action / tools in the plan layer.
- Use only task ids from the task catalog that match the data contract.
- Use skip_execution: true only when the contract says so (trivial chat).
- When ready, use step answer with the full work instructions (JSON or text) in content.
"#;

/// 計画層の頭脳（ルール / LLM / テスト用 Mock）。
pub enum PlanBrainMode {
    Rule(RulePlanBrain),
    Llm(PlanLlmBrain<LlmConnectorKind>),
    /// 統合テスト用（`MockLlmConnector`）。
    Mock(PlanLlmBrain<MockLlmConnector>),
}

impl PlanBrainMode {
    pub fn rule() -> Self {
        Self::Rule(RulePlanBrain::new())
    }

    pub fn from_cli(
        app: &crate::config::AppConfig,
        use_llm: bool,
        no_llm: bool,
        registry: &TaskRegistry,
    ) -> Result<Self, crate::llm::ConnectorError> {
        let want_llm = !no_llm && (use_llm || app.uses_llm());
        if want_llm {
            let config = crate::llm::LlmConfig::from_app(app)?;
            let connector = LlmConnectorKind::from_config(config)?;
            Ok(Self::Llm(PlanLlmBrain::new(connector, registry)))
        } else {
            Ok(Self::rule())
        }
    }
}

impl AgentBrain for PlanBrainMode {
    fn decide(&mut self, ctx: &TurnPromptContext<'_>) -> AgentStep {
        match self {
            Self::Rule(b) => b.decide(ctx),
            Self::Llm(b) => b.decide(ctx),
            Self::Mock(b) => b.decide(ctx),
        }
    }

    fn poll_context_usage(&mut self) -> Option<ContextUsage> {
        match self {
            Self::Rule(b) => b.poll_context_usage(),
            Self::Llm(b) => b.poll_context_usage(),
            Self::Mock(b) => b.poll_context_usage(),
        }
    }
}

/// ルールベース計画層（Thought → Answer with plan JSON）。
#[derive(Debug, Default)]
pub struct RulePlanBrain;

impl RulePlanBrain {
    pub fn new() -> Self {
        Self
    }

    fn plan_json_for_input(input: &str) -> String {
        let input = input.trim();
        if input.eq_ignore_ascii_case("help")
            || input.eq_ignore_ascii_case("time")
            || input.starts_with("echo ")
        {
            return r#"{"summary":"direct","skip_execution":true,"subtasks":[]}"#.into();
        }
        serde_json::json!({
            "summary": "single task",
            "skip_execution": false,
            "subtasks": [{
                "id": 1,
                "goal": input,
                "done_when": "user request satisfied"
            }]
        })
        .to_string()
    }
}

impl AgentBrain for RulePlanBrain {
    fn decide(&mut self, ctx: &TurnPromptContext<'_>) -> AgentStep {
        if ctx.trace.thoughts.is_empty() && !ctx.trace.actions.is_empty() {
            return AgentStep::Answer(Self::plan_json_for_input(ctx.user_input));
        }
        if ctx.trace.thoughts.is_empty() {
            let input = ctx.user_input.trim();
            if input.eq_ignore_ascii_case("help")
                || input.eq_ignore_ascii_case("time")
                || input.starts_with("echo ")
            {
                return AgentStep::Answer(Self::plan_json_for_input(input));
            }
            return AgentStep::Thought("依頼をサブタスクに分解する".into());
        }
        AgentStep::Answer(Self::plan_json_for_input(ctx.user_input))
    }
}

/// LLM 計画層（ReAct ステップ → 最終 Answer で PlanArtifact）。
pub struct PlanLlmBrain<C: LlmConnector> {
    inner: LlmBrain<C>,
    registry: TaskRegistry,
}

impl<C: LlmConnector> PlanLlmBrain<C> {
    pub fn new(connector: C, registry: &TaskRegistry) -> Self {
        Self {
            inner: LlmBrain::new(connector),
            registry: registry.clone(),
        }
    }

    pub fn build_messages(ctx: &TurnPromptContext<'_>, task_catalog: &str) -> Vec<ChatMessage> {
        super::prompt::build_plan_layer_messages_with_catalog(ctx, task_catalog)
    }
}

impl<C: LlmConnector> AgentBrain for PlanLlmBrain<C> {
    fn decide(&mut self, ctx: &TurnPromptContext<'_>) -> AgentStep {
        let catalog = ctx.blocks.plan_task_catalog.clone().unwrap_or_else(|| {
            self.registry
                .catalog_for_planner_opts(ctx.blocks.web_search_enabled)
        });
        let messages = Self::build_messages(ctx, &catalog);
        match self.inner.connector().complete(&messages) {
            Ok(result) => {
                self.inner.set_last_usage(result.usage);
                match parse_plan_agent_step(&result.content) {
                    Ok(step) => step,
                    Err(err) => AgentStep::Answer(format!(
                        "plan step parse error: {err}\nraw: {}",
                        result.content
                    )),
                }
            }
            Err(err) => AgentStep::Answer(format!("LLM connector error: {err}")),
        }
    }

    fn poll_context_usage(&mut self) -> Option<ContextUsage> {
        self.inner.poll_context_usage()
    }
}

/// 計画層ループの `TurnResult` から [`PlanArtifact`] を取り出す。
pub fn artifact_from_plan_turn(answer: &str, user_input: &str) -> PlanArtifact {
    super::parse_step::plan_artifact_from_answer(answer, user_input)
}
