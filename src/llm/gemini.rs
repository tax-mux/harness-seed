use reqwest::blocking::Client;
use serde::Deserialize;
use serde_json::json;

use super::completion::CompletionResult;
use super::connector::{ChatMessage, ConnectorError, LlmConfig, LlmConnector, LlmProvider};
use crate::context_metrics::{format_messages_body, ContextUsage};

const DEFAULT_GEMINI_BASE: &str = "https://generativelanguage.googleapis.com/v1beta";

/// Google Gemini `generateContent` API 用ベース URL。
pub fn normalize_gemini_base_url(host: &str) -> String {
    let trimmed = host.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        DEFAULT_GEMINI_BASE.to_string()
    } else {
        trimmed.to_string()
    }
}

/// Gemini 向けに解決する base URL（LM Studio / Ollama 用 URL の誤設定を無視）。
pub fn resolve_gemini_base_url(configured: Option<&str>, env_gemini_base: Option<String>) -> String {
    if let Some(u) = env_gemini_base.filter(|s| !s.trim().is_empty()) {
        return normalize_gemini_base_url(&u);
    }
    if let Some(u) = configured.filter(|s| !s.trim().is_empty()) {
        if is_plausible_gemini_base_url(u) {
            return normalize_gemini_base_url(u);
        }
    }
    normalize_gemini_base_url("")
}

fn is_plausible_gemini_base_url(url: &str) -> bool {
    let lower = url.to_lowercase();
    if lower.contains("generativelanguage.googleapis.com") || lower.contains("googleapis.com") {
        return true;
    }
    // LM Studio / Ollama / ローカル OpenAI 互換は Gemini では使わない
    if lower.contains("127.0.0.1")
        || lower.contains("localhost")
        || lower.contains(":1234")
        || lower.contains(":11434")
    {
        return false;
    }
    !lower.contains("/v1/chat")
}

/// Gemini API コネクタ（`v1beta/models/{model}:generateContent`）。
#[derive(Debug)]
pub struct GeminiConnector {
    client: Client,
    config: LlmConfig,
}

impl GeminiConnector {
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

    fn partition_messages(messages: &[ChatMessage]) -> (Option<String>, Vec<serde_json::Value>) {
        let mut system_lines = Vec::new();
        let mut contents = Vec::new();

        for m in messages {
            match m.role.as_str() {
                "system" => system_lines.push(m.content.clone()),
                "assistant" => contents.push(json!({
                    "role": "model",
                    "parts": [{ "text": m.content }]
                })),
                _ => contents.push(json!({
                    "role": "user",
                    "parts": [{ "text": m.content }]
                })),
            }
        }

        let system = if system_lines.is_empty() {
            None
        } else {
            Some(system_lines.join("\n\n"))
        };

        (system, contents)
    }
}

#[derive(Deserialize, Default)]
struct UsageMetadata {
    #[serde(default)]
    prompt_token_count: Option<u32>,
    #[serde(default)]
    candidates_token_count: Option<u32>,
}

#[derive(Deserialize)]
struct GenerateContentResponse {
    candidates: Vec<Candidate>,
    #[serde(default)]
    usage_metadata: UsageMetadata,
}

#[derive(Deserialize)]
struct Candidate {
    content: CandidateContent,
}

#[derive(Deserialize)]
struct CandidateContent {
    parts: Vec<Part>,
}

#[derive(Deserialize)]
struct Part {
    text: Option<String>,
}

impl LlmConnector for GeminiConnector {
    fn provider(&self) -> LlmProvider {
        LlmProvider::Gemini
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

        let (system_instruction, contents) = Self::partition_messages(messages);
        if contents.is_empty() {
            return Err(ConnectorError::InvalidResponse(
                "no user/assistant messages for Gemini".into(),
            ));
        }

        let url = format!(
            "{}/models/{}:generateContent?key={}",
            self.config.base_url, self.config.model, api_key
        );

        let mut body = json!({
            "contents": contents,
            "generationConfig": {
                "temperature": 0.2
            }
        });

        if let Some(system) = system_instruction {
            body["systemInstruction"] = json!({
                "parts": [{ "text": system }]
            });
        }

        if self.config.json_mode {
            body["generationConfig"]["responseMimeType"] = json!("application/json");
        }

        let response = self.client.post(&url).json(&body).send()?;
        let status = response.status();
        let text = response.text()?;
        if !status.is_success() {
            return Err(ConnectorError::Http {
                status: status.as_u16(),
                body: text,
            });
        }

        if let Ok(err) = serde_json::from_str::<serde_json::Value>(&text) {
            if let Some(msg) = err.get("error").and_then(|e| e.as_str()).or_else(|| {
                err.pointer("/error/message")
                    .and_then(|m| m.as_str())
            }) {
                return Err(ConnectorError::InvalidResponse(msg.to_string()));
            }
        }

        let parsed: GenerateContentResponse = serde_json::from_str(&text)
            .map_err(|e| ConnectorError::InvalidResponse(format!("{e}; body={text}")))?;

        let content = parsed
            .candidates
            .into_iter()
            .next()
            .and_then(|c| c.content.parts.into_iter().next())
            .and_then(|p| p.text)
            .ok_or_else(|| ConnectorError::InvalidResponse("empty candidates".into()))?;

        let usage = ContextUsage::from_parts(
            &format_messages_body(messages),
            &content,
            parsed.usage_metadata.prompt_token_count,
            parsed.usage_metadata.candidates_token_count,
        );

        Ok(CompletionResult { content, usage })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_default_base() {
        assert_eq!(
            normalize_gemini_base_url(""),
            DEFAULT_GEMINI_BASE
        );
    }

    #[test]
    fn resolve_ignores_lmstudio_base_url() {
        let url = resolve_gemini_base_url(
            Some("http://127.0.0.1:1234"),
            None,
        );
        assert_eq!(url, DEFAULT_GEMINI_BASE);
    }

    #[test]
    fn resolve_honors_gemini_env_base() {
        let url = resolve_gemini_base_url(
            Some("http://127.0.0.1:1234"),
            Some("https://generativelanguage.googleapis.com/v1beta".into()),
        );
        assert_eq!(url, "https://generativelanguage.googleapis.com/v1beta");
    }

    #[test]
    fn partition_extracts_system() {
        let messages = vec![
            ChatMessage::system("rules"),
            ChatMessage::user("hello"),
        ];
        let (sys, contents) = GeminiConnector::partition_messages(&messages);
        assert_eq!(sys.as_deref(), Some("rules"));
        assert_eq!(contents.len(), 1);
    }
}
