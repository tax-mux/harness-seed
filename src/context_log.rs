use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::action::TurnTrace;
use crate::context_metrics::{ContextUsage, TokenSource, TurnContextSummary};
use crate::react::TurnResult;

/// コンテキスト計測ログの既定パス（クレートルート相対）。
pub const DEFAULT_CONTEXT_LOG_REL: &str = "logs/context.jsonl";

/// 既定の JSON Lines ログパス（`CARGO_MANIFEST_DIR` 基準）。
pub fn default_log_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(DEFAULT_CONTEXT_LOG_REL)
}

/// 1 ターン分のコンテキスト計測ログ（JSON Lines）。
#[derive(Debug, Serialize)]
pub struct ContextLogEntry<'a> {
    pub timestamp: String,
    pub user_input: &'a str,
    pub steps_used: usize,
    pub answer_chars: usize,
    pub context: ContextLogSummary,
    pub steps: Vec<ContextLogStep>,
}

#[derive(Debug, Serialize)]
pub struct ContextLogSummary {
    pub llm_calls: usize,
    pub prompt_chars: usize,
    pub prompt_bytes: usize,
    pub prompt_tokens: u32,
    pub completion_chars: usize,
    pub completion_bytes: usize,
    pub completion_tokens: u32,
    pub total_tokens: u32,
    pub token_source: &'static str,
}

#[derive(Debug, Serialize)]
pub struct ContextLogStep {
    pub step: usize,
    /// API に送ったプロンプト全文。
    pub prompt: String,
    /// LLM の生出力全文。
    pub completion: String,
    pub prompt_chars: usize,
    pub prompt_bytes: usize,
    pub prompt_tokens: u32,
    pub completion_chars: usize,
    pub completion_bytes: usize,
    pub completion_tokens: u32,
    pub token_source: &'static str,
}

impl ContextLogSummary {
    fn from_summary(s: &TurnContextSummary) -> Self {
        Self {
            llm_calls: s.llm_calls,
            prompt_chars: s.prompt.chars,
            prompt_bytes: s.prompt.bytes,
            prompt_tokens: s.prompt_tokens,
            completion_chars: s.completion.chars,
            completion_bytes: s.completion.bytes,
            completion_tokens: s.completion_tokens,
            total_tokens: s.prompt_tokens + s.completion_tokens,
            token_source: token_source_str(s.token_source),
        }
    }
}

fn token_source_str(s: TokenSource) -> &'static str {
    match s {
        TokenSource::Api => "api",
        TokenSource::Estimated => "estimated",
    }
}

fn steps_from_trace(trace: &TurnTrace) -> Vec<ContextLogStep> {
    trace
        .context_usages
        .iter()
        .enumerate()
        .map(|(i, u)| step_from_usage(i + 1, u))
        .collect()
}

fn step_from_usage(step: usize, u: &ContextUsage) -> ContextLogStep {
    ContextLogStep {
        step,
        prompt: u.prompt_body.clone(),
        completion: u.completion_body.clone(),
        prompt_chars: u.prompt.chars,
        prompt_bytes: u.prompt.bytes,
        prompt_tokens: u.prompt_tokens_effective(),
        completion_chars: u.completion.chars,
        completion_bytes: u.completion.bytes,
        completion_tokens: u.completion_tokens_effective(),
        token_source: token_source_str(u.token_source),
    }
}

/// コンテキスト計測を JSON Lines ファイルへ追記する。
pub struct ContextLogWriter {
    path: PathBuf,
}

impl ContextLogWriter {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn append_turn(&self, user_input: &str, result: &TurnResult) -> io::Result<()> {
        if result.context.is_empty() {
            return Ok(());
        }

        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }

        let entry = ContextLogEntry {
            timestamp: chrono_lite_timestamp(),
            user_input,
            steps_used: result.steps_used,
            answer_chars: result.answer.chars().count(),
            context: ContextLogSummary::from_summary(&result.context),
            steps: steps_from_trace(&result.trace),
        };

        let line = serde_json::to_string(&entry).map_err(|e| {
            io::Error::new(io::ErrorKind::InvalidData, e.to_string())
        })?;

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;

        writeln!(file, "{line}")?;
        file.flush()?;
        Ok(())
    }
}

/// ISO 8601 風タイムスタンプ（追加依存なし）。
fn chrono_lite_timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}.{:03}Z", dur.as_secs(), dur.subsec_millis())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LlmBrain, MockLlmConnector, ReActLoop};

    #[test]
    fn appends_json_line_to_file() {
        let dir = std::env::temp_dir().join(format!("harness_seed_log_{}", std::process::id()));
        let path = dir.join("context.jsonl");
        let _ = fs::remove_file(&path);

        let mut react = ReActLoop::with_defaults(LlmBrain::new(MockLlmConnector));
        let result = react.run_turn("hello").unwrap();
        let writer = ContextLogWriter::new(&path);
        writer.append_turn("hello", &result).unwrap();

        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"user_input\":\"hello\""));
        assert!(text.contains("\"prompt\":\"system:"));
        assert!(text.contains("\"llm_calls\":3"));
        let _ = fs::remove_dir_all(&dir);
    }
}
