use crate::context_metrics::ContextUsage;

/// LLM 1 回分の応答とコンテキスト計測。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionResult {
    pub content: String,
    pub usage: ContextUsage,
}
