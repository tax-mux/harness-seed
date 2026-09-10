//! プラグイン可能なツール trait。

use serde_json::Value;

use crate::action::Observation;
use crate::brave_search::BraveSearchConfig;
use crate::runtime::RuntimeEnvironment;

/// 1 回のツール呼び出しで共有する実行コンテキスト。
#[derive(Debug, Clone)]
pub struct ToolContext {
    pub env: RuntimeEnvironment,
    pub brave_search: Option<BraveSearchConfig>,
}

impl ToolContext {
    pub fn new(env: RuntimeEnvironment, brave_search: Option<BraveSearchConfig>) -> Self {
        Self {
            env,
            brave_search,
        }
    }
}

/// ReAct から呼び出されるツール（in-process プラグイン）。
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    /// LLM 向け引数説明（`Tool catalog` 1 行分）。
    fn spec(&self) -> &str;
    fn execute(&self, invoke_id: u64, args: &Value, ctx: &ToolContext) -> Observation;
}
