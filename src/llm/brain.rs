use crate::action::AgentStep;
use crate::brain::AgentBrain;
use crate::context::TurnPromptContext;
use crate::context_metrics::ContextUsage;

use super::connector::{ChatMessage, LlmConnector};
use super::parse::parse_agent_step;

/// `LlmConnector` を使って `decide` する頭脳。
pub struct LlmBrain<C: LlmConnector> {
    connector: C,
    next_invoke_id: u64,
    last_usage: Option<ContextUsage>,
}

impl<C: LlmConnector> LlmBrain<C> {
    pub fn new(connector: C) -> Self {
        Self {
            connector,
            next_invoke_id: 0,
            last_usage: None,
        }
    }

    pub fn connector_provider(&self) -> super::connector::LlmProvider {
        self.connector.provider()
    }

    pub fn connector(&self) -> &C {
        &self.connector
    }

    /// 計画フェーズなど `decide` 以外の completion 計測を保持する。
    pub(crate) fn set_last_usage(&mut self, usage: ContextUsage) {
        self.last_usage = Some(usage);
    }

    fn next_id(&mut self) -> u64 {
        self.next_invoke_id += 1;
        self.next_invoke_id
    }

    /// プロンプト文脈から LLM 用メッセージ列を組み立てる（テスト・計測用にも公開）。
    pub fn build_messages(ctx: &TurnPromptContext<'_>) -> Vec<ChatMessage> {
        ctx.render()
    }

    fn step_from_llm(&mut self, raw: &str) -> AgentStep {
        let id = self.next_id();
        match parse_agent_step(raw, id) {
            Ok(step) => step,
            Err(err) => AgentStep::Answer(format!("LLM response parse error: {err}\nraw: {raw}")),
        }
    }
}

impl<C: LlmConnector> AgentBrain for LlmBrain<C> {
    fn decide(&mut self, ctx: &TurnPromptContext<'_>) -> AgentStep {
        let messages = Self::build_messages(ctx);
        match self.connector.complete(&messages) {
            Ok(result) => {
                self.last_usage = Some(result.usage);
                self.step_from_llm(&result.content)
            }
            Err(err) => {
                self.last_usage = None;
                AgentStep::Answer(format!("LLM connector error: {err}"))
            }
        }
    }

    fn poll_context_usage(&mut self) -> Option<ContextUsage> {
        self.last_usage.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::{Observation, TurnTrace};
    use crate::context::PromptBlocks;
    use crate::llm::mock::MockLlmConnector;
    use crate::session::SessionMemory;

    fn ctx<'a>(
        blocks: &'a PromptBlocks,
        user_input: &'a str,
        trace: &'a TurnTrace,
        session: &'a SessionMemory,
    ) -> TurnPromptContext<'a> {
        TurnPromptContext::new(blocks, user_input, trace, session)
    }

    #[test]
    fn mock_returns_thought_first() {
        let mut brain = LlmBrain::new(MockLlmConnector);
        let blocks = PromptBlocks::default();
        let trace = TurnTrace::default();
        let session = SessionMemory::default();
        let step = brain.decide(&ctx(&blocks, "hello", &trace, &session));
        assert!(matches!(step, AgentStep::Thought(_)));
    }

    #[test]
    fn mock_returns_answer_after_observation() {
        let mut brain = LlmBrain::new(MockLlmConnector);
        let mut trace = TurnTrace::default();
        trace.push_observation(Observation::success(1, "x"));
        let blocks = PromptBlocks::default();
        let session = SessionMemory::default();
        let step = brain.decide(&ctx(&blocks, "hello", &trace, &session));
        assert!(matches!(step, AgentStep::Answer(a) if a.contains("mock answer")));
    }

    #[test]
    fn user_prompt_includes_previous_turns() {
        let mut session = SessionMemory::new(4);
        session.push_turn("first", "one");
        let blocks = PromptBlocks::default();
        let messages = LlmBrain::<MockLlmConnector>::build_messages(&ctx(
            &blocks,
            "second",
            &TurnTrace::default(),
            &session,
        ));
        let user = messages
            .iter()
            .find(|m| m.role == "user")
            .expect("user message");
        assert!(user.content.contains("Previous turns:"));
        assert!(user.content.contains("User: first"));
        assert!(user.content.contains("Assistant: one"));
        assert!(user.content.contains("User input:\nsecond"));
    }
}
