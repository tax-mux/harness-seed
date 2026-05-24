//! ツール名 → 実装のレジストリ。

use std::collections::HashMap;
use std::fmt;

use serde_json::Value;

use crate::action::Observation;

use super::traits::{Tool, ToolContext};

/// 登録済みツールの実行・カタログ生成。
#[derive(Default)]
pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn Tool>>,
}

impl fmt::Debug for ToolRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ToolRegistry")
            .field("tools", &self.names())
            .finish()
    }
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, tool: Box<dyn Tool>) {
        let name = tool.name().to_string();
        self.tools.insert(name, tool);
    }

    pub fn contains(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }

    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.tools.keys().cloned().collect();
        names.sort();
        names
    }

    pub fn execute(
        &self,
        name: &str,
        invoke_id: u64,
        args: &Value,
        ctx: &ToolContext,
    ) -> Observation {
        let Some(tool) = self.tools.get(name) else {
            return Observation::failure(invoke_id, format!("unknown tool: {name}"));
        };
        tool.execute(invoke_id, args, ctx)
    }

    /// LLM system に載せる `Tool catalog` ブロック全文。
    pub fn format_catalog(&self) -> String {
        self.format_catalog_filtered(None)
    }

    /// `policy` があれば allow のみ載せる（`None` は全ツール）。
    pub fn format_catalog_filtered(
        &self,
        policy: Option<&crate::tasks::SubtaskToolPolicy>,
    ) -> String {
        let mut out = String::from("Tool catalog:\n");
        for name in self.names() {
            if let Some(p) = policy {
                if !p.is_allowed(&name) {
                    continue;
                }
            }
            if let Some(tool) = self.tools.get(&name) {
                out.push_str(&format!("- {}: {}\n", tool.name(), tool.spec()));
            }
        }
        out
    }
}
