//! MCP stdio トランスポート（改行区切り JSON-RPC）。

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;

use serde_json::{json, Value};

use super::config::McpServerConfig;
use super::result::unwrap_tool_result;
use super::transport::{McpError, McpToolInfo, McpTransport};

pub struct StdioMcpTransport {
    inner: Mutex<Session>,
    timeout: std::time::Duration,
}

struct Session {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl StdioMcpTransport {
    pub fn spawn(
        command: &str,
        args: &[String],
        env: &BTreeMap<String, String>,
        timeout: std::time::Duration,
    ) -> Result<Self, McpError> {
        let mut cmd = Command::new(command);
        cmd.args(args)
            .env("PYTHONIOENCODING", "utf-8")
            .env("PYTHONUTF8", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        for (k, v) in env {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().map_err(|e| {
            McpError::Io(format!("spawn `{command} {}`: {e}", args.join(" ")))
        })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| McpError::Io("stdin missing".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| McpError::Io("stdout missing".into()))?;
        let mut session = Session {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
        };
        session.initialize(timeout)?;
        Ok(Self {
            inner: Mutex::new(session),
            timeout,
        })
    }
}

impl Session {
    fn initialize(&mut self, timeout: std::time::Duration) -> Result<(), McpError> {
        let id = self.next_id;
        self.next_id += 1;
        self.request(
            id,
            "initialize",
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "harness-seed", "version": "0.1.0" }
            }),
            timeout,
        )?;
        self.write_message(&json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {}
        }))?;
        Ok(())
    }

    fn request(
        &mut self,
        id: u64,
        method: &str,
        params: Value,
        timeout: std::time::Duration,
    ) -> Result<Value, McpError> {
        self.write_message(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }))?;
        let response = self.read_message(timeout)?;
        if let Some(err) = response.get("error") {
            return Err(McpError::Rpc(err.to_string()));
        }
        Ok(response.get("result").cloned().unwrap_or(Value::Null))
    }

    fn write_message(&mut self, value: &Value) -> Result<(), McpError> {
        let line =
            serde_json::to_string(value).map_err(|e| McpError::Io(e.to_string()))?;
        self.stdin
            .write_all(line.as_bytes())
            .and_then(|_| self.stdin.write_all(b"\n"))
            .and_then(|_| self.stdin.flush())
            .map_err(|e| McpError::Io(format!("write: {e}")))?;
        Ok(())
    }

    fn read_message(&mut self, timeout: std::time::Duration) -> Result<Value, McpError> {
        let start = std::time::Instant::now();
        let mut line = String::new();
        loop {
            if start.elapsed() > timeout {
                return Err(McpError::Timeout);
            }
            line.clear();
            let n = self
                .stdout
                .read_line(&mut line)
                .map_err(|e| McpError::Io(format!("read: {e}")))?;
            if n == 0 {
                return Err(McpError::Io("server closed stdout".into()));
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let value: Value = serde_json::from_str(trimmed)
                .map_err(|e| McpError::Io(format!("json: {e}: {trimmed}")))?;
            return Ok(value);
        }
    }

    fn list_tools(&mut self, timeout: std::time::Duration) -> Result<Vec<McpToolInfo>, McpError> {
        let id = self.next_id;
        self.next_id += 1;
        let result = self.request(id, "tools/list", json!({}), timeout)?;
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
                let input_schema = t
                    .get("inputSchema")
                    .cloned()
                    .unwrap_or(json!({}));
                Some(McpToolInfo {
                    name,
                    description,
                    input_schema,
                })
            })
            .collect())
    }

    fn call_tool(
        &mut self,
        name: &str,
        arguments: Value,
        timeout: std::time::Duration,
    ) -> Result<Value, McpError> {
        let id = self.next_id;
        self.next_id += 1;
        let result = self.request(
            id,
            "tools/call",
            json!({ "name": name, "arguments": arguments }),
            timeout,
        )?;
        Ok(unwrap_tool_result(&result))
    }
}

impl McpTransport for StdioMcpTransport {
    fn list_tools(&self) -> Result<Vec<McpToolInfo>, McpError> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| McpError::Io("lock poisoned".into()))?;
        guard.list_tools(self.timeout)
    }

    fn call_tool(&self, name: &str, arguments: Value) -> Result<Value, McpError> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| McpError::Io("lock poisoned".into()))?;
        guard.call_tool(name, arguments, self.timeout)
    }
}

impl Drop for StdioMcpTransport {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.inner.lock() {
            let _ = guard.child.kill();
            let _ = guard.child.wait();
        }
    }
}

#[allow(dead_code)]
pub fn spawn_from_config(config: &McpServerConfig, timeout: std::time::Duration) -> Result<StdioMcpTransport, McpError> {
    let command = config
        .command
        .as_ref()
        .ok_or_else(|| McpError::Config("stdio server missing command".into()))?;
    StdioMcpTransport::spawn(command, &config.args, &config.env, timeout)
}
