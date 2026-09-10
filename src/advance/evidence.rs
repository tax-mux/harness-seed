//! 証拠評価関連。

use crate::action::TurnTrace;

/// 判定前に欲しがる「中身のある」成功ツール observation の既定下限。
pub const MIN_SUBSTANTIVE_OK_OBSERVATIONS_BEFORE_JUDGMENT: usize = 3;

/// 後方互換エイリアス（旧名）。
pub const MIN_OK_TOOL_OBSERVATIONS_BEFORE_JUDGMENT: usize =
    MIN_SUBSTANTIVE_OK_OBSERVATIONS_BEFORE_JUDGMENT;

/// 浅い列挙だけでは証拠に数えないツール以外＝実質証拠として数えるツール。
pub const SUBSTANTIVE_EVIDENCE_TOOLS: &[&str] =
    &["read_file", "grep", "web_search", "run_cmd", "write_file"];

/// 成功したツール observation 数（失敗は除外。浅い list も含む）。
pub fn count_ok_tool_observations(trace: &TurnTrace) -> usize {
    trace.observations.iter().filter(|o| o.ok).count()
}

/// 中身のある成功ツール observation 数（`list_dir` 等の浅い列挙は除外）。
pub fn count_substantive_ok_observations(trace: &TurnTrace) -> usize {
    let mut n = 0usize;
    for (action, obs) in trace.actions.iter().zip(trace.observations.iter()) {
        if obs.ok && is_substantive_evidence_tool(&action.tool) {
            n += 1;
        }
    }
    n
}

pub fn is_substantive_evidence_tool(name: &str) -> bool {
    SUBSTANTIVE_EVIDENCE_TOOLS
        .iter()
        .any(|t| name.eq_ignore_ascii_case(t))
}

pub fn prior_evidence_is_thin(substantive_ok: usize, min_substantive_ok: usize) -> bool {
    substantive_ok < min_substantive_ok.max(1)
}

/// 先行証拠が薄いときに差し込む自由記述サブタスク。
pub fn evidence_deepening_subtask(id: u32) -> crate::plan::Subtask {
    crate::plan::Subtask {
        id,
        task: None,
        params: serde_json::json!({}),
        goal: "Prior phase evidence is thin on substantive tools (read/grep/search/run). \
Gather more concrete evidence — prefer read_file, grep, or web_search over repeated list_dir. \
Cite specific paths and findings."
            .into(),
        done_when: crate::plan::EVIDENCE_ORIENTED_DONE_WHEN.into(),
        depends_on: vec![],
    }
}
