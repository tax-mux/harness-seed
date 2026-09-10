//! 外側の推進ループ — 計画フェーズを順次実行し、要約を `recalled` に載せてロングコンテキストを分割する。

pub mod audit;
pub mod escalate;
pub mod evidence;
pub mod gates;
pub mod mode;
pub mod phase;

pub use audit::{
    claim_audit_rules, claim_falsification_retry_subtask, claim_falsification_subtask,
};
pub use escalate::{should_escalate_from_plan, ESCALATE_MIN_SUBTASKS};
pub use evidence::{
    count_ok_tool_observations, count_substantive_ok_observations, evidence_deepening_subtask,
    is_substantive_evidence_tool, prior_evidence_is_thin, MIN_OK_TOOL_OBSERVATIONS_BEFORE_JUDGMENT,
    MIN_SUBSTANTIVE_OK_OBSERVATIONS_BEFORE_JUDGMENT, SUBSTANTIVE_EVIDENCE_TOOLS,
};
pub use mode::AdvanceMode;
pub use gates::{
    apply_absence_gate, apply_citation_gate, classify_absence_claims, extract_absence_claims,
    looks_like_absence_claim, unverified_cited_paths, AbsenceClaimVerdict,
};
pub use phase::{
    build_phase_note, evidence_grounding_rules, evidence_paths_from_notes,
    evidence_paths_from_texts, format_recalled_progress, prepare_phase_recalled,
    prior_has_auditable_claims, restore_base_recalled, AdvancePhaseNote, AdvancePhaseSummary,
    AdvanceProgress,
};

#[cfg(test)]
mod tests;

/// 推進ループの設定（`config.json` の `react.advance`）。
#[derive(Debug, Clone)]
pub struct AdvanceConfig {
    /// 入り方。`Always` は旧 `enabled: true`。`FromPlan` は計画後に昇格判定する。
    pub mode: AdvanceMode,
    /// 1 リクエストあたりの最大フェーズ数（計画サブタスクの上限）。
    pub max_phases: usize,
    /// フェーズ間で `SessionMemory` をクリアする（先頭フェーズは保持）。
    pub clear_session_each_phase: bool,
    /// フェーズ要約を `recalled` に載せる最大文字数（1 フェーズあたり）。
    pub max_note_chars: usize,
    /// 各フェーズ開始を stdout に表示する。
    pub show_phases: bool,
    /// 判定前に必要な実質証拠（read/grep 等）成功 observation 数。
    pub min_substantive_obs: usize,
    /// 最終回答のパス引用を先行 Paths と照合し、無いものを未検証注記する。
    pub citation_check: bool,
    /// 結論・合成の前に、先行 Claims の否定証拠を一度探す。
    pub claim_check: bool,
    /// 最終回答の不在主張を trace と照合し、未検証・矛盾を注記する。
    pub absence_check: bool,
}

impl AdvanceConfig {
    /// 無条件に推進ループへ入るか（旧 `enabled`）。
    pub fn enabled(&self) -> bool {
        self.mode.is_always()
    }
}

impl Default for AdvanceConfig {
    fn default() -> Self {
        Self {
            mode: AdvanceMode::Off,
            max_phases: 8,
            clear_session_each_phase: true,
            max_note_chars: 1500,
            show_phases: true,
            min_substantive_obs: MIN_SUBSTANTIVE_OK_OBSERVATIONS_BEFORE_JUDGMENT,
            citation_check: true,
            claim_check: true,
            absence_check: true,
        }
    }
}
