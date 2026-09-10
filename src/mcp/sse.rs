//! MCP SSE トランスポート（`url` + optional `headers`）。
//!
//! - Legacy HTTP+SSE (2024-11-05): GET /sse → `endpoint` イベント → POST /message
//! - Streamable HTTP (TelosPVL 等): POST /sse で initialize → `mcp-session-id` ヘッダ

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;

use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, ACCEPT, CONTENT_TYPE};
use reqwest::StatusCode;
use serde_json::{json, Value};

use super::result::unwrap_tool_result;
use super::transport::{McpError, McpToolInfo, McpTransport};

enum SseMode {
    Legacy {
        post_url: String,
        pending: Arc<(Mutex<HashMap<u64, Value>>, Condvar)>,
    },
    Streamable {
        url: String,
        session_id: String,
    },
}

pub struct SseMcpTransport {
    mode: SseMode,
    client: Client,
    headers: HeaderMap,
    next_id: AtomicU64,
    timeout: std::time::Duration,
}

impl SseMcpTransport {
    pub fn connect(
        url: &str,
        header_map: &std::collections::BTreeMap<String, String>,
        timeout: std::time::Duration,
    ) -> Result<Self, McpError> {
        let mut headers = HeaderMap::new();
        for (k, v) in header_map {
            let name = HeaderName::from_bytes(k.as_bytes())
                .map_err(|e| McpError::Config(format!("header {k}: {e}")))?;
            let value = HeaderValue::from_str(v)
                .map_err(|e| McpError::Config(format!("header {k}: {e}")))?;
            headers.insert(name, value);
        }

        let client = Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| McpError::Io(e.to_string()))?;

        let mut get_req = client
            .get(url)
            .header(ACCEPT, "text/event-stream")
            .timeout(timeout);
        for (k, v) in headers.iter() {
            get_req = get_req.header(k, v);
        }
        let get_resp = get_req
            .send()
            .map_err(|e| McpError::Io(format!("sse connect {url}: {e}")))?;

        if get_resp.status().is_success() {
            return Self::connect_legacy(url, client, headers, timeout, get_resp);
        }

        if get_resp.status() == StatusCode::BAD_REQUEST {
            let text = get_resp.text().unwrap_or_default();
            if text.contains("Server not initialized") {
                return Self::connect_streamable(url, client, headers, timeout);
            }
            return Err(McpError::Io(format!(
                "sse connect {url} -> 400 Bad Request: {}",
                truncate(&text, 200)
            )));
        }

        Err(McpError::Io(format!(
            "sse connect {url} -> {}",
            get_resp.status()
        )))
    }

    fn connect_legacy(
        url: &str,
        client: Client,
        headers: HeaderMap,
        timeout: std::time::Duration,
        get_resp: reqwest::blocking::Response,
    ) -> Result<Self, McpError> {
        let pending = Arc::new((Mutex::new(HashMap::new()), Condvar::new()));
        let (post_url_tx, post_url_rx) = mpsc::channel();
        let base = base_url(url);
        let reader_pending = Arc::clone(&pending);
        thread::Builder::new()
            .name("mcp-sse-session".into())
            .spawn(move || {
                if let Err(err) =
                    sse_session_loop_from_response(get_resp, &base, post_url_tx, reader_pending)
                {
                    eprintln!("[mcp] sse session ended: {err}");
                }
            })
            .map_err(|e| McpError::Io(format!("spawn sse session: {e}")))?;

        let post_url = post_url_rx
            .recv_timeout(timeout)
            .map_err(|_| McpError::Timeout)?
            .map_err(|e| McpError::Io(format!("sse session: {e}")))?;

        let transport = Self {
            mode: SseMode::Legacy {
                post_url,
                pending,
            },
            client,
            headers,
            next_id: AtomicU64::new(1),
            timeout,
        };
        transport.handshake()?;
        Ok(transport)
    }

    fn connect_streamable(
        url: &str,
        client: Client,
        headers: HeaderMap,
        timeout: std::time::Duration,
    ) -> Result<Self, McpError> {
        let init_id = 1u64;
        let init_body = json!({
            "jsonrpc": "2.0",
            "id": init_id,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "harness-seed", "version": "0.1.0" }
            }
        });

        let mut req = client
            .post(url)
            .header(ACCEPT, "application/json, text/event-stream")
            .header(CONTENT_TYPE, "application/json")
            .json(&init_body)
            .timeout(timeout);
        for (k, v) in headers.iter() {
            req = req.header(k, v);
        }
        let resp = req
            .send()
            .map_err(|e| McpError::Io(format!("streamable initialize {url}: {e}")))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().unwrap_or_default();
            return Err(McpError::Io(format!(
                "streamable initialize {url} -> {status}: {}",
                truncate(&text, 200)
            )));
        }

        let session_id = resp
            .headers()
            .get("mcp-session-id")
            .or_else(|| resp.headers().get("Mcp-Session-Id"))
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| McpError::Io("streamable initialize: mcp-session-id missing".into()))?
            .to_string();

        let text = resp.text().unwrap_or_default();
        let _ = parse_rpc_response(&text, init_id)?;

        let transport = Self {
            mode: SseMode::Streamable {
                url: url.to_string(),
                session_id,
            },
            client,
            headers,
            next_id: AtomicU64::new(2),
            timeout,
        };
        transport.notify("notifications/initialized", json!({}))?;
        Ok(transport)
    }

    fn handshake(&self) -> Result<(), McpError> {
        let _ = self.rpc(
            self.next_id.fetch_add(1, Ordering::SeqCst),
            "initialize",
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "harness-seed", "version": "0.1.0" }
            }),
        )?;
        self.notify("notifications/initialized", json!({}))?;
        Ok(())
    }

    fn notify(&self, method: &str, params: Value) -> Result<(), McpError> {
        let body = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        let mut req = match &self.mode {
            SseMode::Legacy { post_url, .. } => self
                .client
                .post(post_url)
                .json(&body)
                .timeout(self.timeout),
            SseMode::Streamable { url, session_id } => self
                .client
                .post(url)
                .header(ACCEPT, "application/json, text/event-stream")
                .header(CONTENT_TYPE, "application/json")
                .header("mcp-session-id", session_id.as_str())
                .json(&body)
                .timeout(self.timeout),
        };
        for (k, v) in self.headers.iter() {
            req = req.header(k, v);
        }
        let resp = req
            .send()
            .map_err(|e| McpError::Io(format!("notify {method}: {e}")))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().unwrap_or_default();
            return Err(McpError::Io(format!(
                "notify {method} -> {status}: {}",
                truncate(&text, 200)
            )));
        }
        Ok(())
    }

    fn rpc(&self, id: u64, method: &str, params: Value) -> Result<Value, McpError> {
        let body = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });

        match &self.mode {
            SseMode::Streamable { url, session_id } => {
                let mut req = self
                    .client
                    .post(url)
                    .header(ACCEPT, "application/json, text/event-stream")
                    .header(CONTENT_TYPE, "application/json")
                    .header("mcp-session-id", session_id.as_str())
                    .json(&body)
                    .timeout(self.timeout);
                for (k, v) in self.headers.iter() {
                    req = req.header(k, v);
                }
                let resp = req
                    .send()
                    .map_err(|e| McpError::Io(format!("post {url}: {e}")))?;
                if !resp.status().is_success() {
                    let status = resp.status();
                    let text = resp.text().unwrap_or_default();
                    return Err(McpError::Io(format!(
                        "post {url} -> {status}: {}",
                        truncate(&text, 200)
                    )));
                }
                let text = resp.text().unwrap_or_default();
                parse_rpc_response(&text, id)
            }
            SseMode::Legacy { post_url, pending } => {
                let mut req = self
                    .client
                    .post(post_url)
                    .json(&body)
                    .timeout(self.timeout);
                for (k, v) in self.headers.iter() {
                    req = req.header(k, v);
                }
                let resp = req
                    .send()
                    .map_err(|e| McpError::Io(format!("post {post_url}: {e}")))?;
                if !resp.status().is_success() {
                    let status = resp.status();
                    let text = resp.text().unwrap_or_default();
                    return Err(McpError::Io(format!(
                        "post {post_url} -> {status}: {}",
                        truncate(&text, 200)
                    )));
                }

                let text = resp.text().unwrap_or_default();
                if !text.trim().is_empty() {
                    if let Ok(value) = serde_json::from_str::<Value>(&text) {
                        if let Some(err) = value.get("error") {
                            return Err(McpError::Rpc(err.to_string()));
                        }
                        return Ok(value.get("result").cloned().unwrap_or(value));
                    }
                }

                let (lock, cv) = &**pending;
                let mut guard =
                    lock.lock().map_err(|_| McpError::Io("lock poisoned".into()))?;
                let deadline = std::time::Instant::now() + self.timeout;
                while !guard.contains_key(&id) {
                    let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                    if remaining.is_zero() {
                        return Err(McpError::Timeout);
                    }
                    guard = cv
                        .wait_timeout(guard, remaining)
                        .map_err(|_| McpError::Io("condvar poisoned".into()))?
                        .0;
                }
                let response = guard.remove(&id).unwrap_or(Value::Null);
                if let Some(err) = response.get("error") {
                    return Err(McpError::Rpc(err.to_string()));
                }
                Ok(response.get("result").cloned().unwrap_or(Value::Null))
            }
        }
    }

    fn next_rpc(&self, method: &str, params: Value) -> Result<Value, McpError> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        self.rpc(id, method, params)
    }
}

impl McpTransport for SseMcpTransport {
    fn list_tools(&self) -> Result<Vec<McpToolInfo>, McpError> {
        let result = self.next_rpc("tools/list", json!({}))?;
        let tools = result
            .get("tools")
            .and_then(|t| t.as_array())
            .cloned()
            .unwrap_or_default();
        Ok(tools
            .into_iter()
            .filter_map(|t| {
                let name = t.get("name")?.as_str()?.to_string();
                let description = t
                    .get("description")
                    .and_then(|d| d.as_str())
                    .map(str::to_string);
                let input_schema = t.get("inputSchema").cloned().unwrap_or(json!({}));
                Some(McpToolInfo {
                    name,
                    description,
                    input_schema,
                })
            })
            .collect())
    }

    fn call_tool(&self, name: &str, arguments: Value) -> Result<Value, McpError> {
        let result = self.next_rpc(
            "tools/call",
            json!({ "name": name, "arguments": arguments }),
        )?;
        Ok(unwrap_tool_result(&result))
    }
}

fn sse_session_loop_from_response(
    resp: reqwest::blocking::Response,
    base: &str,
    post_url_tx: mpsc::Sender<Result<String, McpError>>,
    pending: Arc<(Mutex<HashMap<u64, Value>>, Condvar)>,
) -> Result<(), McpError> {
    let mut reader = BufReader::new(resp);
    let mut line = String::new();
    let mut event = String::new();
    let mut data = String::new();
    let mut post_url_sent = false;

    loop {
        line.clear();
        let n = reader
            .read_line(&mut line)
            .map_err(|e| McpError::Io(format!("sse read: {e}")))?;
        if n == 0 {
            break;
        }
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            if data.is_empty() {
                event.clear();
                continue;
            }
            dispatch_sse_event(&event, &data, base, &mut post_url_sent, &post_url_tx, &pending)?;
            data.clear();
            event.clear();
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("event:") {
            event = rest.trim().to_string();
        } else if let Some(rest) = trimmed.strip_prefix("data:") {
            data = rest.trim().to_string();
        }
    }

    if !post_url_sent {
        let _ = post_url_tx.send(Err(McpError::Io("sse closed before endpoint".into())));
    }
    Ok(())
}

fn dispatch_sse_event(
    event: &str,
    data: &str,
    base: &str,
    post_url_sent: &mut bool,
    post_url_tx: &mpsc::Sender<Result<String, McpError>>,
    pending: &Arc<(Mutex<HashMap<u64, Value>>, Condvar)>,
) -> Result<(), McpError> {
    if event == "endpoint" || (!*post_url_sent && (data.contains("/message") || data.starts_with("http")))
    {
        if !*post_url_sent {
            let url = resolve_post_url(base, data);
            let _ = post_url_tx.send(Ok(url));
            *post_url_sent = true;
        }
        return Ok(());
    }

    if event == "message" || looks_like_json_rpc(data) {
        if let Ok(value) = serde_json::from_str::<Value>(data) {
            if let Some(id) = value.get("id").and_then(|i| i.as_u64()) {
                let (lock, cv) = &**pending;
                if let Ok(mut guard) = lock.lock() {
                    guard.insert(id, value);
                    cv.notify_all();
                }
            }
        }
    }
    Ok(())
}

fn parse_rpc_response(text: &str, id: u64) -> Result<Value, McpError> {
    let trimmed = text.trim();
    if trimmed.starts_with('{') {
        if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
            if value.get("jsonrpc").is_some() {
                return extract_rpc_result(value, id);
            }
        }
    }

    for data in sse_data_lines(text) {
        if let Ok(value) = serde_json::from_str::<Value>(&data) {
            if value.get("id").and_then(|i| i.as_u64()) == Some(id) {
                return extract_rpc_result(value, id);
            }
        }
    }

    Err(McpError::Io(format!(
        "no rpc response for id {id}: {}",
        truncate(trimmed, 200)
    )))
}

fn extract_rpc_result(value: Value, id: u64) -> Result<Value, McpError> {
    if value.get("id").and_then(|i| i.as_u64()) != Some(id) {
        return Err(McpError::Io(format!("unexpected rpc id in response")));
    }
    if let Some(err) = value.get("error") {
        return Err(McpError::Rpc(err.to_string()));
    }
    Ok(value.get("result").cloned().unwrap_or(Value::Null))
}

fn sse_data_lines(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for line in text.lines() {
        if line.is_empty() {
            if !current.is_empty() {
                out.push(std::mem::take(&mut current));
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("data:") {
            if !current.is_empty() {
                current.push('\n');
            }
            current.push_str(rest.trim());
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

fn looks_like_json_rpc(data: &str) -> bool {
    let trimmed = data.trim();
    trimmed.starts_with('{') && trimmed.contains("\"jsonrpc\"")
}

fn base_url(url: &str) -> String {
    if let Some(idx) = url.find("://") {
        if let Some(slash) = url[idx + 3..].find('/') {
            return url[..idx + 3 + slash].to_string();
        }
        return url.to_string();
    }
    url.to_string()
}

fn resolve_post_url(base: &str, data: &str) -> String {
    if data.starts_with("http://") || data.starts_with("https://") {
        return data.to_string();
    }
    if data.starts_with('/') {
        return format!("{base}{data}");
    }
    format!("{base}/{data}")
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars().take(max).collect::<String>() + "…"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_post_url_relative() {
        assert_eq!(
            resolve_post_url("http://127.0.0.1:3200", "/message?sessionId=abc"),
            "http://127.0.0.1:3200/message?sessionId=abc"
        );
    }

    #[test]
    fn looks_like_json_rpc_detects_response() {
        assert!(looks_like_json_rpc(
            r#"{"jsonrpc":"2.0","id":1,"result":{}}"#
        ));
    }

    #[test]
    fn parse_rpc_response_from_sse() {
        let body = "event: message\ndata: {\"result\":{\"ok\":true},\"jsonrpc\":\"2.0\",\"id\":2}\n\n";
        let result = parse_rpc_response(body, 2).unwrap();
        assert_eq!(result, json!({"ok": true}));
    }

    #[test]
    fn parse_rpc_response_from_json() {
        let body = r#"{"jsonrpc":"2.0","id":1,"result":{"tools":[]}}"#;
        let result = parse_rpc_response(body, 1).unwrap();
        assert_eq!(result, json!({"tools": []}));
    }
}
