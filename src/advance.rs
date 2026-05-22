//! 外側の推進ループ — 計画フェーズを順次実行し、要約を `recalled` に載せてロングコンテキストを分割する。

use crate::context::PromptBlocks;
use crate::plan::{PlanArtifact, Subtask};
use serde_json::json;

/// 推進ループの設定（`config.json` の `react.advance`）。
#[derive(Debug, Clone)]
pub struct AdvanceConfig {
    /// 有効時は `run_turn` が計画 → フェーズ逐次実行（`two_phase` より優先）。
    pub enabled: bool,
    /// 1 リクエストあたりの最大フェーズ数（計画サブタスクの上限）。
    pub max_phases: usize,
    /// 各フェーズの前に `SessionMemory` をクリアする（`Previous turns` を載せない）。
    pub clear_session_each_phase: bool,
    /// フェーズ要約を `recalled` に載せる最大文字数（1 フェーズあたり）。
    pub max_note_chars: usize,
    /// 各フェーズ開始を stdout に表示する。
    pub show_phases: bool,
}

impl Default for AdvanceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_phases: 8,
            clear_session_each_phase: true,
            max_note_chars: 1500,
            show_phases: true,
        }
    }
}

/// 完了フェーズの記録（次フェーズへ `recalled` 注入）。
#[derive(Debug, Clone, Default)]
pub struct AdvanceProgress {
    pub mission: String,
    pub plan_summary: String,
    pub steps: Vec<AdvancePhaseNote>,
}

/// 1 フェーズ分のメモ。
#[derive(Debug, Clone)]
pub struct AdvancePhaseNote {
    pub id: u32,
    pub goal: String,
    pub answer: String,
}

impl AdvanceProgress {
    pub fn new(mission: impl Into<String>, plan_summary: impl Into<String>) -> Self {
        Self {
            mission: mission.into(),
            plan_summary: plan_summary.into(),
            steps: Vec::new(),
        }
    }

    pub fn push(&mut self, id: u32, goal: impl Into<String>, answer: impl Into<String>) {
        self.steps.push(AdvancePhaseNote {
            id,
            goal: goal.into(),
            answer: answer.into(),
        });
    }
}

/// 推進ループ 1 フェーズの実行サマリ（`TurnResult.advance_phases` 用）。
#[derive(Debug, Clone)]
pub struct AdvancePhaseSummary {
    pub id: u32,
    pub goal: String,
    pub answer: String,
    pub steps_used: usize,
}

fn truncate_note(text: &str, max_chars: usize) -> String {
    let max_chars = max_chars.max(80);
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_string();
    }
    let snippet: String = text.chars().take(max_chars).collect();
    format!("{snippet}…")
}

/// 完了フェーズの要約を `recalled` 用テキストにする。
pub fn format_recalled_progress(
    progress: &AdvanceProgress,
    plan: &PlanArtifact,
    max_note_chars: usize,
) -> String {
    let mut out = String::from("## Advance progress (completed phases only)\n\n");
    out.push_str(&format!("Mission: {}\n", progress.mission));
    out.push_str(&format!(
        "Plan summary: {}\n",
        if progress.plan_summary.is_empty() {
            &plan.summary
        } else {
            &progress.plan_summary
        }
    ));
    if progress.steps.is_empty() {
        out.push_str("\n(No prior phases yet.)\n");
        return out;
    }
    out.push('\n');
    for note in &progress.steps {
        out.push_str(&format!(
            "### Phase {} — done\nGoal: {}\nResult:\n{}\n\n",
            note.id,
            note.goal,
            truncate_note(&note.answer, max_note_chars)
        ));
    }
    out.push_str(
        "Use the above as ground truth. Do not redo completed phases unless the current goal requires it.\n",
    );
    out
}

fn format_phase_directive(plan: &PlanArtifact, current: &Subtask) -> String {
    let mut out = String::from("## Current phase (execute ONLY this)\n\n");
    out.push_str(&format!(
        "Phase {} / {}\nGoal: {}\nDone when: {}\n\n",
        current.id,
        plan.subtasks.len(),
        current.goal,
        current.done_when
    ));
    if let Some(task) = &current.task {
        out.push_str(&format!("Registered task id: {task}\n"));
    }
    out.push_str(
        "Complete only this phase. Prior phase results are in Recalled context above.\n",
    );
    out
}

/// フェーズ開始前に `PromptBlocks::recalled` を組み立てる（ホスト注入分は保持）。
pub fn prepare_phase_recalled(
    blocks: &mut PromptBlocks,
    base_recalled: &[String],
    progress: &AdvanceProgress,
    plan: &PlanArtifact,
    current: &Subtask,
    config: &AdvanceConfig,
) {
    blocks.clear_recalled();
    for chunk in base_recalled {
        blocks.push_recalled(chunk.as_str());
    }
    if !progress.steps.is_empty() {
        blocks.push_recalled(format_recalled_progress(
            progress,
            plan,
            config.max_note_chars,
        ));
    }
    blocks.push_recalled(format_phase_directive(plan, current));
}

/// 推進ループ終了後にホストの `recalled` を復元する。
pub fn restore_base_recalled(blocks: &mut PromptBlocks, base_recalled: &[String]) {
    blocks.clear_recalled();
    for chunk in base_recalled {
        blocks.push_recalled(chunk.as_str());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::PlanArtifact;

    #[test]
    fn recalled_progress_lists_prior_phases() {
        let plan = PlanArtifact::single_subtask("mission");
        let mut progress = AdvanceProgress::new("mission", "plan sum");
        progress.push(1, "first goal", "first answer");
        let text = format_recalled_progress(&progress, &plan, 500);
        assert!(text.contains("Phase 1 — done"));
        assert!(text.contains("first answer"));
        assert!(text.contains("Mission: mission"));
    }

    #[test]
    fn second_phase_recalled_contains_first_answer() {
        let plan = PlanArtifact {
            summary: "two steps".into(),
            skip_execution: false,
            subtasks: vec![
                Subtask {
                    id: 1,
                    task: None,
                    params: json!({}),
                    goal: "step one".into(),
                    done_when: "done".into(),
                },
                Subtask {
                    id: 2,
                    task: None,
                    params: json!({}),
                    goal: "step two".into(),
                    done_when: "done".into(),
                },
            ],
        };
        let mut progress = AdvanceProgress::new("mission", "two steps");
        progress.push(1, "step one", "answer one");
        let text = format_recalled_progress(&progress, &plan, 500);
        assert!(text.contains("answer one"));
        assert!(text.contains("Phase 1 — done"));
    }

    #[test]
    fn prepare_phase_includes_directive() {
        let plan = PlanArtifact::single_subtask("do thing");
        let progress = AdvanceProgress::default();
        let mut blocks = PromptBlocks::new();
        blocks.push_recalled("host note");
        let base = blocks.recalled.clone();
        let st = plan.subtasks[0].clone();
        prepare_phase_recalled(
            &mut blocks,
            &base,
            &progress,
            &plan,
            &st,
            &AdvanceConfig::default(),
        );
        // base was cleared and re-pushed; should have host + directive
        assert!(blocks.recalled.iter().any(|c| c.contains("host note")));
        assert!(blocks.recalled.iter().any(|c| c.contains("Current phase")));
    }
}
