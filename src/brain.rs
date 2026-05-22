use crate::action::AgentStep;
use crate::context::TurnPromptContext;
use crate::tool::{echo_action, time_action, HELP_TEXT};

use crate::context_metrics::ContextUsage;

/// ユーザー入力とターン途中経過から次ステップを決める。
pub trait AgentBrain {
    fn decide(&mut self, ctx: &TurnPromptContext<'_>) -> AgentStep;

    /// 直近の LLM 呼び出しのコンテキスト計測（LLM 頭脳のみ）。
    fn poll_context_usage(&mut self) -> Option<ContextUsage> {
        None
    }
}

/// ルールベースの頭脳（LLM なし・ReAct 構造のデモ用）。
#[derive(Debug, Default)]
pub struct SimpleRuleBrain {
    next_invoke_id: u64,
}

impl SimpleRuleBrain {
    pub fn new() -> Self {
        Self::default()
    }

    fn next_id(&mut self) -> u64 {
        self.next_invoke_id += 1;
        self.next_invoke_id
    }
}

impl AgentBrain for SimpleRuleBrain {
    fn decide(&mut self, ctx: &TurnPromptContext<'_>) -> AgentStep {
        let input = ctx.user_input.trim();
        let trace = ctx.trace;

        if input.eq_ignore_ascii_case("help") {
            return AgentStep::Answer(HELP_TEXT.to_string());
        }

        if trace.observations.is_empty() {
            if let Some(message) = input.strip_prefix("echo ") {
                let id = self.next_id();
                return AgentStep::Action(echo_action(id, message));
            }
            if input.eq_ignore_ascii_case("time") {
                let id = self.next_id();
                return AgentStep::Action(time_action(id));
            }
            if trace.thoughts.is_empty() {
                return AgentStep::Thought(
                    "入力を確認し、echo ツールで記録してから応答する".to_string(),
                );
            }
            let id = self.next_id();
            return AgentStep::Action(echo_action(id, input));
        }

        let last = trace.observations.last().expect("observations non-empty");
        if last.ok {
            AgentStep::Answer(format!("受け取りました: {}", last.output))
        } else {
            AgentStep::Answer(format!("ツールエラー: {}", last.output))
        }
    }
}

/// REPL で使う頭脳の選択肢。
pub enum BrainMode {
    Rule(SimpleRuleBrain),
    Llm(crate::llm::LlmBrain<crate::llm::LlmConnectorKind>),
}

impl AgentBrain for BrainMode {
    fn decide(&mut self, ctx: &TurnPromptContext<'_>) -> AgentStep {
        match self {
            Self::Rule(b) => b.decide(ctx),
            Self::Llm(b) => b.decide(ctx),
        }
    }

    fn poll_context_usage(&mut self) -> Option<ContextUsage> {
        match self {
            Self::Rule(b) => b.poll_context_usage(),
            Self::Llm(b) => b.poll_context_usage(),
        }
    }
}

impl BrainMode {
    pub fn from_cli(
        app: &crate::config::AppConfig,
        use_llm: bool,
        no_llm: bool,
    ) -> Result<Self, crate::llm::ConnectorError> {
        let want_llm = !no_llm && (use_llm || app.uses_llm());
        if want_llm {
            let config = crate::llm::LlmConfig::from_app(app)?;
            let connector = crate::llm::LlmConnectorKind::from_config(config)?;
            Ok(Self::Llm(crate::llm::LlmBrain::new(connector)))
        } else {
            Ok(Self::Rule(SimpleRuleBrain::new()))
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::Rule(_) => "rule".into(),
            Self::Llm(b) => format!("llm:{}", b.connector_provider().as_str()),
        }
    }
}

/// 実行層・計画層の頭脳ペア。
pub struct BrainPair {
    pub exec: BrainMode,
    pub plan: crate::plan::PlanBrainMode,
}

impl BrainPair {
    pub fn from_cli(
        app: &crate::config::AppConfig,
        use_llm: bool,
        no_llm: bool,
    ) -> Result<Self, crate::llm::ConnectorError> {
        let registry = crate::tasks::TaskRegistry::load_default();
        Ok(Self {
            exec: BrainMode::from_cli(app, use_llm, no_llm)?,
            plan: crate::plan::PlanBrainMode::from_cli(app, use_llm, no_llm, &registry)?,
        })
    }

    pub fn label(&self) -> String {
        format!("exec:{} + plan", self.exec.label())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::{Observation, TurnTrace};
    use crate::session::SessionMemory;

    #[test]
    fn rule_brain_has_no_context_usage() {
        let mut mode = BrainMode::Rule(SimpleRuleBrain::new());
        let blocks = crate::context::PromptBlocks::default();
        let trace = TurnTrace::default();
        let session = SessionMemory::default();
        let ctx = TurnPromptContext::new(&blocks, "help", &trace, &session);
        let _ = mode.decide(&ctx);
        assert!(mode.poll_context_usage().is_none());
    }

    #[test]
    fn help_returns_answer_immediately() {
        let mut brain = SimpleRuleBrain::new();
        let blocks = crate::context::PromptBlocks::default();
        let trace = TurnTrace::default();
        let session = SessionMemory::default();
        let ctx = TurnPromptContext::new(&blocks, "help", &trace, &session);
        let step = brain.decide(&ctx);
        assert!(matches!(step, AgentStep::Answer(_)));
    }

    #[test]
    fn default_path_uses_thought_then_echo() {
        let mut brain = SimpleRuleBrain::new();
        let trace = TurnTrace::default();
        let blocks = crate::context::PromptBlocks::default();
        let session = SessionMemory::default();
        let ctx1 = TurnPromptContext::new(&blocks, "hello", &trace, &session);
        let step1 = brain.decide(&ctx1);
        assert!(matches!(step1, AgentStep::Thought(_)));

        let mut trace = TurnTrace::default();
        trace.push_thought("x".into());
        let ctx2 = TurnPromptContext::new(&blocks, "hello", &trace, &session);
        let step2 = brain.decide(&ctx2);
        assert!(matches!(step2, AgentStep::Action(a) if a.tool == "echo"));
    }

    #[test]
    fn after_observation_returns_answer() {
        let mut brain = SimpleRuleBrain::new();
        let mut trace = TurnTrace::default();
        trace.push_observation(Observation::success(1, "hello"));
        let blocks = crate::context::PromptBlocks::default();
        let session = SessionMemory::default();
        let ctx = TurnPromptContext::new(&blocks, "hello", &trace, &session);
        let step = brain.decide(&ctx);
        assert!(matches!(step, AgentStep::Answer(a) if a.contains("hello")));
    }
}
