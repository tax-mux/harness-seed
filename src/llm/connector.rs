use std::fmt;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// LLM バックエンド種別。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmProvider {
    OpenAi,
    Ollama,
    LmStudio,
    Gemini,
    Anthropic,
}

impl LlmProvider {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Ollama => "ollama",
            Self::LmStudio => "lmstudio",
            Self::Gemini => "gemini",
            Self::Anthropic => "anthropic",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "openai" => Some(Self::OpenAi),
            "ollama" => Some(Self::Ollama),
            "lmstudio" | "lm_studio" | "lm-studio" => Some(Self::LmStudio),
            "gemini" | "google" => Some(Self::Gemini),
            "anthropic" | "claude" => Some(Self::Anthropic),
            _ => None,
        }
    }
}

/// チャットメッセージ本文（テキストまたは OpenAI 互換マルチモーダル parts）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Parts(Vec<ContentPart>),
}

impl MessageContent {
    pub fn text(value: impl Into<String>) -> Self {
        Self::Text(value.into())
    }

    pub fn as_text(&self) -> String {
        match self {
            Self::Text(s) => s.clone(),
            Self::Parts(parts) => parts
                .iter()
                .filter_map(|p| match p {
                    ContentPart::Text { text } => Some(text.as_str()),
                    ContentPart::ImageUrl { .. } => None,
                })
                .collect::<Vec<_>>()
                .join(""),
        }
    }

    pub fn image_count(&self) -> usize {
        match self {
            Self::Text(_) => 0,
            Self::Parts(parts) => parts
                .iter()
                .filter(|p| matches!(p, ContentPart::ImageUrl { .. }))
                .count(),
        }
    }

    /// Gemini `generateContent` の `parts` 配列へ変換する。
    pub fn gemini_parts(&self) -> Vec<serde_json::Value> {
        use serde_json::json;
        match self {
            Self::Text(s) => vec![json!({ "text": s })],
            Self::Parts(parts) => parts
                .iter()
                .map(|part| match part {
                    ContentPart::Text { text } => json!({ "text": text }),
                    ContentPart::ImageUrl { image_url } => {
                        if let Some((mime, data)) = parse_data_url(&image_url.url) {
                            json!({
                                "inline_data": {
                                    "mime_type": mime,
                                    "data": data
                                }
                            })
                        } else {
                            json!({ "text": image_url.url.clone() })
                        }
                    }
                })
                .collect(),
        }
    }

    /// Anthropic Messages API の `content` フィールドへ変換する。
    pub fn anthropic_content(&self) -> serde_json::Value {
        use serde_json::json;
        match self {
            Self::Text(s) => json!(s),
            Self::Parts(parts) => {
                let blocks: Vec<serde_json::Value> = parts
                    .iter()
                    .map(|part| match part {
                        ContentPart::Text { text } => json!({
                            "type": "text",
                            "text": text
                        }),
                        ContentPart::ImageUrl { image_url } => {
                            if let Some((mime, data)) = parse_data_url(&image_url.url) {
                                json!({
                                    "type": "image",
                                    "source": {
                                        "type": "base64",
                                        "media_type": mime,
                                        "data": data
                                    }
                                })
                            } else {
                                json!({
                                    "type": "text",
                                    "text": image_url.url.clone()
                                })
                            }
                        }
                    })
                    .collect();
                json!(blocks)
            }
        }
    }
}

fn parse_data_url(url: &str) -> Option<(String, String)> {
    let rest = url.strip_prefix("data:")?;
    let (mime, data) = rest.split_once(";base64,")?;
    Some((mime.to_string(), data.to_string()))
}

/// OpenAI Chat Completions 互換の content part。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPart {
    Text { text: String },
    ImageUrl { image_url: ImageUrlDetail },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageUrlDetail {
    pub url: String,
}

/// チャットメッセージ（OpenAI / Ollama 互換）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatMessage {
    pub role: String,
    pub content: MessageContent,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".into(),
            content: MessageContent::text(content),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            content: MessageContent::text(content),
        }
    }

    pub fn user_with_vision(
        text: impl Into<String>,
        images: &[crate::context_manifest::VisionAttachment],
    ) -> Self {
        Self::user_with_reference_vision("## Reference attachments", images, text)
    }

    /// 固定ゾーン参照（画像）+ 実行層 user 本文。画像は参照情報として先頭に載せる。
    pub fn user_with_reference_vision(
        reference_header: impl Into<String>,
        images: &[crate::context_manifest::VisionAttachment],
        operational: impl Into<String>,
    ) -> Self {
        let mut parts = vec![ContentPart::Text {
            text: reference_header.into(),
        }];
        for image in images {
            parts.push(ContentPart::Text {
                text: format!(
                    "\n[reference image] entry={} path={}",
                    image.entry_id,
                    image.path.display()
                ),
            });
            parts.push(ContentPart::ImageUrl {
                image_url: ImageUrlDetail {
                    url: format!("data:{};base64,{}", image.mime, image.base64),
                },
            });
        }
        parts.push(ContentPart::Text {
            text: format!("\n\n---\n\n{}", operational.into()),
        });
        Self {
            role: "user".into(),
            content: MessageContent::Parts(parts),
        }
    }
}

/// 接続設定。
#[derive(Debug, Clone)]
pub struct LlmConfig {
    pub provider: LlmProvider,
    pub api_key: Option<String>,
    pub base_url: String,
    pub model: String,
    pub timeout: Duration,
    /// `response_format: json_object` を付与するか（Ollama では通常 false）。
    pub json_mode: bool,
}

impl LlmConfig {
    pub fn from_app(app: &crate::config::AppConfig) -> Result<Self, ConnectorError> {
        app.build_llm_config()
    }

    pub fn from_env() -> Result<Self, ConnectorError> {
        let app = crate::config::AppConfig::load_default().map_err(|e| {
            ConnectorError::Config(e.to_string())
        })?;
        Self::from_app(&app)
    }

    pub fn is_available() -> bool {
        crate::config::AppConfig::load_default()
            .map(|a| a.llm_available())
            .unwrap_or(false)
    }
}

/// OpenAI 互換 API のベース URL に `/v1` を付与する。
pub fn normalize_openai_compatible_base_url(host: &str) -> String {
    let trimmed = host.trim().trim_end_matches('/');
    if trimmed.ends_with("/v1") {
        trimmed.to_string()
    } else {
        format!("{trimmed}/v1")
    }
}

/// `OLLAMA_HOST` 等を OpenAI 互換の `/v1` 付き URL に正規化する。
pub fn normalize_ollama_base_url(host: &str) -> String {
    normalize_openai_compatible_base_url(host)
}

/// LM Studio ローカルサーバー URL を正規化する（既定 `http://127.0.0.1:1234`）。
pub fn normalize_lmstudio_base_url(host: &str) -> String {
    normalize_openai_compatible_base_url(host)
}

#[derive(Debug)]
pub enum ConnectorError {
    MissingApiKey,
    Config(String),
    Http { status: u16, body: String },
    Request(reqwest::Error),
    InvalidResponse(String),
}

impl fmt::Display for ConnectorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingApiKey => write!(
                f,
                "API key not set (config llm.api_key, OPENAI_API_KEY, GEMINI_API_KEY, ANTHROPIC_API_KEY, or HARNESS_SEED_API_KEY)"
            ),
            Self::Config(msg) => write!(f, "config error: {msg}"),
            Self::Http { status, body } => write!(f, "HTTP {status}: {body}"),
            Self::Request(e) => write!(f, "request error: {e}"),
            Self::InvalidResponse(msg) => write!(f, "invalid response: {msg}"),
        }
    }
}

impl std::error::Error for ConnectorError {}

impl From<reqwest::Error> for ConnectorError {
    fn from(value: reqwest::Error) -> Self {
        Self::Request(value)
    }
}

use super::completion::CompletionResult;

/// LLM API への抽象接続。
pub trait LlmConnector {
    fn complete(&self, messages: &[ChatMessage]) -> Result<CompletionResult, ConnectorError>;
    fn provider(&self) -> LlmProvider {
        LlmProvider::OpenAi
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_ollama_host_adds_v1() {
        assert_eq!(
            normalize_ollama_base_url("http://127.0.0.1:11434"),
            "http://127.0.0.1:11434/v1"
        );
    }

    #[test]
    fn normalize_ollama_host_keeps_existing_v1() {
        assert_eq!(
            normalize_ollama_base_url("http://localhost:11434/v1/"),
            "http://localhost:11434/v1"
        );
    }

    #[test]
    fn normalize_lmstudio_adds_v1() {
        assert_eq!(
            normalize_lmstudio_base_url("http://127.0.0.1:1234"),
            "http://127.0.0.1:1234/v1"
        );
    }

    #[test]
    fn parses_lmstudio_provider_name() {
        assert_eq!(LlmProvider::parse("lmstudio"), Some(LlmProvider::LmStudio));
        assert_eq!(LlmProvider::parse("lm-studio"), Some(LlmProvider::LmStudio));
    }

    #[test]
    fn parses_gemini_provider_name() {
        assert_eq!(LlmProvider::parse("gemini"), Some(LlmProvider::Gemini));
        assert_eq!(LlmProvider::parse("google"), Some(LlmProvider::Gemini));
    }

    #[test]
    fn parses_anthropic_provider_name() {
        assert_eq!(
            LlmProvider::parse("anthropic"),
            Some(LlmProvider::Anthropic)
        );
        assert_eq!(LlmProvider::parse("claude"), Some(LlmProvider::Anthropic));
    }
}
