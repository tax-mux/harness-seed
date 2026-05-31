//! Harness 内部状態（LLM には JSON を渡さず、テキスト変換のみ）。

use serde::{Deserialize, Serialize};

use super::reference::{format_references_for_prompt, HarnessReference};
use crate::plan::{PlanArtifact, Subtask};
use crate::tasks::{SubtaskToolPolicy, TaskRegistry};

/// 計画フェーズの Harness パース結果。実行層・ミニ Planner はこの JSON を参照する。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HarnessState {
    /// Planner が返した作業指示書（生テキスト）。
    pub work_instructions: String,
    /// パース済み計画（Harness 専用。LLM には [`format_work_instructions_for_prompt`] で渡す）。
    pub plan: PlanArtifact,
    /// 現在のサブタスク番号（1 始まり。未実行・スキップ時は 0）。
    pub current_step: u32,
    /// 実行対象サブタスク総数。
    pub total_steps: u32,
    /// 現在ステップで注入するツール名（ミニ Planner / tool_policy 由来）。
    #[serde(default)]
    pub tool_set: Vec<String>,
    /// ターン開始時に注入する参照文書（メール等）。
    #[serde(default)]
    pub references: Vec<HarnessReference>,
    pub status: HarnessStatus,
}

/// Harness 内部状態のライフサイクル。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HarnessStatus {
    /// 計画直後・実行前。
    Ready,
    /// サブタスク実行中。
    Executing,
    /// 全サブタスク完了。
    Completed,
    /// ユーザー中断等。
    Aborted,
}

impl HarnessState {
    pub fn new(work_instructions: impl Into<String>, plan: PlanArtifact) -> Self {
        let work_instructions = work_instructions.into();
        let total_steps = if plan.skip_execution {
            0
        } else {
            plan.subtasks.len() as u32
        };
        let current_step = if total_steps > 0 { 1 } else { 0 };
        Self {
            work_instructions,
            plan,
            current_step,
            total_steps,
            tool_set: Vec::new(),
            references: Vec::new(),
            status: if total_steps > 0 {
                HarnessStatus::Ready
            } else {
                HarnessStatus::Completed
            },
        }
    }

    pub fn current_subtask(&self) -> Option<&Subtask> {
        if self.current_step == 0 {
            return None;
        }
        self.plan
            .subtasks
            .iter()
            .find(|st| st.id == self.current_step)
    }

    /// 実行開始前に呼ぶ（`current_step` を 1 にし status を Executing に）。
    pub fn begin_execution(&mut self) {
        if self.total_steps > 0 && self.current_step == 0 {
            self.current_step = 1;
        }
        if self.total_steps > 0 {
            self.status = HarnessStatus::Executing;
        }
    }

    /// サブタスク完了後に次へ進める。戻り値はまだ残りがあるか。
    pub fn advance_after_subtask(&mut self, completed_id: u32) -> bool {
        if completed_id != self.current_step {
            return self.current_step > 0 && self.current_step <= self.total_steps;
        }
        if self.current_step >= self.total_steps {
            self.status = HarnessStatus::Completed;
            self.tool_set.clear();
            return false;
        }
        self.current_step += 1;
        self.tool_set.clear();
        true
    }

    pub fn set_tool_set(&mut self, tools: Vec<String>) {
        self.tool_set = tools;
    }

    pub fn set_tool_set_from_policy(&mut self, policy: Option<&SubtaskToolPolicy>) {
        self.tool_set = policy.map(|p| p.allow.clone()).unwrap_or_default();
    }

    pub fn add_references(&mut self, refs: impl IntoIterator<Item = HarnessReference>) {
        self.references.extend(refs);
    }

    /// 固定ゾーン用: 参照情報（Harness `references` ノードから生成）。
    pub fn format_references_for_prompt(&self) -> String {
        Self::format_references_for_prompt_from_slice(&self.references)
    }

    pub fn format_references_for_prompt_from_slice(refs: &[HarnessReference]) -> String {
        format_references_for_prompt(refs)
    }

    /// 固定ゾーン用: 作業指示書テキスト（Planner 出力の要約 or 全文）。
    pub fn format_work_instructions_for_prompt(&self) -> String {
        let trimmed = self.work_instructions.trim();
        if trimmed.is_empty() {
            return self.plan.summary.clone();
        }
        trimmed.to_string()
    }

    /// 固定ゾーン用: 今のステップ（Harness が JSON から生成したテキスト）。
    pub fn format_current_step_for_prompt(&self, registry: &TaskRegistry) -> String {
        let Some(st) = self.current_subtask() else {
            return if self.plan.skip_execution {
                "(no execution steps — direct reply)".into()
            } else {
                "(all steps completed)".into()
            };
        };
        let mut out = format!(
            "Step {}/{} (id {})\n",
            self.current_step, self.total_steps, st.id
        );
        if let Some(ref task) = st.task {
            out.push_str(&format!("task: {task}\n"));
        }
        if !st.goal.is_empty() {
            out.push_str(&format!("goal: {}\n", st.goal));
        }
        if !st.done_when.is_empty() {
            out.push_str(&format!("done_when: {}\n", st.done_when));
        }
        if !st.params.is_null() && st.params != serde_json::json!({}) {
            out.push_str(&format!("params: {}\n", st.params));
        }
        let exec = registry.format_subtask_execution_for_display(st);
        if !exec.is_empty() {
            out.push_str("execution contract:\n");
            for line in exec.lines() {
                out.push_str("  ");
                out.push_str(line);
                out.push('\n');
            }
        }
        if !self.tool_set.is_empty() {
            out.push_str(&format!(
                "allowed tools (this step): {}\n",
                self.tool_set.join(", ")
            ));
        }
        match self.status {
            HarnessStatus::Ready => out.push_str("status: ready\n"),
            HarnessStatus::Executing => out.push_str("status: executing\n"),
            HarnessStatus::Completed => out.push_str("status: completed\n"),
            HarnessStatus::Aborted => out.push_str("status: aborted\n"),
        }
        out
    }

    /// デバッグ・ログ向け JSON（内部状態のスナップショット）。
    pub fn to_json_pretty(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".into())
    }

    /// パース直後の内部状態を stderr に出力する。
    pub fn eprintln_parsed(&self) {
        eprintln!(
            "### Harness内部状態（JSON） ###\n{}\n### END Harness内部状態（JSON） ###",
            self.to_json_pretty()
        );
    }
}
