use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use serde::Deserialize;
use serde_json::json;

use super::completion::CompletionResult;
use super::connector::{ChatMessage, ConnectorError, LlmConfig, LlmConnector, LlmProvider};
use crate::context_metrics::{format_messages_body, ContextUsage};

const DEFAULT_ANTHROPIC_BASE: &str = "https://api.anthropic.com";
/// Messages API の `anthropic-version`（[Anthropic API](https://docs.anthropic.com)）。
const ANTHROPIC_VERSION: &str = "2023-06-01";
const DEFAULT_MAX_TOKENS: u32 = 8192;

/// Anthropic API のルート URL（`/v1/messages` はコネクタ側で付与）。
pub fn normalize_anthropic_base_url(host: &str) -> String {
    let trimmed = host.trim().trim_end_matches('/');
    let base = if trimmed.is_empty() {
        DEFAULT_ANTHROPIC_BASE
    } else {
        trimmed
    };
    base.strip_suffix("/v1")
        .unwrap_or(base)
        .to_string()
}

/// Anthropic Messages API コネクタ（Claude 直）。
#[derive(Debug)]
pub struct AnthropicConnector {
    client: Client,
    config: LlmConfig,
}

impl AnthropicConnector {
    pub fn new(config: LlmConfig) -> Result<Self, ConnectorError> {
        if config
            .api_key
            .as_ref()
            .filter(|k| !k.is_empty())
            .is_none()
        {
            return Err(ConnectorError::MissingApiKey);
        }

        let client = Client::builder()
            .timeout(config.timeout)
            .build()?;
        Ok(Self { client, config })
    }

    pub fn config(&self) -> &LlmConfig {
        &self.config
    }

    fn messages_url(&self) -> String {
        format!("{}/v1/messages", self.config.base_url)
    }

    fn partition_messages(messages: &[ChatMessage]) -> (Option<String>, Vec<serde_json::Value>) {
        let mut system_lines = Vec::new();
        let mut api_messages = Vec::new();

        for m in messages {
            match m.role.as_str() {
                "system" => system_lines.push(m.content.clone()),
                "assistant" => api_messages.push(json!({
                    "role": "assistant",
                    "content": m.content
                })),
                _ => api_messages.push(json!({
                    "role": "user",
                    "content": m.content
                })),
            }
        }

        let system = if system_lines.is_empty() {
            None
        } else {
            Some(system_lines.join("\n\n"))
        };

        (system, api_messages)
    }

    fn auth_headers(api_key: &str) -> Result<HeaderMap, ConnectorError> {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-api-key",
            HeaderValue::from_str(api_key).map_err(|e| ConnectorError::Config(e.to_string()))?,
        );
        headers.insert(
            "anthropic-version",
            HeaderValue::from_static(ANTHROPIC_VERSION),
        );
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        Ok(headers)
    }
}

#[derive(Deserialize, Default)]
struct Usage {
    #[serde(default)]
    input_tokens: Option<u32>,
    #[serde(default)]
    output_tokens: Option<u32>,
}

#[derive(Deserialize)]
struct MessagesResponse {
    content: Vec<ContentBlock>,
    #[serde(default)]
    usage: Usage,
}

#[derive(Deserialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    block_type: String,
    text: Option<String>,
}

impl LlmConnector for AnthropicConnector {
    fn provider(&self) -> LlmProvider {
        LlmProvider::Anthropic
    }

    fn complete(&self, messages: &[ChatMessage]) -> Result<CompletionResult, ConnectorError> {
        if messages.is_empty() {
            return Err(ConnectorError::InvalidResponse(
                "messages must not be empty".into(),
            ));
        }

        let api_key = self
            .config
            .api_key
            .as_deref()
            .filter(|k| !k.is_empty())
            .ok_or(ConnectorError::MissingApiKey)?;

        let (mut system, api_messages) = Self::partition_messages(messages);
        if api_messages.is_empty() {
            return Err(ConnectorError::InvalidResponse(
                "no user/assistant messages for Anthropic".into(),
            ));
        }

        if self.config.json_mode {
            let hint = "You must reply with a single valid JSON object only (no markdown fences).";
            system = Some(match system {
                Some(s) => format!("{s}\n\n{hint}"),
                None => hint.to_string(),
            });
        }

        let mut body = json!({
            "model": self.config.model,
            "max_tokens": DEFAULT_MAX_TOKENS,
            "temperature": 0.2,
            "messages": api_messages
        });

        if let Some(system) = system {
            body["system"] = json!(system);
        }

        let response = self
            .client
            .post(self.messages_url())
            .headers(Self::auth_headers(api_key)?)
            .json(&body)
            .send()?;

        let status = response.status();
        let text = response.text()?;
        if !status.is_success() {
            return Err(ConnectorError::Http {
                status: status.as_u16(),
                body: text,
            });
        }

        let parsed: MessagesResponse = serde_json::from_str(&text)
            .map_err(|e| ConnectorError::InvalidResponse(format!("{e}; body={text}")))?;

        let content = parsed
            .content
            .into_iter()
            .find(|b| b.block_type == "text")
            .and_then(|b| b.text)
            .ok_or_else(|| ConnectorError::InvalidResponse("empty content blocks".into()))?;

        let usage = ContextUsage::from_parts(
            &format_messages_body(messages),
            &content,
            parsed.usage.input_tokens,
            parsed.usage.output_tokens,
        );

        Ok(CompletionResult { content, usage })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_strips_v1_suffix() {
        assert_eq!(
            normalize_anthropic_base_url("https://api.anthropic.com/v1"),
            "https://api.anthropic.com"
        );
    }

    #[test]
    fn partition_extracts_system() {
        let messages = vec![
            ChatMessage::system("rules"),
            ChatMessage::user("hello"),
        ];
        let (sys, msgs) = AnthropicConnector::partition_messages(&messages);
        assert_eq!(sys.as_deref(), Some("rules"));
        assert_eq!(msgs.len(), 1);
    }
}
