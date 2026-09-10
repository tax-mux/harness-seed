//! 単一 MCP ツールの `Tool` 実装。

use std::sync::Arc;

use serde_json::Value;

use crate::action::Observation;
use crate::tool::{Tool, ToolContext};

use super::config::spec_from_schema;
use super::result::format_tool_output;
use super::transport::McpTransport;

pub struct McpTool {
    pub qualified_name: String,
    spec: String,
    remote_name: String,
    transport: Arc<dyn McpTransport>,
}

impl McpTool {
    pub fn new(
        qualified_name: String,
        remote_name: String,
        description: Option<&str>,
        input_schema: &Value,
        transport: Arc<dyn McpTransport>,
    ) -> Self {
        let spec = spec_from_schema(input_schema, description);
        Self {
            qualified_name,
            spec,
            remote_name,
            transport,
        }
    }
}

impl Tool for McpTool {
    fn name(&self) -> &str {
        &self.qualified_name
    }

    fn spec(&self) -> &str {
        &self.spec
    }

    fn execute(&self, invoke_id: u64, args: &Value, _ctx: &ToolContext) -> Observation {
        match self.transport.call_tool(&self.remote_name, args.clone()) {
            Ok(value) => Observation::success(invoke_id, format_tool_output(&value)),
            Err(err) => Observation::failure(invoke_id, err.to_string()),
        }
    }
}
