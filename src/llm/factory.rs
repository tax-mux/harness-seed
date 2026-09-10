use super::chat_completions::ChatCompletionsConnector;
use super::connector::{ConnectorError, LlmConfig, LlmConnector, LlmProvider};
use super::anthropic::AnthropicConnector;
use super::gemini::GeminiConnector;
use super::lmstudio::LmStudioConnector;
use super::openai::OpenAiConnector;

/// 設定に応じた LLM コネクタ。
#[derive(Debug)]
pub enum LlmConnectorKind {
    OpenAi(OpenAiConnector),
    LmStudio(LmStudioConnector),
    /// Ollama も OpenAI 互換エンドポイント経由。
    ChatCompletions(ChatCompletionsConnector),
    Gemini(GeminiConnector),
    Anthropic(AnthropicConnector),
}

impl LlmConnectorKind {
    pub fn from_config(config: LlmConfig) -> Result<Self, ConnectorError> {
        match config.provider {
            LlmProvider::LmStudio => {
                Ok(Self::LmStudio(LmStudioConnector::new(config)?))
            }
            LlmProvider::OpenAi => Ok(Self::OpenAi(OpenAiConnector::new(config)?)),
            LlmProvider::Ollama => Ok(Self::ChatCompletions(ChatCompletionsConnector::new(
                config,
            )?)),
            LlmProvider::Gemini => Ok(Self::Gemini(GeminiConnector::new(config)?)),
            LlmProvider::Anthropic => Ok(Self::Anthropic(AnthropicConnector::new(config)?)),
        }
    }
}

impl LlmConnector for LlmConnectorKind {
    fn provider(&self) -> LlmProvider {
        match self {
            Self::OpenAi(c) => c.provider(),
            Self::LmStudio(c) => c.provider(),
            Self::ChatCompletions(c) => c.provider(),
            Self::Gemini(c) => c.provider(),
            Self::Anthropic(c) => c.provider(),
        }
    }

    fn complete(
        &self,
        messages: &[super::connector::ChatMessage],
    ) -> Result<super::completion::CompletionResult, ConnectorError> {
        match self {
            Self::OpenAi(c) => c.complete(messages),
            Self::LmStudio(c) => c.complete(messages),
            Self::ChatCompletions(c) => c.complete(messages),
            Self::Gemini(c) => c.complete(messages),
            Self::Anthropic(c) => c.complete(messages),
        }
    }
}
