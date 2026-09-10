use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::action::TurnTrace;
use crate::config::LogRotationConfig;
use crate::context_metrics::{ContextUsage, TokenSource, TurnContextSummary};
use crate::react::TurnResult;

/// コンテキスト計測ログの既定相対パス（[`crate::config::user_config_dir`] 基準）。
pub const DEFAULT_CONTEXT_LOG_REL: &str = "logs/events.jsonl";

/// スキーマ版（events 行の `v`）。
pub const CONTEXT_LOG_SCHEMA_VERSION: u32 = 1;

/// events 行に載せる preview の最大文字数。
pub const PREVIEW_CHARS: usize = 240;

/// 既定の JSON Lines ログパス（`…/harness-seed/logs/events.jsonl`）。
pub fn default_log_path() -> PathBuf {
    crate::config::user_config_dir().join(DEFAULT_CONTEXT_LOG_REL)
}

static TURN_SEQ: AtomicU64 = AtomicU64::new(1);

fn next_turn_id(run_id: &str) -> String {
    let n = TURN_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("{run_id}-t{n}")
}

fn new_run_id() -> String {
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("r{:x}-{}", dur.as_secs(), std::process::id())
}

/// ISO 8601 UTC（ミリ秒・`Z`）。追加依存なし。
pub fn iso8601_timestamp_now() -> String {
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    iso8601_from_unix(dur.as_secs(), dur.subsec_millis())
}

fn iso8601_from_unix(secs: u64, millis: u32) -> String {
    let (y, m, d, hh, mm, ss) = civil_utc_from_unix(secs);
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}.{millis:03}Z")
}

/// Howard Hinnant の civil-from-days（UTC）。
fn civil_utc_from_unix(secs: u64) -> (i32, u32, u32, u32, u32, u32) {
    let days = (secs / 86_400) as i64;
    let tod = (secs % 86_400) as u32;
    let hh = tod / 3600;
    let mm = (tod % 3600) / 60;
    let ss = tod % 60;

    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y } as i32;
    (y, m, d, hh, mm, ss)
}

pub fn preview_text(s: &str, max_chars: usize) -> String {
    let count = s.chars().count();
    if count <= max_chars {
        return s.to_string();
    }
    let truncated: String = s.chars().take(max_chars).collect();
    format!("{truncated}...")
}

/// prompt 本文からフェーズを推定する。
pub fn infer_phase(prompt: &str) -> &'static str {
    if prompt.contains("You are a planning agent") {
        return "plan";
    }
    if prompt.contains("claim-falsification") {
        return "falsify";
    }
    if prompt.contains("Replan directive") {
        return "replan";
    }
    if prompt.contains("Evidence grounding") || prompt.contains("evidence-deepening") {
        return "execute";
    }
    if prompt.contains("audit") && prompt.contains("Supported") {
        return "audit";
    }
    "react"
}

/// completion JSON から step 種別と tool を抜く。
pub fn infer_step_meta(completion: &str) -> (String, Option<String>) {
    let trimmed = completion.trim();
    let jsonish = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .map(|s| s.trim_end_matches('`').trim())
        .unwrap_or(trimmed);

    if let Ok(v) = serde_json::from_str::<serde_json::Value>(jsonish) {
        let step_kind = v
            .get("step")
            .and_then(|x| x.as_str())
            .unwrap_or("unknown")
            .to_string();
        let tool = v
            .get("tool")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string());
        return (step_kind, tool);
    }

    // 緩いフォールバック
    let step_kind = if completion.contains("\"step\":\"action\"")
        || completion.contains("\"step\": \"action\"")
    {
        "action"
    } else if completion.contains("\"step\":\"answer\"")
        || completion.contains("\"step\": \"answer\"")
    {
        "answer"
    } else if completion.contains("\"step\":\"thought\"")
        || completion.contains("\"step\": \"thought\"")
    {
        "thought"
    } else {
        "unknown"
    };
    let tool = extract_json_string_field(completion, "tool");
    (step_kind.to_string(), tool)
}

fn extract_json_string_field(s: &str, key: &str) -> Option<String> {
    let patterns = [
        format!("\"{key}\":\""),
        format!("\"{key}\": \""),
    ];
    for pat in patterns {
        if let Some(i) = s.find(&pat) {
            let rest = &s[i + pat.len()..];
            let end = rest.find('"')?;
            return Some(rest[..end].to_string());
        }
    }
    None
}

fn sha256_hex(data: &str) -> String {
    let digest = Sha256::digest(data.as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

fn blobs_dir_for_log(log_path: &Path) -> PathBuf {
    log_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("blobs")
}

fn write_blob(blobs_dir: &Path, content: &str) -> io::Result<String> {
    fs::create_dir_all(blobs_dir)?;
    let hash = sha256_hex(content);
    let path = blobs_dir.join(&hash);
    if !path.exists() {
        fs::write(&path, content)?;
    }
    Ok(format!("sha256:{hash}"))
}

/// 1 ターン分のコンテキスト計測ログ（JSON Lines, schema v1）。
#[derive(Debug, Serialize)]
pub struct ContextLogEntry<'a> {
    pub v: u32,
    pub ts: String,
    pub run_id: &'a str,
    pub turn_id: String,
    pub kind: &'static str,
    pub user_input: &'a str,
    pub steps_used: usize,
    pub answer_chars: usize,
    pub answer_preview: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub thoughts: Vec<String>,
    pub phases: Vec<&'static str>,
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
    pub phase: &'static str,
    pub step_kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    pub prompt_preview: String,
    pub completion_preview: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion_ref: Option<String>,
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

fn steps_from_trace(trace: &TurnTrace, blobs_dir: &Path) -> io::Result<Vec<ContextLogStep>> {
    trace
        .context_usages
        .iter()
        .enumerate()
        .map(|(i, u)| step_from_usage(i + 1, u, blobs_dir))
        .collect()
}

fn step_from_usage(step: usize, u: &ContextUsage, blobs_dir: &Path) -> io::Result<ContextLogStep> {
    let phase = infer_phase(&u.prompt_body);
    let (step_kind, tool) = infer_step_meta(&u.completion_body);
    let prompt_ref = if u.prompt_body.chars().count() > PREVIEW_CHARS {
        Some(write_blob(blobs_dir, &u.prompt_body)?)
    } else {
        None
    };
    let completion_ref = if u.completion_body.chars().count() > PREVIEW_CHARS {
        Some(write_blob(blobs_dir, &u.completion_body)?)
    } else {
        None
    };
    Ok(ContextLogStep {
        step,
        phase,
        step_kind,
        tool,
        prompt_preview: preview_text(&u.prompt_body, PREVIEW_CHARS),
        completion_preview: preview_text(&u.completion_body, PREVIEW_CHARS),
        prompt_ref,
        completion_ref,
        prompt_chars: u.prompt.chars,
        prompt_bytes: u.prompt.bytes,
        prompt_tokens: u.prompt_tokens_effective(),
        completion_chars: u.completion.chars,
        completion_bytes: u.completion.bytes,
        completion_tokens: u.completion_tokens_effective(),
        token_source: token_source_str(u.token_source),
    })
}

/// 既定コンソール用の1行ターンサマリ（巨大 prompt を含まない）。
pub fn format_turn_console_summary(user_input: &str, result: &TurnResult) -> String {
    let phases: Vec<&str> = result
        .trace
        .context_usages
        .iter()
        .map(|u| infer_phase(&u.prompt_body))
        .collect();
    let phase_flow = if phases.is_empty() {
        "-".to_string()
    } else {
        let mut out = Vec::new();
        for p in phases {
            if out.last().copied() != Some(p) {
                out.push(p);
            }
        }
        out.join("->")
    };
    let input_preview = preview_text(user_input, 40);
    let tokens = result.context.prompt_tokens + result.context.completion_tokens;
    format!(
        "> turn  steps={}  tokens={}  phase={phase_flow}  ok  | {input_preview}",
        result.steps_used, tokens
    )
}

/// ステップ短行（tool 名とトークン程度）。
pub fn format_step_console_lines(result: &TurnResult) -> Vec<String> {
    result
        .trace
        .context_usages
        .iter()
        .enumerate()
        .map(|(i, u)| {
            let phase = infer_phase(&u.prompt_body);
            let (kind, tool) = infer_step_meta(&u.completion_body);
            let tool_bit = tool
                .as_deref()
                .map(|t| format!(":{t}"))
                .unwrap_or_default();
            format!(
                "  - #{} {phase}  {kind}{tool_bit}   {}->{}",
                i + 1,
                u.prompt_tokens_effective(),
                u.completion_tokens_effective()
            )
        })
        .collect()
}

fn backup_path(path: &Path, index: u32) -> PathBuf {
    PathBuf::from(format!("{}.{}", path.to_string_lossy(), index))
}

/// サイズ超過時に `path` → `path.1` → … と世代をずらす。
pub fn rotate_log_file(path: &Path, config: LogRotationConfig) -> io::Result<()> {
    if !config.enabled() {
        return Ok(());
    }
    let meta = match fs::metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    if meta.len() < config.max_bytes {
        return Ok(());
    }

    let backup_slots = config.max_files.saturating_sub(1);
    if backup_slots == 0 {
        return Ok(());
    }

    let oldest = backup_path(path, backup_slots);
    if oldest.exists() {
        fs::remove_file(&oldest)?;
    }
    for i in (1..backup_slots).rev() {
        let from = backup_path(path, i);
        if from.exists() {
            let to = backup_path(path, i + 1);
            fs::rename(&from, &to)?;
        }
    }
    if path.exists() {
        fs::rename(path, backup_path(path, 1))?;
    }
    Ok(())
}

/// コンテキスト計測を JSON Lines ファイルへ追記する。
pub struct ContextLogWriter {
    path: PathBuf,
    rotation: LogRotationConfig,
    run_id: String,
}

impl ContextLogWriter {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            rotation: LogRotationConfig::disabled(),
            run_id: new_run_id(),
        }
    }

    pub fn with_rotation(mut self, rotation: LogRotationConfig) -> Self {
        self.rotation = rotation;
        self
    }

    pub fn with_run_id(mut self, run_id: impl Into<String>) -> Self {
        self.run_id = run_id.into();
        self
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    pub fn append_turn(&self, user_input: &str, result: &TurnResult) -> io::Result<()> {
        if result.context.is_empty() {
            return Ok(());
        }

        rotate_log_file(&self.path, self.rotation)?;

        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }

        let blobs_dir = blobs_dir_for_log(&self.path);
        let steps = steps_from_trace(&result.trace, &blobs_dir)?;
        let phases: Vec<&'static str> = steps.iter().map(|s| s.phase).collect();

        let entry = ContextLogEntry {
            v: CONTEXT_LOG_SCHEMA_VERSION,
            ts: iso8601_timestamp_now(),
            run_id: &self.run_id,
            turn_id: next_turn_id(&self.run_id),
            kind: "turn.summary",
            user_input,
            steps_used: result.steps_used,
            answer_chars: result.answer.chars().count(),
            answer_preview: preview_text(&result.answer, PREVIEW_CHARS),
            thoughts: result.trace.thoughts.clone(),
            phases,
            context: ContextLogSummary::from_summary(&result.context),
            steps,
        };

        let line = serde_json::to_string(&entry)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;

        writeln!(file, "{line}")?;
        file.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LogRotationConfig;
    use crate::{LlmBrain, MockLlmConnector, ReActLoop};

    #[test]
    fn iso8601_looks_like_rfc3339() {
        let ts = iso8601_from_unix(1_788_225_842, 866);
        assert!(ts.ends_with('Z'), "{ts}");
        assert!(ts.contains('T'), "{ts}");
        assert_eq!(ts.len(), 24, "{ts}");
        assert!(!ts.chars().all(|c| c.is_ascii_digit() || c == '.' || c == 'Z'));
    }

    #[test]
    fn infer_phase_and_tool() {
        assert_eq!(
            infer_phase("system: You are a planning agent in a ReAct-style loop."),
            "plan"
        );
        assert_eq!(
            infer_phase("claim-falsification phase — Prefer answer"),
            "falsify"
        );
        let (kind, tool) = infer_step_meta(
            r#"{"step":"action","tool":"read_file","args":{"path":"README.md"}}"#,
        );
        assert_eq!(kind, "action");
        assert_eq!(tool.as_deref(), Some("read_file"));
    }

    #[test]
    fn preview_truncates() {
        let s: String = (0..300).map(|_| 'あ').collect();
        let p = preview_text(&s, 10);
        assert!(p.ends_with("..."));
        assert_eq!(p.chars().count(), 13);
    }

    #[test]
    fn rotates_when_over_max_bytes() {
        let dir = std::env::temp_dir().join(format!("harness_seed_rot_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("events.jsonl");
        fs::write(&path, vec![b'x'; 64]).unwrap();

        let cfg = LogRotationConfig {
            max_bytes: 32,
            max_files: 3,
        };
        rotate_log_file(&path, cfg).unwrap();
        assert!(!path.exists());
        assert!(backup_path(&path, 1).exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn console_summary_has_no_system_dump() {
        let mut react = ReActLoop::with_defaults(LlmBrain::new(MockLlmConnector));
        let result = react.run_turn("hello").unwrap();
        let line = format_turn_console_summary("hello", &result);
        assert!(line.starts_with('>'));
        assert!(line.contains("steps="));
        assert!(line.contains("tokens="));
        assert!(line.contains("phase="));
        assert!(!line.contains("system:"));
        assert!(!line.contains("You are an agent"));
        let steps = format_step_console_lines(&result);
        assert!(!steps.is_empty());
        assert!(steps.iter().all(|s| !s.contains("system:")));
    }

    #[test]
    fn appends_v1_json_line_without_full_prompt_key() {
        let dir = std::env::temp_dir().join(format!("harness_seed_log_{}", std::process::id()));
        let path = dir.join("events.jsonl");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let mut react = ReActLoop::with_defaults(LlmBrain::new(MockLlmConnector));
        let result = react.run_turn("hello").unwrap();
        let writer = ContextLogWriter::new(&path).with_run_id("test-run");
        writer.append_turn("hello", &result).unwrap();

        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"v\":1"));
        assert!(text.contains("\"kind\":\"turn.summary\""));
        assert!(text.contains("\"user_input\":\"hello\""));
        assert!(text.contains("\"run_id\":\"test-run\""));
        assert!(text.contains("\"prompt_preview\""));
        assert!(!text.contains("\"prompt\":\"system:"));
        // ISO8601-ish ts
        let line: serde_json::Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
        let ts = line["ts"].as_str().unwrap();
        assert!(ts.contains('T') && ts.ends_with('Z'), "{ts}");
        assert!(line["steps"].as_array().unwrap()[0].get("phase").is_some());

        let _ = fs::remove_dir_all(&dir);
    }
}
