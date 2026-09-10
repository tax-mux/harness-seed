//! MCP サーバー接続と ReAct ツール登録。

use std::sync::Arc;

use crate::config::McpToolsSection;
use crate::tool::Tool;

use super::config::{load_mcp_servers, qualified_tool_name, timeout_for};
use super::tool::McpTool;
use super::transport::connect_server;

pub fn load_mcp_tools(section: &McpToolsSection) -> Vec<Box<dyn Tool>> {
    let servers = load_mcp_servers(section);
    if servers.is_empty() {
        return vec![];
    }
    let timeout = timeout_for(section);
    let mut tools: Vec<Box<dyn Tool>> = Vec::new();
    for server in servers {
        match connect_server(&server, timeout) {
            Ok(transport) => {
                let transport: Arc<dyn super::transport::McpTransport> = Arc::from(transport);
                match transport.list_tools() {
                    Ok(list) => {
                        eprintln!(
                            "[mcp] {}: {} tool(s)",
                            server.name,
                            list.len()
                        );
                        for info in list {
                            let qualified = qualified_tool_name(&server.name, &info.name);
                            tools.push(Box::new(McpTool::new(
                                qualified,
                                info.name,
                                info.description.as_deref(),
                                &info.input_schema,
                                Arc::clone(&transport),
                            )));
                        }
                    }
                    Err(err) => {
                        eprintln!("[mcp] {} tools/list: {err}", server.name);
                    }
                }
            }
            Err(err) => {
                eprintln!("[mcp] {} connect: {err}", server.name);
            }
        }
    }
    if !tools.is_empty() {
        eprintln!("[mcp] registered {} ReAct tool(s)", tools.len());
    }
    tools
}
