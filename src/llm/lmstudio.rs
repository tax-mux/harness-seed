use super::chat_completions::ChatCompletionsConnector;
use super::connector::{ConnectorError, LlmConfig, LlmConnector, LlmProvider};
use super::completion::CompletionResult;
use super::connector::ChatMessage;

/// [LM Studio](https://lmstudio.ai/) ローカルサーバー用コネクタ（OpenAI 互換 API）。
///
/// 既定 URL: `http://127.0.0.1:1234/v1`（LM Studio → Developer → Local Server）
#[derive(Debug)]
pub struct LmStudioConnector(ChatCompletionsConnector);

impl LmStudioConnector {
    pub fn new(config: LlmConfig) -> Result<Self, ConnectorError> {
        Ok(Self(ChatCompletionsConnector::new(config)?))
    }

    pub fn from_env() -> Result<Self, ConnectorError> {
        let app = crate::config::AppConfig::load_default().map_err(|e| {
            ConnectorError::Config(e.to_string())
        })?;
        Self::new(app.build_llm_config()?)
    }

    pub fn config(&self) -> &LlmConfig {
        self.0.config()
    }

    pub fn inner(&self) -> &ChatCompletionsConnector {
        &self.0
    }
}

impl LlmConnector for LmStudioConnector {
    fn provider(&self) -> LlmProvider {
        LlmProvider::LmStudio
    }

    fn complete(&self, messages: &[ChatMessage]) -> Result<CompletionResult, ConnectorError> {
        self.0.complete(messages)
    }
}
