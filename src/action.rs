use std::fmt;

use serde_json::Value;

use crate::context_metrics::ContextUsage;

/// 1回のツール呼び出し（最少行動単位）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Action {
    pub invoke_id: u64,
    pub tool: String,
    pub args: Value,
}

impl Action {
    pub fn new(invoke_id: u64, tool: impl Into<String>, args: Value) -> Self {
        Self {
            invoke_id,
            tool: tool.into(),
            args,
        }
    }
}

/// Observation をプロンプトに載せるときの文字上限（巨大ファイル読みで文脈を壊さない）。
pub const MAX_OBSERVATION_CHARS: usize = 24_000;

/// ツール実行の結果（Observation）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation {
    pub invoke_id: u64,
    pub ok: bool,
    pub output: String,
}

impl Observation {
    pub fn success(invoke_id: u64, output: impl Into<String>) -> Self {
        Self {
            invoke_id,
            ok: true,
            output: truncate_observation(output.into()),
        }
    }

    pub fn failure(invoke_id: u64, output: impl Into<String>) -> Self {
        Self {
            invoke_id,
            ok: false,
            output: truncate_observation(output.into()),
        }
    }
}

fn truncate_observation(s: String) -> String {
    let count = s.chars().count();
    if count <= MAX_OBSERVATION_CHARS {
        return s;
    }
    let keep = MAX_OBSERVATION_CHARS.saturating_sub(96);
    let head: String = s.chars().take(keep).collect();
    format!("{head}\n\n…[truncated: {count} chars total, showing first {keep}]")
}

/// エージェントが1ステップで返す判断。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentStep {
    /// 内部推論（環境への副作用なし）。
    Thought(String),
    /// ツール呼び出し。
    Action(Action),
    /// ユーザーへの最終応答。
    Answer(String),
    /// 計画層専用: 読み取り専用メモリ検索（`MemoryBridge::search`）。実行層では無視される。
    Recall(String),
}

/// 1ターンの途中経過（Thought / Observation の蓄積）。
#[derive(Debug, Default)]
pub struct TurnTrace {
    pub thoughts: Vec<String>,
    pub actions: Vec<Action>,
    pub observations: Vec<Observation>,
    /// ReAct 各ステップの LLM プロンプト／出力計測。
    pub context_usages: Vec<ContextUsage>,
}

impl TurnTrace {
    pub fn push_thought(&mut self, thought: String) {
        self.thoughts.push(thought);
    }

    pub fn push_action(&mut self, action: Action) {
        self.actions.push(action);
    }

    pub fn push_observation(&mut self, observation: Observation) {
        self.observations.push(observation);
    }

    pub fn push_context_usage(&mut self, usage: ContextUsage) {
        self.context_usages.push(usage);
    }
}

impl fmt::Display for TurnTrace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, thought) in self.thoughts.iter().enumerate() {
            writeln!(f, "[thought {i}] {thought}")?;
        }
        for action in &self.actions {
            writeln!(f, "[action {}] {} {:?}", action.invoke_id, action.tool, action.args)?;
        }
        for obs in &self.observations {
            let status = if obs.ok { "ok" } else { "err" };
            writeln!(f, "[observation {}] {status}: {}", obs.invoke_id, obs.output)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observation_truncates_huge_output() {
        let huge = "あ".repeat(MAX_OBSERVATION_CHARS + 500);
        let obs = Observation::success(1, huge);
        assert!(obs.ok);
        assert!(obs.output.contains("truncated:"));
        assert!(obs.output.chars().count() <= MAX_OBSERVATION_CHARS + 40);
    }

    #[test]
    fn observation_keeps_small_output() {
        let obs = Observation::success(1, "hello");
        assert_eq!(obs.output, "hello");
    }
}
