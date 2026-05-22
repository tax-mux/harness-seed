//! LLM コネクタと LLM 駆動の `AgentBrain`。

mod brain;
mod chat_completions;
mod completion;
mod connector;
mod anthropic;
mod factory;
mod gemini;
mod lmstudio;
mod mock;
mod openai;
mod parse;

pub use brain::LlmBrain;
pub use chat_completions::ChatCompletionsConnector;
pub use completion::CompletionResult;
pub use connector::{
    normalize_lmstudio_base_url, normalize_ollama_base_url,
    normalize_openai_compatible_base_url, ChatMessage, ConnectorError, LlmConfig, LlmConnector,
    LlmProvider,
};
pub use factory::LlmConnectorKind;
pub use anthropic::{normalize_anthropic_base_url, AnthropicConnector};
pub use gemini::{normalize_gemini_base_url, GeminiConnector};
pub use lmstudio::LmStudioConnector;
pub use mock::MockLlmConnector;
pub use openai::OpenAiConnector;
pub use parse::{parse_agent_step, ParseError};
