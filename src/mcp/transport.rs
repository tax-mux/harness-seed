//! MCP トランスポート trait。

use serde_json::Value;

use super::config::McpServerConfig;

#[derive(Debug)]
pub enum McpError {
    Config(String),
    Io(String),
    Rpc(String),
    Timeout,
}

impl std::fmt::Display for McpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Config(s) => write!(f, "mcp config: {s}"),
            Self::Io(s) => write!(f, "mcp io: {s}"),
            Self::Rpc(s) => write!(f, "mcp rpc: {s}"),
            Self::Timeout => write!(f, "mcp timeout"),
        }
    }
}

impl std::error::Error for McpError {}

#[derive(Debug, Clone)]
pub struct McpToolInfo {
    pub name: String,
    pub description: Option<String>,
    pub input_schema: Value,
}

pub trait McpTransport: Send + Sync {
    fn list_tools(&self) -> Result<Vec<McpToolInfo>, McpError>;
    fn call_tool(&self, name: &str, arguments: Value) -> Result<Value, McpError>;
}

pub fn connect_server(
    config: &McpServerConfig,
    timeout: std::time::Duration,
) -> Result<Box<dyn McpTransport>, McpError> {
    if let Some(command) = &config.command {
        return Ok(Box::new(super::stdio::StdioMcpTransport::spawn(
            command,
            &config.args,
            &config.env,
            timeout,
        )?));
    }
    if let Some(url) = &config.url {
        return Ok(Box::new(super::sse::SseMcpTransport::connect(
            url,
            &config.headers,
            timeout,
        )?));
    }
    Err(McpError::Config("no command or url".into()))
}
