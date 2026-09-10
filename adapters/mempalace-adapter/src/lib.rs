//! mempalace アダプタ（search / diary_read / diary_write）。
//!
//! 既定は Cursor `mcp.json` と同じ **MCP stdio**:
//! `python -m mempalace.mcp_server`（改行区切り JSON-RPC）。
//!
//! harness-seed 側の薄いラッパ: `harness_seed::memory::MempalaceBridge`。
//!
//! ツール名:
//! - `mempalace_search`
//! - `mempalace_diary_read`
//! - `mempalace_diary_write`

mod client;
mod config;
mod mcp_stdio;
mod parse;
mod types;

pub use client::{HttpTransport, MempalaceClient, MempalaceTransport};
pub use config::{MempalaceConfig, MempalaceProtocol};
pub use mcp_stdio::McpStdioTransport;
pub use types::{DiaryReadEntry, MempalaceError, SearchHit};

/// MCP ツール名（Cursor mempalace サーバと同一）。
pub const TOOL_SEARCH: &str = "mempalace_search";
pub const TOOL_DIARY_READ: &str = "mempalace_diary_read";
pub const TOOL_DIARY_WRITE: &str = "mempalace_diary_write";
pub const TOOL_LIST_WINGS: &str = "mempalace_list_wings";
pub const TOOL_ADD_DRAWER: &str = "mempalace_add_drawer";
