use super::anthropic::AnthropicConnector;
use super::chat_completions::ChatCompletionsConnector;
use super::connector::{ConnectorError, LlmConfig, LlmConnector, LlmProvider};
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
            LlmProvider::LmStudio => Ok(Self::LmStudio(LmStudioConnector::new(config)?)),
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

    fn can_stream(&self) -> bool {
        match self {
            Self::OpenAi(c) => c.can_stream(),
            Self::LmStudio(c) => c.can_stream(),
            Self::ChatCompletions(c) => c.can_stream(),
            Self::Gemini(c) => c.can_stream(),
            Self::Anthropic(c) => c.can_stream(),
        }
    }

    fn complete_stream(
        &self,
        messages: &[super::connector::ChatMessage],
        on_token: &mut dyn FnMut(&str),
    ) -> Result<Option<super::connector::StreamStats>, ConnectorError> {
        match self {
            Self::OpenAi(c) => c.complete_stream(messages, on_token),
            Self::LmStudio(c) => c.complete_stream(messages, on_token),
            Self::ChatCompletions(c) => c.complete_stream(messages, on_token),
            Self::Gemini(c) => c.complete_stream(messages, on_token),
            Self::Anthropic(c) => c.complete_stream(messages, on_token),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn cfg(provider: LlmProvider, base_url: &str) -> LlmConfig {
        LlmConfig {
            provider,
            api_key: Some("test-key".into()),
            base_url: base_url.to_string(),
            model: "test-model".into(),
            timeout: Duration::from_secs(5),
            max_tokens: 128,
            json_mode: false,
        }
    }

    #[test]
    fn can_stream_dispatch_per_provider() {
        // Chat Completions 系（OpenAI / Ollama / LM Studio）は true
        let openai = LlmConnectorKind::from_config(
            cfg(LlmProvider::OpenAi, "http://127.0.0.1:8080/v1"),
        )
        .unwrap();
        let ollama = LlmConnectorKind::from_config(
            cfg(LlmProvider::Ollama, "http://localhost:11434/v1"),
        )
        .unwrap();
        assert!(openai.can_stream());
        assert!(ollama.can_stream());

        // Gemini は #661 で実装済で true、Anthropic は既定 false（#662 で実装予定）
        let gemini =
            LlmConnectorKind::from_config(cfg(LlmProvider::Gemini, "https://generativelanguage.googleapis.com"))
                .unwrap();
        let anthropic = LlmConnectorKind::from_config(cfg(
            LlmProvider::Anthropic,
            "https://api.anthropic.com/v1",
        ))
        .unwrap();
        assert!(gemini.can_stream(), "Gemini は #661 でストリーミング実装済み");
        assert!(!anthropic.can_stream(), "Anthropic は #662 で実装予定");
    }
}
