use super::chat_completions::ChatCompletionsConnector;
use super::connector::{ConnectorError, LlmConfig, LlmConnector};

/// OpenAI 互換 API コネクタ。
#[derive(Debug)]
pub struct OpenAiConnector(ChatCompletionsConnector);

impl OpenAiConnector {
    pub fn new(config: LlmConfig) -> Result<Self, ConnectorError> {
        Ok(Self(ChatCompletionsConnector::new(config)?))
    }

    pub fn from_env() -> Result<Self, ConnectorError> {
        Self::new(LlmConfig::from_env()?)
    }

    pub fn config(&self) -> &LlmConfig {
        self.0.config()
    }

    pub fn inner(&self) -> &ChatCompletionsConnector {
        &self.0
    }
}

impl LlmConnector for OpenAiConnector {
    fn provider(&self) -> super::connector::LlmProvider {
        self.0.provider()
    }

    fn complete(
        &self,
        messages: &[super::connector::ChatMessage],
    ) -> Result<super::completion::CompletionResult, ConnectorError> {
        self.0.complete(messages)
    }
}
