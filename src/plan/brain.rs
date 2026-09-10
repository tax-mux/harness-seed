use crate::action::AgentStep;
use crate::brain::AgentBrain;
use crate::context::{format_trace, TurnPromptContext};
use crate::context_metrics::ContextUsage;
use crate::llm::{ChatMessage, LlmBrain, LlmConnector, LlmConnectorKind, MockLlmConnector};
use crate::tasks::TaskRegistry;

use super::parse_step::{parse_plan_agent_step, plan_artifact_from_answer};
use super::PlanArtifact;

/// 計画層用 ReAct system（`thought` / `answer` のみ。ツールは不可）。
pub const PLAN_REACT_SYSTEM_CORE: &str = r#"You are a planning agent in a ReAct-style loop. Reply with ONE JSON object only (no markdown).

Schema:
- {"step":"thought","content":"<reasoning>"}
- {"step":"answer","content":"<PlanArtifact JSON string — same schema as below>"}

PlanArtifact schema (inside answer content):
{
  "summary": "<one-line summary>",
  "skip_execution": <bool>,
  "subtasks": [
    {"id": 1, "task": "<registered task id>", "params": {}, "goal": "", "done_when": ""}
  ]
}

Rules:
- Prefer emitting `answer` with the PlanArtifact JSON when the plan is clear; use `thought` only for brief decomposition if needed.
- Do NOT emit action / tools in the plan layer.
- Prefer registered task ids from the task catalog (with params).
- Use skip_execution: true only for trivial chat/help with no tool work.
- When ready to output the final plan, use step answer with the full PlanArtifact JSON in content.
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
        let mut system = String::from(PLAN_REACT_SYSTEM_CORE);
        if ctx.blocks.web_search_enabled {
            system.push_str(
                "\n- Web search is enabled: assign task `web_research` with params {\"query\":\"...\"} for external/current-events questions.\n",
            );
        }
        if !ctx.blocks.rules.is_empty() {
            system.push_str("\n\nAdditional rules:\n");
            for (i, rule) in ctx.blocks.rules.iter().enumerate() {
                system.push_str(&format!("\n[rule {}]\n{rule}\n", i + 1));
            }
        }
        if !ctx.blocks.recalled.is_empty() {
            system.push_str("\n\nRecalled context:\n");
            for (i, chunk) in ctx.blocks.recalled.iter().enumerate() {
                system.push_str(&format!("\n[recalled {}]\n{chunk}\n", i + 1));
            }
        }
        if !task_catalog.is_empty() {
            system.push_str("\n\n");
            system.push_str(task_catalog);
        }
        system.push_str("\n\nExecution environment:\n");
        system.push_str(&ctx.blocks.runtime.prompt_hint());

        let previous = ctx.session.format_for_prompt();
        let previous_block = if previous.is_empty() {
            String::new()
        } else {
            format!("{previous}\n")
        };
        let trace_text = format_trace(ctx.trace);
        let user = format!(
            "{previous_block}Plan request:\n{}\n\nPlan trace so far:\n{trace_text}\n\nNext plan step JSON:",
            ctx.user_input
        );

        vec![ChatMessage::system(system), ChatMessage::user(user)]
    }
}

impl<C: LlmConnector> AgentBrain for PlanLlmBrain<C> {
    fn decide(&mut self, ctx: &TurnPromptContext<'_>) -> AgentStep {
        let catalog = self
            .registry
            .catalog_for_planner_opts(ctx.blocks.web_search_enabled);
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
    plan_artifact_from_answer(answer, user_input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::TurnTrace;
    use crate::context::PromptBlocks;
    use crate::llm::MockLlmConnector;
    use crate::session::SessionMemory;

    #[test]
    fn rule_plan_help_skips_in_one_step() {
        let mut brain = RulePlanBrain::new();
        let blocks = PromptBlocks::default();
        let trace = TurnTrace::default();
        let session = SessionMemory::default();
        let ctx = TurnPromptContext::new(&blocks, "help", &trace, &session);
        let step = brain.decide(&ctx);
        let answer = match step {
            AgentStep::Answer(a) => a,
            _ => panic!("expected answer"),
        };
        let plan = plan_artifact_from_answer(&answer, "help");
        assert!(plan.skip_execution);
    }

    #[test]
    fn rule_plan_generic_two_steps() {
        let mut brain = RulePlanBrain::new();
        let blocks = PromptBlocks::default();
        let session = SessionMemory::default();
        let trace0 = TurnTrace::default();
        let ctx1 = TurnPromptContext::new(&blocks, "hello", &trace0, &session);
        assert!(matches!(brain.decide(&ctx1), AgentStep::Thought(_)));

        let mut trace = TurnTrace::default();
        trace.push_thought("plan".into());
        let ctx2 = TurnPromptContext::new(&blocks, "hello", &trace, &session);
        assert!(matches!(brain.decide(&ctx2), AgentStep::Answer(_)));
    }

    #[test]
    fn plan_llm_mock_thought_then_answer() {
        let reg = TaskRegistry::builtin();
        let mut brain = PlanLlmBrain::new(MockLlmConnector, &reg);
        let blocks = PromptBlocks::default();
        let session = SessionMemory::default();
        let s1 = brain.decide(&TurnPromptContext::new(&blocks, "do x", &TurnTrace::default(), &session));
        assert!(matches!(s1, AgentStep::Thought(_)));

        let mut trace = TurnTrace::default();
        trace.push_thought("t".into());
        let s2 = brain.decide(&TurnPromptContext::new(&blocks, "do x", &trace, &session));
        assert!(matches!(s2, AgentStep::Answer(_)));
    }
}
