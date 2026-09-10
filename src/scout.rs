//! 計画前スカウト — 要望を満たす情報が足りるか評価し、ツールで収集して `recalled` に載せる。

use serde::Deserialize;

use crate::action::TurnTrace;
use crate::brain::AgentBrain;
use crate::context::PromptBlocks;
use crate::layer::{run_layer_loop, LayerLoopOptions};
use crate::react::ReActError;
use crate::session::SessionMemory;
use crate::tool::ToolRuntime;

/// スカウトフェーズ設定（`config.json` の `react.scout`）。
#[derive(Debug, Clone)]
pub struct ScoutConfig {
    pub enabled: bool,
    /// スカウト ReAct の最大ステップ（ツール呼び出し含む）。
    pub max_steps: usize,
    /// 挨拶・help などでスカウトをスキップする。
    pub skip_trivial: bool,
    /// `recalled` に載せる `notes` の上限文字数。
    pub max_note_chars: usize,
    pub show_scout: bool,
}

impl Default for ScoutConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_steps: 6,
            skip_trivial: true,
            max_note_chars: 2000,
            show_scout: true,
        }
    }
}

/// スカウト層の成果（計画層へ渡す）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResearchArtifact {
    /// この内容で計画に進んでよいか。
    pub ready_to_plan: bool,
    /// まだ不明な点（`ready_to_plan == false` のとき有用）。
    pub gaps: Vec<String>,
    /// 計画層向けの調査メモ（観測の要約）。
    pub notes: String,
}

impl ResearchArtifact {
    pub fn trivial(reason: impl Into<String>) -> Self {
        Self {
            ready_to_plan: true,
            gaps: vec![],
            notes: reason.into(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct ResearchJson {
    #[serde(default)]
    ready_to_plan: bool,
    #[serde(default)]
    gaps: Vec<String>,
    #[serde(default)]
    notes: String,
}

/// 計画前スカウト用の system 追記（`PromptBlocks::system_extra` に一時注入）。
pub const SCOUT_SYSTEM_APPEND: &str = r#"

Scout phase (before planning):
- Your job is to decide if there is ENOUGH information to make a solid plan for the user's request.
- Use tools when the request depends on this repo, files, commands, or external/current facts.
- Do NOT produce a full execution plan or subtask list here — only gather and assess.
- When done (or at step limit), reply with ONE JSON object:
  {"step":"answer","content":"<ResearchArtifact JSON as string>"}

ResearchArtifact schema (inside answer content):
{
  "ready_to_plan": <true if enough facts to plan; false if critical unknowns remain>,
  "gaps": ["what is still missing, if any"],
  "notes": "<concise findings: paths, commands, URLs, constraints — for the planner>"
}
- For pure greetings / help / echo with no repo or web work: ready_to_plan true, short notes.
"#;

/// スカウト層の user ブロック前置き。
pub fn format_scout_user_input(user_input: &str) -> String {
    format!(
        "Scout request (assess information gaps BEFORE planning):\n\
         {user_input}\n\n\
         Gather only what planning needs. Finish with ResearchArtifact JSON in answer."
    )
}

/// LLM の answer 本文から [`ResearchArtifact`] を復元する。
pub fn artifact_from_scout_answer(answer: &str, trace: &TurnTrace) -> ResearchArtifact {
    let trimmed = strip_code_fence(answer.trim());
    if let Ok(json) = serde_json::from_str::<ResearchJson>(trimmed) {
        return ResearchArtifact {
            ready_to_plan: json.ready_to_plan,
            gaps: json.gaps,
            notes: json.notes,
        };
    }
    // フォールバック: ツール観測があれば計画可能とみなし、answer を notes に
    let has_observations = !trace.observations.is_empty();
    ResearchArtifact {
        ready_to_plan: has_observations || trimmed.contains("ready_to_plan"),
        gaps: if has_observations {
            vec![]
        } else {
            vec!["scout answer was not valid ResearchArtifact JSON".into()]
        },
        notes: if has_observations {
            summarize_observations(trace)
        } else if trimmed.is_empty() {
            String::new()
        } else {
            trimmed.to_string()
        },
    }
}

fn summarize_observations(trace: &TurnTrace) -> String {
    let mut out = String::new();
    for obs in &trace.observations {
        let status = if obs.ok { "ok" } else { "err" };
        let snippet: String = obs.output.chars().take(400).collect();
        out.push_str(&format!("[{status}] {snippet}\n"));
    }
    if out.is_empty() {
        out.push_str("(no tool observations)");
    }
    out
}

fn strip_code_fence(s: &str) -> &str {
    let s = s
        .strip_prefix("```json")
        .or_else(|| s.strip_prefix("```"))
        .unwrap_or(s);
    s.strip_suffix("```").unwrap_or(s).trim()
}

fn truncate(text: &str, max: usize) -> String {
    let max = max.max(80);
    if text.chars().count() <= max {
        return text.to_string();
    }
    format!("{}…", text.chars().take(max).collect::<String>())
}

/// 計画層向け `recalled` チャンク。
pub fn format_scout_recalled(artifact: &ResearchArtifact, max_note_chars: usize) -> String {
    let gaps = if artifact.gaps.is_empty() {
        "(none listed)".to_string()
    } else {
        artifact.gaps.join("; ")
    };
    format!(
        "## Scout findings (pre-plan research)\n\n\
         ready_to_plan: {}\n\
         gaps: {gaps}\n\n\
         notes:\n{}\n",
        artifact.ready_to_plan,
        truncate(&artifact.notes, max_note_chars)
    )
}

/// ホスト `recalled` を保持したままスカウト結果を追記する。
pub fn apply_scout_recalled(
    blocks: &mut PromptBlocks,
    base_recalled: &[String],
    artifact: &ResearchArtifact,
    max_note_chars: usize,
) {
    blocks.clear_recalled();
    for c in base_recalled {
        blocks.push_recalled(c.as_str());
    }
    blocks.push_recalled(format_scout_recalled(artifact, max_note_chars));
}

pub fn is_trivial_scout_skip(user_input: &str) -> bool {
    let t = user_input.trim();
    t.eq_ignore_ascii_case("help")
        || t.eq_ignore_ascii_case("time")
        || t.starts_with("echo ")
        || t.is_empty()
}

/// スカウト層 ReAct（ツール可）を 1 回走らせる。
pub fn run_scout_phase<E: AgentBrain>(
    exec_brain: &mut E,
    tools: &mut ToolRuntime,
    blocks: &mut PromptBlocks,
    session: &SessionMemory,
    user_input: &str,
    config: &ScoutConfig,
    verbose: bool,
    show_prompt: bool,
    show_tool_output: bool,
) -> Result<(ResearchArtifact, usize), ReActError> {
    let saved_extra = blocks.system_extra.clone();
    if !saved_extra.contains("Scout phase") {
        blocks.system_extra = format!("{saved_extra}{SCOUT_SYSTEM_APPEND}");
    }

    let scout_input = format_scout_user_input(user_input);
    let turn = run_layer_loop(
        exec_brain,
        tools,
        blocks,
        session,
        &scout_input,
        LayerLoopOptions::scout(config.max_steps),
        verbose,
        show_prompt,
        show_tool_output,
        None,
        vec![],
    )?;

    blocks.system_extra = saved_extra;

    let artifact = artifact_from_scout_answer(&turn.answer, &turn.trace);
    Ok((artifact, turn.steps_used))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::Observation;

    #[test]
    fn parses_research_json() {
        let raw = r#"{"ready_to_plan":false,"gaps":["API version"],"notes":"saw Cargo.toml"}"#;
        let a = artifact_from_scout_answer(raw, &TurnTrace::default());
        assert!(!a.ready_to_plan);
        assert_eq!(a.gaps[0], "API version");
        assert!(a.notes.contains("Cargo"));
    }

    #[test]
    fn format_recalled_includes_ready_flag() {
        let a = ResearchArtifact {
            ready_to_plan: true,
            gaps: vec![],
            notes: "layout ok".into(),
        };
        let t = format_scout_recalled(&a, 500);
        assert!(t.contains("ready_to_plan: true"));
        assert!(t.contains("layout ok"));
    }

    #[test]
    fn trivial_skip_detects_help() {
        assert!(is_trivial_scout_skip("help"));
        assert!(!is_trivial_scout_skip("refactor src/main.rs"));
    }

    #[test]
    fn fallback_uses_observations() {
        let mut trace = TurnTrace::default();
        trace.push_observation(Observation::success(1, "found lib.rs"));
        let a = artifact_from_scout_answer("not json", &trace);
        assert!(a.ready_to_plan);
        assert!(a.notes.contains("found lib.rs"));
    }
}
