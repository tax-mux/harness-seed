//! mempalace MCP サーバ（stdio / 改行区切り JSON-RPC）クライアント。
//!
//! Cursor の `mcp.json` と同じ起動:
//! `python -m mempalace.mcp_server`

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;
use std::time::Instant;

use serde_json::{json, Value};

use crate::config::MempalaceConfig;
use crate::parse::unwrap_tool_result;
use crate::types::MempalaceError;
use crate::MempalaceTransport;

/// 長寿命の MCP stdio 子プロセス。
pub struct McpStdioTransport {
    inner: Mutex<McpSession>,
    timeout: std::time::Duration,
}

struct McpSession {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl McpStdioTransport {
    pub fn spawn(config: &MempalaceConfig) -> Result<Self, MempalaceError> {
        let command = config.mcp_command();
        let args = config.mcp_args();
        // Windows 既定コードページだと日本語 diary が壊れ、chromadb upsert が
        // Internal tool error / TextInputSequence になる。UTF-8 を強制する。
        let mut child = Command::new(&command)
            .args(&args)
            .env("PYTHONIOENCODING", "utf-8")
            .env("PYTHONUTF8", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| {
                MempalaceError::Http(format!(
                    "failed to spawn MCP server `{command} {}`: {e}",
                    args.join(" ")
                ))
            })?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| MempalaceError::Http("MCP stdin missing".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| MempalaceError::Http("MCP stdout missing".into()))?;

        let mut session = McpSession {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
        };
        let timeout = config.timeout();
        session.initialize(timeout)?;

        Ok(Self {
            inner: Mutex::new(session),
            timeout,
        })
    }
}

impl McpSession {
    fn initialize(&mut self, timeout: std::time::Duration) -> Result<(), MempalaceError> {
        let id = self.next_id;
        self.next_id += 1;
        let req = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {
                    "name": "harness-seed",
                    "version": "0.1.0"
                }
            }
        });
        self.write_message(&req)?;
        let _ = self.read_message(timeout)?;

        // notification — no response
        let note = json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {}
        });
        self.write_message(&note)?;
        Ok(())
    }

    fn write_message(&mut self, value: &Value) -> Result<(), MempalaceError> {
        let line = serde_json::to_string(value)
            .map_err(|e| MempalaceError::Parse(e.to_string()))?;
        self.stdin
            .write_all(line.as_bytes())
            .and_then(|_| self.stdin.write_all(b"\n"))
            .and_then(|_| self.stdin.flush())
            .map_err(|e| MempalaceError::Http(format!("MCP write: {e}")))
    }

    fn read_message(&mut self, timeout: std::time::Duration) -> Result<Value, MempalaceError> {
        let start = Instant::now();
        let mut line = String::new();
        loop {
            if start.elapsed() > timeout {
                return Err(MempalaceError::Http("MCP read timeout".into()));
            }
            line.clear();
            // Blocking read — timeout is best-effort (process-level timeout_secs on config).
            let n = self
                .stdout
                .read_line(&mut line)
                .map_err(|e| MempalaceError::Http(format!("MCP read: {e}")))?;
            if n == 0 {
                return Err(MempalaceError::Http("MCP server closed stdout".into()));
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let value: Value = serde_json::from_str(trimmed)
                .map_err(|e| MempalaceError::Parse(format!("MCP JSON: {e}: {trimmed}")))?;
            return Ok(value);
        }
    }

    fn call_tool(
        &mut self,
        name: &str,
        arguments: Value,
        timeout: std::time::Duration,
    ) -> Result<Value, MempalaceError> {
        let id = self.next_id;
        self.next_id += 1;
        let req = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": {
                "name": name,
                "arguments": arguments,
            }
        });
        self.write_message(&req)?;
        let response = self.read_message(timeout)?;
        if let Some(err) = response.get("error") {
            return Err(MempalaceError::Backend(err.to_string()));
        }
        Ok(unwrap_tool_result(&response))
    }
}

impl MempalaceTransport for McpStdioTransport {
    fn call_tool(&self, name: &str, arguments: Value) -> Result<Value, MempalaceError> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| MempalaceError::Http("MCP session lock poisoned".into()))?;
        guard.call_tool(name, arguments, self.timeout)
    }
}

impl Drop for McpStdioTransport {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.inner.lock() {
            let _ = guard.child.kill();
            let _ = guard.child.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{TOOL_SEARCH, MempalaceTransport};
    use serde_json::json;

    #[test]
    #[ignore = "requires python -m mempalace.mcp_server and a local palace"]
    fn mcp_stdio_live_search() {
        let cfg = MempalaceConfig::default();
        let transport = McpStdioTransport::spawn(&cfg).expect("spawn MCP");
        let value = transport
            .call_tool(
                TOOL_SEARCH,
                json!({"query": "harness", "limit": 2}),
            )
            .expect("search");
        let hits = crate::parse::parse_search_hits(&value);
        assert!(
            !hits.is_empty(),
            "expected search hits, got: {value}"
        );
        eprintln!("hit0: {} — {}", hits[0].title, &hits[0].body[..hits[0].body.len().min(80)]);
    }
}
