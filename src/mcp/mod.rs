//! MCP クライアント — `mcp.json` のサーバーを ReAct ツールとして登録する。

mod config;
mod loader;
mod result;
mod sse;
mod stdio;
mod tool;
mod transport;

pub use config::{expand_path, load_mcp_servers, McpServerConfig};
pub use loader::load_mcp_tools;
pub use tool::McpTool;
