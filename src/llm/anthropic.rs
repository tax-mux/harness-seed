use std::io::Read;

use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use serde::Deserialize;
use serde_json::json;

use super::completion::CompletionResult;
use super::connector::{ChatMessage, ConnectorError, LlmConfig, LlmConnector, LlmProvider, StreamStats};
use super::sse::{SseEvent, SseEventKind, SseParser};
use crate::context_metrics::{format_messages_body, ContextUsage};

const DEFAULT_ANTHROPIC_BASE: &str = "https://api.anthropic.com";
/// Messages API の `anthropic-version`（[Anthropic API](https://docs.anthropic.com)）。
const ANTHROPIC_VERSION: &str = "2023-06-01";
/// Anthropic API のルート URL（`/v1/messages` はコネクタ側で付与）。
pub fn normalize_anthropic_base_url(host: &str) -> String {
    let trimmed = host.trim().trim_end_matches('/');
    let base = if trimmed.is_empty() {
        DEFAULT_ANTHROPIC_BASE
    } else {
        trimmed
    };
    base.strip_suffix("/v1").unwrap_or(base).to_string()
}

/// Anthropic Messages API コネクタ（Claude 直）。
#[derive(Debug)]
pub struct AnthropicConnector {
    client: Client,
    config: LlmConfig,
}

impl AnthropicConnector {
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

    fn messages_url(&self) -> String {
        format!("{}/v1/messages", self.config.base_url)
    }

    fn partition_messages(messages: &[ChatMessage]) -> (Option<String>, Vec<serde_json::Value>) {
        let mut system_lines = Vec::new();
        let mut api_messages = Vec::new();

        for m in messages {
            match m.role.as_str() {
                "system" => system_lines.push(m.content.as_text()),
                "assistant" => api_messages.push(json!({
                    "role": "assistant",
                    "content": m.content.as_text()
                })),
                _ => api_messages.push(json!({
                    "role": "user",
                    "content": m.content.anthropic_content()
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

    /// リクエスト本文（`complete` と `complete_stream` で共通）。`stream` で切り替え。
    fn build_request_body(
        &self,
        messages: &[ChatMessage],
        stream: bool,
    ) -> Result<serde_json::Value, ConnectorError> {
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
            "max_tokens": self.config.max_tokens,
            "temperature": 0.2,
            "messages": api_messages
        });

        if let Some(system) = system {
            body["system"] = json!(system);
        }
        if stream {
            body["stream"] = json!(true);
        }

        Ok(body)
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

/// Streaming SSE の 1 イベント（`data:` の JSON）。`type` で種別を判別する。
#[derive(Deserialize, Default)]
struct StreamEvent {
    #[serde(rename = "type")]
    event_type: Option<String>,
    message: Option<serde_json::Value>,
    delta: Option<serde_json::Value>,
    usage: Option<Usage>,
    error: Option<serde_json::Value>,
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

        let body = self.build_request_body(messages, false)?;

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

        let body = self.build_request_body(messages, true)?;

        let request = self
            .client
            .post(self.messages_url())
            .headers(Self::auth_headers(api_key)?)
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

        // Anthropic のストリームイベント:
        //   message_start      → message.usage.input_tokens
        //   content_block_delta → delta.text を on_token
        //   message_delta      → usage.output_tokens
        //   error              → エラー終了
        let mut on_event = |ev: &SseEvent| {
            match ev.kind {
                SseEventKind::Done => {}
                SseEventKind::Error => {
                    stream_error = Some(Self::error_message(ev.data).unwrap_or("stream error".into()));
                }
                SseEventKind::Data => {
                    if ev.data.is_empty() {
                        return;
                    }
                    let Ok(event) = serde_json::from_slice::<StreamEvent>(ev.data) else {
                        return;
                    };
                    if let Some(msg) = Self::error_message(ev.data) {
                        stream_error = Some(msg);
                        return;
                    }
                    match event.event_type.as_deref() {
                        // message_start の message.usage から input_tokens
                        Some("message_start") => {
                            if let Some(p) = event
                                .message
                                .as_ref()
                                .and_then(|m| m.pointer("/usage/input_tokens"))
                                .and_then(|v| v.as_u64())
                                .map(|v| v as u32)
                            {
                                stats.prompt_tokens = Some(p);
                            }
                        }
                        Some("content_block_delta") => {
                            if let Some(text) = event
                                .delta
                                .as_ref()
                                .and_then(|d| d.get("text"))
                                .and_then(|t| t.as_str())
                            {
                                on_token(text);
                                stats.chunks += 1;
                            }
                        }
                        Some("message_delta") => {
                            if let Some(c) = event.usage.as_ref().and_then(|u| u.output_tokens) {
                                stats.completion_tokens = Some(c);
                            }
                        }
                        // ping / message_stop / content_block_* 等は無視
                        _ => {}
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
            return Err(ConnectorError::InvalidResponse("empty content blocks".into()));
        }
        Ok(Some(stats))
    }
}

impl AnthropicConnector {
    /// `event: error` / data の `type: "error"` からメッセージを取り出す。
    fn error_message(data: &[u8]) -> Option<String> {
        let value: serde_json::Value = serde_json::from_slice(data).ok()?;
        value
            .pointer("/error/message")
            .and_then(|m| m.as_str())
            .map(|m| m.to_string())
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
        let messages = vec![ChatMessage::system("rules"), ChatMessage::user("hello")];
        let (sys, msgs) = AnthropicConnector::partition_messages(&messages);
        assert_eq!(sys.as_deref(), Some("rules"));
        assert_eq!(msgs.len(), 1);
    }

    fn test_config(base_url: &str) -> LlmConfig {
        LlmConfig {
            provider: LlmProvider::Anthropic,
            api_key: Some("sk-test".into()),
            base_url: base_url.to_string(),
            model: "claude-test".into(),
            timeout: std::time::Duration::from_secs(10),
            max_tokens: 128,
            json_mode: false,
        }
    }

    #[test]
    fn anthropic_can_stream_is_true() {
        let c = AnthropicConnector::new(test_config("https://api.anthropic.com")).unwrap();
        assert!(c.can_stream());
    }

    fn fake_stream_server(sse: &str) -> (u16, std::thread::JoinHandle<()>) {
        let sse = sse.to_string();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            use std::io::{Read, Write};
            let (mut sock, _) = listener.accept().unwrap();
            let _ = sock.set_read_timeout(Some(std::time::Duration::from_millis(500)));
            let mut acc = Vec::new();
            let mut buf = [0u8; 4096];
            while !acc.windows(4).any(|w| w == b"\r\n\r\n") {
                match sock.read(&mut buf) {
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
    fn anthropic_complete_stream_emits_deltas_and_usage() {
        let sse = concat!(
            "event: message_start\n",
            "data: {\"type\":\"message_start\",\"message\":{\"id\":\"m\",\"usage\":{\"input_tokens\":3}}}\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"Hel\"}}\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"lo\"}}\n\n",
            "event: message_delta\n",
            "data: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":5}}\n\n",
        );
        let (port, server) = fake_stream_server(sse);
        let conn = AnthropicConnector::new(test_config(
            &format!("http://127.0.0.1:{port}"),
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

        assert_eq!(got, "Hello", "content_block_delta の delta.text が順に on_token へ");
        assert_eq!(count, 2);
        assert_eq!(stats.chunks, 2);
        assert_eq!(stats.prompt_tokens, Some(3), "message_start の input_tokens");
        assert_eq!(stats.completion_tokens, Some(5), "message_delta の output_tokens");
    }

    #[test]
    fn anthropic_complete_stream_reports_error_event() {
        let sse = concat!(
            "event: error\n",
            "data: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"overloaded\"}}\n\n",
        );
        let (port, server) = fake_stream_server(sse);
        let conn = AnthropicConnector::new(test_config(
            &format!("http://127.0.0.1:{port}"),
        ))
        .unwrap();

        let err = conn
            .complete_stream(&[ChatMessage::user("hi")], &mut |_| ())
            .unwrap_err();
        server.join().unwrap();

        let msg = format!("{err}");
        assert!(msg.contains("overloaded"), "エラーメッセージが伝播: {msg}");
    }
}
