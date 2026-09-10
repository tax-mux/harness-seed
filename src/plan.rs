//! 計画層（ReAct 派生ループ・ツールなし）→ 実行層（ReAct + ツール）の直列オーケストレーション。

mod brain;
mod contract;
mod display;
mod parse;
mod parse_step;
mod prompt;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::context::PromptBlocks;
use crate::session::SessionMemory;
use crate::tasks::TaskRegistry;

pub use brain::{
    artifact_from_plan_turn, PlanBrainMode, PlanLlmBrain, RulePlanBrain, PLAN_REACT_SYSTEM_CORE,
};
pub use contract::{PlanDataContract, PlanReadSource, PlanWriteTarget};
pub use parse::{parse_plan, PlanParseError};
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
  "skip_execution": <true if trivial chat/help with no tools needed>,
}

Rules:
- Prefer registered task ids from the task catalog (with params). Each task declares required tool methods and execution order (`steps`).
- Break non-trivial work into ordered subtasks (1–5 items).
- Keep `input` and `output` equal to the fixed contract in prompt; only design `steps`.
- For external / current-events / web-only questions, use task `web_research` with params `{"query":"<search string>"}` when it appears in the catalog.
- For repo-only coding work, use tasks like `list_dir`, `write_file_verify`, or freeform goals with grep/read_file/write_file/run_cmd.
- Use skip_execution: true only for pure Q&A, greetings, or help with no filesystem/shell/web work.
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
}

/// 計画フェーズの成果物。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanArtifact {
    pub summary: String,
    pub skip_execution: bool,
    pub subtasks: Vec<Subtask>,
}

impl PlanArtifact {
    /// 計画をスキップし、元の入力をそのまま 1 回の実行ループへ渡す。
    pub fn passthrough(_user_input: &str) -> Self {
        Self {
            summary: "direct execution".into(),
            skip_execution: true,
            subtasks: vec![],
        }
    }

    /// 実行ループへそのまま渡す単一サブタスク。
    pub fn single_subtask(user_input: &str) -> Self {
        Self {
            summary: "single task".into(),
            skip_execution: false,
            subtasks: vec![Subtask {
                id: 1,
                task: None,
                params: json!({}),
                goal: user_input.to_string(),
                done_when: "user request satisfied".into(),
            }],
        }
    }

    /// 実行フェーズに進むか。
    pub fn needs_execution(&self) -> bool {
        !self.skip_execution && !self.subtasks.is_empty()
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
        "\nComplete ONLY this subtask. Do not replan or work ahead to other subtasks.",
    );
    mission
}
