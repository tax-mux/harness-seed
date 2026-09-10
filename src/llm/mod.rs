//! LLM コネクタと LLM 駆動の `AgentBrain`。

mod anthropic;
mod brain;
mod chat_completions;
mod completion;
mod connector;
mod factory;
mod gemini;
mod lmstudio;
mod mock;
mod openai;
mod parse;
mod sse;

pub use anthropic::{normalize_anthropic_base_url, AnthropicConnector};
pub use brain::LlmBrain;
pub use chat_completions::ChatCompletionsConnector;
pub use completion::CompletionResult;
pub use connector::{
    format_error_chain, normalize_lmstudio_base_url, normalize_ollama_base_url,
    normalize_openai_compatible_base_url, require_absolute_http_base, ChatMessage, ConnectorError,
    LlmConfig, LlmConnector, LlmProvider, StreamStats,
};
pub use factory::LlmConnectorKind;
pub use gemini::{normalize_gemini_base_url, resolve_gemini_base_url, GeminiConnector};
pub use lmstudio::LmStudioConnector;
pub use mock::MockLlmConnector;
pub use openai::OpenAiConnector;
pub use parse::{
    coerce_tool_named_step_json, extract_json_objects, parse_agent_step,
    salvage_answer_step_content, ParseError,
};
pub use sse::{SseEvent, SseEventKind, SseParser, DONE_SENTINEL};
