//! 計画層（ReAct 派生ループ・ツールなし）→ 実行層（ReAct + ツール）の直列オーケストレーション。

mod brain;
mod candidates;
mod contract;
mod display;
mod parse;
mod parse_step;
mod prompt;
mod queue;
mod schedule;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::context::PromptBlocks;
use crate::session::SessionMemory;
use crate::tasks::TaskRegistry;

pub use candidates::{
    normalize_candidates, parse_candidate_selection, select_and_register_plan_candidates,
    select_and_register_plan_candidates_with_budget, CANDIDATE_SELECTION_SYSTEM,
    PLAN_CATALOG_SUMMARY_MAX_CHARS, PLAN_CATALOG_SUMMARY_MAX_ENTRIES,
};
pub use brain::{
    artifact_from_plan_turn, PlanBrainMode, PlanLlmBrain, RulePlanBrain, PLAN_REACT_SYSTEM_CORE,
};
pub use contract::{PlanDataContract, PlanEnforceFn};
pub use parse::{parse_plan, PlanParseError};
pub use schedule::{execution_waves, ScheduleError};
pub use parse_step::{
    harness_state_from_plan_answer as harness_state_from_plan_turn, parse_plan_agent_step,
    plan_artifact_from_answer, PlanStepParseError,
};
pub use prompt::{
    build_plan_layer_messages, build_plan_layer_messages_with_catalog, format_plan_fixed_zone_system,
    format_plan_layer_prompt,
};
pub use display::{
    format_plan_zone_after_preview, format_plan_zone_prompt_preview,
    format_planner_fixed_zone_html,
};
pub use queue::{
    control_plane_catalog_footer, is_replan_subtask, is_reserved_control_task, PlanQueue,
    PlanQueueError, REPLAN_TASK_ID,
};

/// 計画フェーズ用 system 指示（ツールカタログなし）。
pub const PLAN_SYSTEM_CORE: &str = r#"You are a planning agent. Reply with ONE JSON object only (no markdown).

Schema:
{
  "input": ["<fixed INPUT contract lines copied from prompt>"],
  "steps": [
    {"id": 1, "task": "<registered task id>", "params": {}, "goal": "", "done_when": ""},
    {"id": 2, "goal": "<freeform if no task id>", "done_when": "<criterion>"}
  ],
  "output": "<fixed OUTPUT contract line copied from prompt>",
  "skip_execution": <true only if knowledge_sufficient is true>,
  "knowledge_sufficient": <true if Recalled and/or general knowledge fully answer the user without tools>
}

Rules:
- Prefer registered task ids from the task catalog (with params). Each task declares required tool methods and execution order (`steps`).
- Break non-trivial work into ordered subtasks (1–5 items).
- Keep `input` and `output` equal to the fixed contract in prompt; only design `steps`.
- For external / current-events / web-only questions, use task `web_research` with params `{"query":"<search string>"}` when it appears in the catalog.
- For repo-only coding work, use tasks like `list_dir`, `write_file_verify`, or freeform goals with grep/read_file/write_file/run_cmd.
- Decide knowledge_sufficient first: true only when Recalled context and/or general knowledge fully answer the user (greetings, chit-chat, facts you can state without files). false when the answer needs workspace files, tools, or more evidence than Recalled provides (project overview, code, logs).
- skip_execution: true is allowed ONLY when knowledge_sufficient is true. The harness rejects skip otherwise and runs freeform execution (tools chosen from the catalog).
- When skip_execution is true, set `output` to the final user-facing reply (execution layer may not run).
- Recalled context: use it when relevant; general knowledge is fine when Recalled is irrelevant — do not claim it came from memory. Thin or off-topic Recalled is NOT sufficient for project-specific questions.
- Subtask ids must be unique positive integers starting at 1.
"#;

/// 1 サブタスク（登録タスク参照 or 自由記述）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Subtask {
    pub id: u32,
    /// `tasks/*.json` の id。`None` のときは `goal` / `done_when` をそのまま使う。
    pub task: Option<String>,
    pub params: Value,
    pub goal: String,
    pub done_when: String,
    /// このサブタスクより先に完了しているべき id（空なら依存なし）。
    /// 同一波内（互いに依存しない集合）は `parallel_subtasks` 時に並列実行できる。
    #[serde(default)]
    pub depends_on: Vec<u32>,
}

/// 弱すぎる `done_when`（番号付き計画の既定など）を証拠志向の完了条件へ置き換えるときの本文。
pub const EVIDENCE_ORIENTED_DONE_WHEN: &str =
    "goal met with concrete evidence from tools (cite paths, findings, or observations)";

/// `step completed` / `done` など、実質なんでも通る完了条件か。
pub fn is_weak_done_when(done_when: &str) -> bool {
    let t = done_when.trim().to_ascii_lowercase();
    t.is_empty()
        || matches!(
            t.as_str(),
            "step completed"
                | "step complete"
                | "done"
                | "ok"
                | "finished"
                | "complete"
                | "completed"
                | "完了"
                | "終了"
        )
}

/// 弱い `done_when` を証拠志向の文言へ上げる（制御プレーン以外）。
pub fn strengthen_weak_done_when(subtask: &mut Subtask) {
    if subtask
        .task
        .as_deref()
        .is_some_and(queue::is_reserved_control_task)
    {
        return;
    }
    if is_weak_done_when(&subtask.done_when) {
        subtask.done_when = EVIDENCE_ORIENTED_DONE_WHEN.into();
    }
}

/// 計画フェーズの成果物。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanArtifact {
    pub summary: String,
    pub skip_execution: bool,
    pub subtasks: Vec<Subtask>,
    /// Recalled / 一般知識だけで最終回答できるか。
    /// `skip_execution: true` はこれが `Some(true)` のときだけ許可（ハーネスが強制）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knowledge_sufficient: Option<bool>,
    /// `skip_execution` 時のユーザー向け本文（ハーネス／パーサが構造的にセット）。
    /// `None` なら exec LLM フォールバック（内部ラベルを返答に使わない）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_reply: Option<String>,
}

impl PlanArtifact {
    /// 計画をスキップし、元の入力をそのまま 1 回の実行ループへ渡す。
    pub fn passthrough(_user_input: &str) -> Self {
        Self {
            summary: "direct execution".into(),
            skip_execution: true,
            subtasks: vec![],
            knowledge_sufficient: Some(true),
            user_reply: None,
        }
    }

    /// 実行ループへそのまま渡す単一サブタスク。
    pub fn single_subtask(user_input: &str) -> Self {
        Self {
            summary: "single task".into(),
            skip_execution: false,
            subtasks: vec![],
            knowledge_sufficient: Some(false),
            user_reply: None,
        }
        .with_single_goal(user_input)
    }

    fn with_single_goal(mut self, user_input: &str) -> Self {
        self.subtasks = vec![Subtask {
            id: 1,
            task: None,
            params: json!({}),
            goal: user_input.to_string(),
            done_when: "user request satisfied".into(),
            depends_on: vec![],
        }];
        self
    }

    /// 雑談等: skip するがユーザー本文は無く、exec LLM に任せる。
    pub fn skip_needs_exec(summary: impl Into<String>) -> Self {
        Self {
            summary: summary.into(),
            skip_execution: true,
            subtasks: vec![],
            knowledge_sufficient: Some(true),
            user_reply: None,
        }
    }

    /// skip し、そのまま返すユーザー本文を持つ。
    pub fn skip_with_reply(summary: impl Into<String>, reply: impl Into<String>) -> Self {
        Self {
            summary: summary.into(),
            skip_execution: true,
            subtasks: vec![],
            knowledge_sufficient: Some(true),
            user_reply: Some(reply.into()),
        }
    }

    /// 実行フェーズに進むか。
    pub fn needs_execution(&self) -> bool {
        !self.skip_execution && !self.subtasks.is_empty()
    }

    /// `skip_execution` は `knowledge_sufficient == true` のときだけ許可。
    /// 不足（false / 未設定）なら実行へ落とし、steps が空なら自由記述の実行 subtask を足す。
    /// （特定ドメインのツール列は決めない — 実行層 ReAct がカタログから選ぶ。）
    pub fn enforce_knowledge_sufficiency(&mut self) {
        if self.skip_execution {
            if self.knowledge_sufficient == Some(true) {
                return;
            }
            self.skip_execution = false;
            self.knowledge_sufficient = Some(false);
            self.user_reply = None;
        }
        if !self.skip_execution && self.subtasks.is_empty() {
            self.subtasks = vec![default_evidence_subtask(self.summary.trim())];
            if self.knowledge_sufficient != Some(true) {
                self.knowledge_sufficient = Some(false);
            }
        }
    }

    /// `skip_execution` 時に exec LLM を呼ばず返す本文。
    ///
    /// 優先: 構造化 `user_reply` → 計画 JSON の `output`。
    /// summary / 平文 WI は内部ラベルになり得るため使わない。
    pub fn direct_reply(&self, work_instructions: &str) -> Option<String> {
        if let Some(r) = &self.user_reply {
            let t = r.trim();
            if !t.is_empty() {
                return Some(t.to_string());
            }
            return None;
        }
        let wi = work_instructions.trim();
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(wi) {
            if let Some(o) = v
                .get("output")
                .and_then(|x| x.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                return Some(o.to_string());
            }
        }
        None
    }
}

/// 知識不足で steps が空のときの汎用実行 subtask（タスク id 固定なし）。
fn default_evidence_subtask(summary: &str) -> Subtask {
    let goal = if summary.chars().count() >= 4 {
        format!(
            "{summary}. Recalled context alone is insufficient — use available tools as needed, then fully answer the user."
        )
    } else {
        "Recalled context alone is insufficient — use available tools as needed to gather evidence, then fully answer the user.".into()
    };
    Subtask {
        id: 1,
        task: None,
        params: json!({}),
        goal,
        done_when: "user request satisfied with sufficient evidence".into(),
        depends_on: vec![],
    }
}

/// 計画層の成果物をコンソール向けに整形する。
pub fn format_plan_for_display(plan: &PlanArtifact, registry: &TaskRegistry) -> String {
    let mut out = String::from("--- Plan ---\n");
    out.push_str(&format!("summary: {}\n", plan.summary));
    out.push_str(&format!(
        "skip_execution: {}\n",
        plan.skip_execution
    ));
    if plan.subtasks.is_empty() {
        out.push_str("subtasks: (none)\n");
    } else {
        out.push_str("subtasks:\n");
        for st in &plan.subtasks {
            let tag = st
                .task
                .as_deref()
                .map(|t| format!("task:{t}"))
                .unwrap_or_else(|| "freeform".into());
            let params = if st.params.as_object().is_some_and(|o| !o.is_empty()) {
                format!(" params={}", st.params)
            } else {
                String::new()
            };
            out.push_str(&format!(
                "  - id {} [{tag}]{params}\n    goal: {}\n    done_when: {}\n",
                st.id, st.goal, st.done_when
            ));
            let exec = registry.format_subtask_execution_for_display(st);
            for line in exec.lines() {
                out.push_str("    ");
                out.push_str(line);
                out.push('\n');
            }
        }
    }
    out.push_str("--- end plan ---");
    out
}

/// サブタスク実行の要約（次サブタスクへの引き継ぎ用）。
#[derive(Debug, Clone, Default)]
pub struct PlanProgress {
    pub results: Vec<(u32, String)>,
}

impl PlanProgress {
    pub fn push(&mut self, id: u32, summary: impl Into<String>) {
        self.results.push((id, summary.into()));
    }

    pub fn format_for_mission(&self) -> String {
        if self.results.is_empty() {
            return "(none yet)\n".into();
        }
        let mut out = String::new();
        for (id, text) in &self.results {
            let snippet: String = text.chars().take(500).collect();
            let suffix = if text.chars().count() > 500 { "…" } else { "" };
            out.push_str(&format!("[{id}] {snippet}{suffix}\n"));
        }
        out
    }
}

/// 計画フェーズ用プロンプト文脈。
#[derive(Debug, Clone, Copy)]
pub struct PlanPromptContext<'a> {
    pub blocks: &'a PromptBlocks,
    pub user_input: &'a str,
    pub session: &'a SessionMemory,
    pub task_registry: Option<&'a TaskRegistry>,
}

impl<'a> PlanPromptContext<'a> {
    pub fn new(
        blocks: &'a PromptBlocks,
        user_input: &'a str,
        session: &'a SessionMemory,
        task_registry: Option<&'a TaskRegistry>,
    ) -> Self {
        Self {
            blocks,
            user_input,
            session,
            task_registry,
        }
    }

    pub fn render(&self) -> Vec<crate::llm::ChatMessage> {
        vec![
            crate::llm::ChatMessage::system(self.system_content()),
            crate::llm::ChatMessage::user(self.user_content()),
        ]
    }

    fn system_content(&self) -> String {
        let mut out = String::from(PLAN_SYSTEM_CORE);
        if !self.blocks.rules.is_empty() {
            out.push_str("\n\nAdditional rules:\n");
            for (i, rule) in self.blocks.rules.iter().enumerate() {
                out.push_str(&format!("\n[rule {}]\n{rule}\n", i + 1));
            }
        }
        if !self.blocks.recalled.is_empty() {
            out.push_str("\n\nRecalled context:\n");
            for (i, chunk) in self.blocks.recalled.iter().enumerate() {
                out.push_str(&format!("\n[recalled {}]\n{chunk}\n", i + 1));
            }
        }
        if !self.blocks.system_extra.is_empty() {
            out.push_str("\n\n");
            out.push_str(&self.blocks.system_extra);
        }
        if let Some(reg) = self.task_registry {
            out.push_str("\n\n");
            out.push_str(&reg.catalog_for_planner_opts(self.blocks.web_search_enabled));
        }
        out
    }

    fn user_content(&self) -> String {
        let previous = self.session.format_for_prompt();
        let previous_block = if previous.is_empty() {
            String::new()
        } else {
            format!("{previous}\n")
        };
        format!(
            "{previous_block}ゴール:\n{}\n\nOutput plan JSON:",
            self.user_input
        )
    }
}

/// 実行ループへ渡す mission プロンプト（タスクレジストリ経由）。
pub fn format_mission(
    registry: &TaskRegistry,
    original: &str,
    plan: &PlanArtifact,
    subtask: &Subtask,
    progress: &PlanProgress,
) -> String {
    registry
        .render_mission(original, plan, subtask, progress)
        .unwrap_or_else(|err| {
            eprintln!("[tasks] mission render fallback: {err}");
            format_mission_freeform(original, plan, subtask, progress)
        })
}

fn format_mission_freeform(
    original: &str,
    _plan: &PlanArtifact,
    subtask: &Subtask,
    progress: &PlanProgress,
) -> String {
    let task = subtask
        .task
        .as_deref()
        .unwrap_or("(freeform)");
    let reference = if subtask.task.is_none() {
        original.trim()
    } else {
        ""
    };
    let mut mission = format!(
        "## Subtask\nid: {}\ntask: {}\nparams: {}\ngoal: {}\ndone_when: {}\n\n\
         ## Task contract\n(freeform)\n\n\
         ## Prior subtask results\n{}",
        subtask.id,
        task,
        subtask.params,
        subtask.goal,
        subtask.done_when,
        progress.format_for_mission(),
    );
    if !reference.is_empty() {
        mission.push_str("\n\n## User request (reference)\n");
        mission.push_str(reference);
        mission.push('\n');
    }
    mission.push_str(
        "\nComplete ONLY this subtask. Do not invent control-plane actions \
         (e.g. a replan tool) or work ahead to other subtasks.",
    );
    mission
}

#[cfg(test)]
mod direct_reply_tests {
    use super::*;

    #[test]
    fn prefers_output_field_in_work_instructions() {
        let plan = PlanArtifact {
            summary: "short label".into(),
            skip_execution: true,
            subtasks: vec![],
            knowledge_sufficient: Some(true),
            user_reply: None,
        };
        let wi = r#"{"summary":"short label","skip_execution":true,"subtasks":[],"output":"最終回答本文"}"#;
        assert_eq!(
            plan.direct_reply(wi).as_deref(),
            Some("最終回答本文")
        );
    }

    #[test]
    fn uses_structured_user_reply() {
        let plan = PlanArtifact::skip_with_reply(
            "label",
            "これは十分な長さの要約回答です",
        );
        assert!(plan.direct_reply("{}").unwrap().contains("要約回答"));
    }

    #[test]
    fn ignores_placeholder_summary() {
        let plan = PlanArtifact::passthrough("hi");
        assert!(plan.direct_reply("").is_none());
    }

    #[test]
    fn ignores_harness_internal_work_instructions() {
        let plan = PlanArtifact::skip_needs_exec("direct chat");
        assert!(plan
            .direct_reply("(no task candidates — direct chat)")
            .is_none());
        assert!(plan
            .direct_reply("(trivial chat — plan layer skipped)")
            .is_none());
    }
}
