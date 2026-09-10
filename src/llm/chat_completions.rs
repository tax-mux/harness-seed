use std::io::Read;

use reqwest::blocking::Client;
use reqwest::blocking::RequestBuilder;
use serde::Deserialize;

use super::completion::CompletionResult;
use super::connector::{ChatMessage, ConnectorError, LlmConfig, LlmConnector, StreamStats};
use super::sse::{SseEvent, SseEventKind, SseParser};
use crate::context_metrics::{format_messages_body, ContextUsage};

/// OpenAI 互換 Chat Completions API クライアント（OpenAI / Ollama / LM Studio 共通）。
#[derive(Debug)]
pub struct ChatCompletionsConnector {
    client: Client,
    config: LlmConfig,
}

impl ChatCompletionsConnector {
    pub fn new(config: LlmConfig) -> Result<Self, ConnectorError> {
        crate::llm::connector::require_absolute_http_base(&config.base_url)?;
        let client = Client::builder().timeout(config.timeout).build()?;
        Ok(Self { client, config })
    }

    pub fn config(&self) -> &LlmConfig {
        &self.config
    }

    fn apply_auth(&self, request: RequestBuilder) -> RequestBuilder {
        if let Some(key) = &self.config.api_key {
            if !key.is_empty() {
                return request.bearer_auth(key);
            }
        }
        request
    }
}

#[derive(serde::Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: &'a [ChatMessage],
    temperature: f32,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<ResponseFormat>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    stream: bool,
}

/// SSE チャンク（`data:` の JSON）。
///
/// OpenAI 互換のストリームチャンク: `choices[0].delta.content` がトークンスライス。
/// `usage`（OpenAI）/ `prompt_eval_count`・`eval_count`（Ollama）は末尾チャンクに
/// 付与されることがある。
#[derive(Deserialize, Default)]
struct StreamChunk {
    choices: Option<Vec<StreamChoice>>,
    usage: Option<Usage>,
    prompt_eval_count: Option<u32>,
    eval_count: Option<u32>,
}

#[derive(Deserialize, Default)]
struct StreamChoice {
    delta: StreamDelta,
}

#[derive(Deserialize, Default)]
struct StreamDelta {
    content: Option<String>,
}

#[derive(serde::Serialize)]
struct ResponseFormat {
    #[serde(rename = "type")]
    format_type: &'static str,
}

#[derive(Deserialize, Default)]
struct Usage {
    prompt_tokens: Option<u32>,
    completion_tokens: Option<u32>,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
    #[serde(default)]
    usage: Usage,
    prompt_eval_count: Option<u32>,
    eval_count: Option<u32>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

fn resolve_token_usage(parsed: &ChatResponse) -> (Option<u32>, Option<u32>) {
    let mut prompt = parsed.usage.prompt_tokens;
    let mut completion = parsed.usage.completion_tokens;

    if prompt.is_none() {
        prompt = parsed.prompt_eval_count;
    }
    if completion.is_none() {
        completion = parsed.eval_count;
    }

    (prompt, completion)
}

impl LlmConnector for ChatCompletionsConnector {
    fn provider(&self) -> super::connector::LlmProvider {
        self.config.provider
    }

    fn complete(&self, messages: &[ChatMessage]) -> Result<CompletionResult, ConnectorError> {
        let url = format!("{}/chat/completions", self.config.base_url);
        let response_format = self.config.json_mode.then(|| ResponseFormat {
            format_type: "json_object",
        });

        let body = ChatRequest {
            model: &self.config.model,
            messages,
            temperature: 0.2,
            max_tokens: self.config.max_tokens,
            response_format,
            stream: false,
        };

        let request = self.client.post(&url).json(&body);
        let response = self.apply_auth(request).send()?;

        let status = response.status();
        let text = response.text()?;
        if !status.is_success() {
            return Err(ConnectorError::Http {
                status: status.as_u16(),
                body: text,
            });
        }

        let parsed: ChatResponse = serde_json::from_str(&text)
            .map_err(|e| ConnectorError::InvalidResponse(format!("{e}; body={text}")))?;

        let (prompt_tokens, completion_tokens) = resolve_token_usage(&parsed);

        let content = parsed
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content.as_text())
            .ok_or_else(|| ConnectorError::InvalidResponse("empty choices".into()))?;

        let usage = ContextUsage::from_parts(
            &format_messages_body(messages),
            &content,
            prompt_tokens,
            completion_tokens,
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
        let url = format!("{}/chat/completions", self.config.base_url);
        let response_format = self.config.json_mode.then(|| ResponseFormat {
            format_type: "json_object",
        });

        let body = ChatRequest {
            model: &self.config.model,
            messages,
            temperature: 0.2,
            max_tokens: self.config.max_tokens,
            response_format,
            stream: true,
        };

        let request = self
            .client
            .post(&url)
            .header(reqwest::header::ACCEPT, "text/event-stream")
            .json(&body);
        let mut response = self.apply_auth(request).send()?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().unwrap_or_default();
            return Err(ConnectorError::Http {
                status: status.as_u16(),
                body,
            });
        }

        let mut stats = StreamStats::default();
        let mut stream_error: Option<String> = None;

        // イベントハンドラ：`data` JSON を解き、トークンを on_token へ。
        let mut on_event = |ev: &SseEvent| {
            match ev.kind {
                SseEventKind::Done => {}
                SseEventKind::Error => {
                    stream_error = Some(String::from_utf8_lossy(ev.data).into_owned());
                }
                SseEventKind::Data => {
                    if ev.data.is_empty() {
                        return;
                    }
                    // `event: ping` 等の JSON ではない data は無視
                    let Ok(chunk) = serde_json::from_slice::<StreamChunk>(ev.data) else {
                        return;
                    };
                    collect_stream_usage(&chunk, &mut stats);
                    if let Some(choices) = chunk.choices {
                        for choice in choices {
                            if let Some(token) = choice.delta.content {
                                on_token(&token);
                                stats.chunks += 1;
                            }
                        }
                    }
                }
            }
        };

        // ストリーミング本体：逐次チャンクを読み SseParser（#659）に給餌する。
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
            return Err(ConnectorError::InvalidResponse(format!("stream error: {msg}")));
        }
        Ok(Some(stats))
    }
}

/// ストリームチャンクから usage（OpenAI `usage` / Ollama `*_eval_count`）を取り出す。
fn collect_stream_usage(chunk: &StreamChunk, stats: &mut StreamStats) {
    if let Some(usage) = &chunk.usage {
        if let Some(p) = usage.prompt_tokens {
            stats.prompt_tokens = Some(p);
        }
        if let Some(c) = usage.completion_tokens {
            stats.completion_tokens = Some(c);
        }
    }
    if let Some(p) = chunk.prompt_eval_count {
        stats.prompt_tokens = Some(p);
    }
    if let Some(c) = chunk.eval_count {
        stats.completion_tokens = Some(c);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::connector::LlmProvider;
    use std::io::{Read, Write};
    use std::time::Duration;

    #[test]
    fn chat_request_serializes_max_tokens() {
        let messages = [ChatMessage::user("hi")];
        let body = ChatRequest {
            model: "ornith-1.5:35b",
            messages: &messages,
            temperature: 0.2,
            max_tokens: 16384,
            response_format: None,
            stream: false,
        };
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["max_tokens"], 16384);
        assert_eq!(json["model"], "ornith-1.5:35b");
        assert!(
            json.get("stream").is_none(),
            "非ストリーム要求では stream フィールドを除外"
        );
    }

    #[test]
    fn chat_request_serializes_stream_true() {
        let messages = [ChatMessage::user("hi")];
        let body = ChatRequest {
            model: "gemma-4-26b",
            messages: &messages,
            temperature: 0.2,
            max_tokens: 1024,
            response_format: None,
            stream: true,
        };
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["stream"], true);
    }

    #[test]
    fn stream_chunk_parses_delta_and_ollama_usage() {
        // OpenAI 互換チャンク
        let c: StreamChunk =
            serde_json::from_str(r#"{"choices":[{"delta":{"content":"He"},"index":0}]}"#)
                .unwrap();
        assert_eq!(
            c.choices.as_deref().unwrap()[0]
                .delta
                .content
                .as_deref(),
            Some("He")
        );
        let mut s = StreamStats::default();
        collect_stream_usage(&c, &mut s);
        assert_eq!(s.prompt_tokens, None);

        // Ollama usage チャンク
        let o: StreamChunk = serde_json::from_str(
            r#"{"choices":[],"prompt_eval_count":11,"eval_count":3,"done":true}"#,
        )
        .unwrap();
        collect_stream_usage(&o, &mut s);
        assert_eq!(s.prompt_tokens, Some(11));
        assert_eq!(s.completion_tokens, Some(3));

        // OpenAI usage（stream_options.include_usage）
        let u: StreamChunk = serde_json::from_str(
            r#"{"usage":{"prompt_tokens":5,"completion_tokens":9}}"#,
        )
        .unwrap();
        collect_stream_usage(&u, &mut s);
        assert_eq!(s.prompt_tokens, Some(5));
        assert_eq!(s.completion_tokens, Some(9));
    }

    #[test]
    fn chat_completions_can_stream_is_true() {
        let cfg = LlmConfig {
            provider: LlmProvider::Ollama,
            api_key: None,
            base_url: "http://127.0.0.1:11434/v1".into(),
            model: "gemma".into(),
            timeout: Duration::from_secs(5),
            max_tokens: 128,
            json_mode: false,
        };
        let c = ChatCompletionsConnector::new(cfg).unwrap();
        assert!(c.can_stream());
    }

    /// ローカル TCP SSE 偽サーバーでストリーミング実装を検証する。
    ///
    /// 応答を2分割して送り返し、逐次チャンク読み（`Read for Response`）と
    /// `SseParser` の組み合わせがトークン単位で `on_token` に出すことを確認する。
    #[test]
    fn complete_stream_emits_tokens_per_sse_event() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        let sse = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"lo\"}}]}\n\n",
            "data: {\"choices\":[],\"prompt_eval_count\":11,\"eval_count\":3}\n\n",
            "data: [DONE]\n\n",
        );

        let server = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let _ = sock.set_read_timeout(Some(Duration::from_millis(500)));
            let mut seen_headers = false;
            let mut buf = [0u8; 4096];
            let mut acc = Vec::new();
            while !seen_headers {
                match sock.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        acc.extend_from_slice(&buf[..n]);
                        if memmem(&acc, b"\r\n\r\n") {
                            seen_headers = true;
                        }
                    }
                    Err(_) => break,
                }
            }
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n"
            );
            sock.write_all(head.as_bytes()).unwrap();
            // 2 分割で送り返す（逐次チャンク読み込みの確認）
            let half = sse.len() / 2;
            sock.write_all(sse[..half].as_bytes()).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(50));
            sock.write_all(sse[half..].as_bytes()).unwrap();
            let _ = sock.flush();
        });

        let cfg = LlmConfig {
            provider: LlmProvider::Ollama,
            api_key: None,
            base_url: format!("http://127.0.0.1:{port}/v1"),
            model: "gemma".into(),
            timeout: Duration::from_secs(10),
            max_tokens: 128,
            json_mode: false,
        };
        let conn = ChatCompletionsConnector::new(cfg).unwrap();

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

        assert_eq!(got, "Hello", "各チャンクの delta.content が順に on_token へ");
        assert_eq!(count, 2, "トークンチャンクごとに 1 回だけ発火");
        assert_eq!(stats.chunks, 2);
        assert_eq!(stats.prompt_tokens, Some(11));
        assert_eq!(stats.completion_tokens, Some(3));
    }

    /// リトルヘルパー：`needle` が `hay` に含まれるか。
    fn memmem(hay: &[u8], needle: &[u8]) -> bool {
        hay.windows(needle.len()).any(|w| w == needle)
    }
}

