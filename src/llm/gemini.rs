use std::io::Read;

use reqwest::blocking::Client;
use serde::Deserialize;
use serde_json::json;

use super::completion::CompletionResult;
use super::connector::{ChatMessage, ConnectorError, LlmConfig, LlmConnector, LlmProvider, StreamStats};
use super::sse::{SseEvent, SseEventKind, SseParser};
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
pub fn resolve_gemini_base_url(
    configured: Option<&str>,
    env_gemini_base: Option<String>,
) -> String {
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
        if config.api_key.as_ref().filter(|k| !k.is_empty()).is_none() {
            return Err(ConnectorError::MissingApiKey);
        }

        let client = Client::builder().timeout(config.timeout).build()?;
        crate::llm::connector::require_absolute_http_base(&config.base_url)?;
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
                "system" => system_lines.push(m.content.as_text()),
                "assistant" => contents.push(json!({
                    "role": "model",
                    "parts": m.content.gemini_parts()
                })),
                _ => contents.push(json!({
                    "role": "user",
                    "parts": m.content.gemini_parts()
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

    /// リクエスト本文（`complete` と `complete_stream` で共通）。
    fn build_request_body(
        &self,
        messages: &[ChatMessage],
    ) -> Result<serde_json::Value, ConnectorError> {
        let (system_instruction, contents) = Self::partition_messages(messages);
        if contents.is_empty() {
            return Err(ConnectorError::InvalidResponse(
                "no user/assistant messages for Gemini".into(),
            ));
        }

        let mut body = json!({
            "contents": contents,
            "generationConfig": {
                "temperature": 0.2,
                "maxOutputTokens": self.config.max_tokens
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

        Ok(body)
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

/// ``:streamGenerateContent?alt=sse`` の 1 チャンク（`data:` の JSON）。
/// Gemini の JSON はキャメルケース（`usageMetadata`）。
#[derive(Deserialize, Default)]
struct StreamChunk {
    candidates: Option<Vec<Candidate>>,
    error: Option<serde_json::Value>,
    #[serde(default, rename = "usageMetadata")]
    usage_metadata: Option<StreamUsage>,
}

#[derive(Deserialize, Default)]
struct StreamUsage {
    #[serde(default, rename = "promptTokenCount")]
    prompt_token_count: Option<u32>,
    #[serde(default, rename = "candidatesTokenCount")]
    candidates_token_count: Option<u32>,
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

        let (_, contents) = Self::partition_messages(messages);
        if contents.is_empty() {
            return Err(ConnectorError::InvalidResponse(
                "no user/assistant messages for Gemini".into(),
            ));
        }

        let url = format!(
            "{}/models/{}:generateContent?key={}",
            self.config.base_url, self.config.model, api_key
        );

        let body = self.build_request_body(messages)?;

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
            if let Some(msg) = err
                .get("error")
                .and_then(|e| e.as_str())
                .or_else(|| err.pointer("/error/message").and_then(|m| m.as_str()))
            {
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

    fn can_stream(&self) -> bool {
        true
    }

    fn complete_stream(
        &self,
        messages: &[ChatMessage],
        on_token: &mut dyn FnMut(&str),
    ) -> Result<Option<StreamStats>, ConnectorError> {
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

        let body = self.build_request_body(messages)?;

        // Gemini のストリーミング専用のエンドポイント（SSE）。
        let url = format!(
            "{}/models/{}:streamGenerateContent?alt=sse&key={}",
            self.config.base_url, self.config.model, api_key
        );

        let request = self
            .client
            .post(&url)
            .header(reqwest::header::ACCEPT, "text/event-stream")
            .json(&body);
        let mut response = request.send()?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().unwrap_or_default();
            return Err(ConnectorError::Http {
                status: status.as_u16(),
                body: text,
            });
        }

        let mut stats = StreamStats::default();
        let mut stream_error: Option<String> = None;

        // 各 `data` チャンク: candidates[].content.parts[].text を順に on_token へ。
        let mut on_event = |ev: &SseEvent| {
            match ev.kind {
                SseEventKind::Done => {}
                SseEventKind::Error => {
                    stream_error =
                        Some(gemini_error_message(ev.data).unwrap_or("stream error".into()));
                }
                SseEventKind::Data => {
                    if ev.data.is_empty() {
                        return;
                    }
                    let Ok(chunk) = serde_json::from_slice::<StreamChunk>(ev.data) else {
                        return;
                    };
                    // Gemini はエラーを data JSON の `error` フィールドで送出する
                    if let Some(msg) = gemini_error_message(ev.data) {
                        stream_error = Some(msg);
                        return;
                    }
                    if let Some(usage) = chunk.usage_metadata {
                        if let Some(p) = usage.prompt_token_count {
                            stats.prompt_tokens = Some(p);
                        }
                        if let Some(c) = usage.candidates_token_count {
                            stats.completion_tokens = Some(c);
                        }
                    }
                    if let Some(candidates) = chunk.candidates {
                        for candidate in candidates {
                            for part in candidate.content.parts {
                                if let Some(text) = part.text {
                                    on_token(&text);
                                    stats.chunks += 1;
                                }
                            }
                        }
                    }
                }
            }
        };

        // 逐次チャンクを読み SseParser（#659）に給餌する（#660 と同じパターン）。
        let mut parser = SseParser::new();
        loop {
            let mut buf = [0u8; 64 * 1024];
            let n = response.read(&mut buf).map_err(ConnectorError::Stream)?;
            if n == 0 {
                break;
            }
            parser.feed(&buf[..n], &mut on_event);
        }
        parser.finalize(&mut on_event);

        if let Some(msg) = stream_error {
            return Err(ConnectorError::InvalidResponse(msg));
        }
        if stats.chunks == 0 {
            return Err(ConnectorError::InvalidResponse("empty candidates".into()));
        }
        Ok(Some(stats))
    }
}

/// Gemini のエラーペイロード（`{"error":{"message":...}}` 等）からメッセージを取り出す。
fn gemini_error_message(data: &[u8]) -> Option<String> {
    let value: serde_json::Value = serde_json::from_slice(data).ok()?;
    value
        .pointer("/error/message")
        .and_then(|m| m.as_str())
        .map(|m| m.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn normalize_default_base() {
        assert_eq!(normalize_gemini_base_url(""), DEFAULT_GEMINI_BASE);
    }

    #[test]
    fn resolve_ignores_lmstudio_base_url() {
        let url = resolve_gemini_base_url(Some("http://127.0.0.1:1234"), None);
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
        let messages = vec![ChatMessage::system("rules"), ChatMessage::user("hello")];
        let (sys, contents) = GeminiConnector::partition_messages(&messages);
        assert_eq!(sys.as_deref(), Some("rules"));
        assert_eq!(contents.len(), 1);
    }

    fn test_config(base_url: &str) -> LlmConfig {
        LlmConfig {
            provider: LlmProvider::Gemini,
            api_key: Some("test-key".into()),
            base_url: base_url.to_string(),
            model: "gemini-test".into(),
            timeout: std::time::Duration::from_secs(10),
            max_tokens: 128,
            json_mode: false,
        }
    }

    #[test]
    fn gemini_can_stream_is_true() {
        let c = GeminiConnector::new(test_config("https://generativelanguage.googleapis.com/v1beta")).unwrap();
        assert!(c.can_stream());
    }

    fn fake_stream_server(sse: &str) -> (u16, std::thread::JoinHandle<()>) {
        let sse = sse.to_string();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let _ = sock.set_read_timeout(Some(std::time::Duration::from_millis(500)));
            let mut acc = Vec::new();
            let mut buf = [0u8; 4096];
            while !acc.windows(4).any(|w| w == b"\r\n\r\n") {
                match std::io::Read::read(&mut sock, &mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        acc.extend_from_slice(&buf[..n]);
                        if acc.len() > 64 * 1024 {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n"
            );
            sock.write_all(head.as_bytes()).unwrap();
            let (first, rest) = sse.split_at(sse.len() / 2);
            sock.write_all(first.as_bytes()).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(30));
            sock.write_all(rest.as_bytes()).unwrap();
            let _ = sock.flush();
        });
        (port, server)
    }

    #[test]
    fn gemini_complete_stream_emits_parts_and_usage() {
        let sse = concat!(
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Hel\"}]}}]}\n\n",
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"lo\"}]}}]}\n\n",
            "data: {\"candidates\":[{\"content\":{\"parts\":[]}}],\"usageMetadata\":{\"promptTokenCount\":7,\"candidatesTokenCount\":2}}\n\n",
        );
        let (port, server) = fake_stream_server(sse);
        let conn = GeminiConnector::new(test_config(
            &format!("http://127.0.0.1:{port}/v1beta"),
        ))
        .unwrap();

        let mut got = String::new();
        let mut count = 0usize;
        let stats = conn
            .complete_stream(
                &[ChatMessage::user("hi")],
                &mut |t| {
                    got.push_str(t);
                    count += 1;
                },
            )
            .unwrap()
            .expect("stream 統計は Some");
        server.join().unwrap();

        assert_eq!(got, "Hello");
        assert_eq!(count, 2);
        assert_eq!(stats.chunks, 2);
        assert_eq!(stats.prompt_tokens, Some(7));
        assert_eq!(stats.completion_tokens, Some(2));
    }

    #[test]
    fn gemini_complete_stream_reports_error_payload() {
        let sse = "data: {\"error\":{\"code\":429,\"message\":\"quota exceeded\"}}\n\n";
        let (port, server) = fake_stream_server(sse);
        let conn = GeminiConnector::new(test_config(
            &format!("http://127.0.0.1:{port}/v1beta"),
        ))
        .unwrap();

        let err = conn
            .complete_stream(&[ChatMessage::user("hi")], &mut |_| ())
            .unwrap_err();
        server.join().unwrap();

        let msg = format!("{err}");
        assert!(msg.contains("quota exceeded"), "エラーメッセージが伝播: {msg}");
    }
}
