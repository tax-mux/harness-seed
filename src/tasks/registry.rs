use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::action::TurnTrace;
use crate::plan::{PlanArtifact, PlanProgress, Subtask};
use crate::tool::workspace_root;

use super::audit::{audit_trace, TaskExecutionAudit};
use super::spec::{TaskDefinition, TaskError};

/// 組み込みタスク JSON（`tasks/` ディレクトリと同期すること）。
const BUILTIN_LIST_DIR: &str = include_str!("../../tasks/list_dir.json");
const BUILTIN_GENERIC: &str = include_str!("../../tasks/generic.json");
const BUILTIN_WRITE_FILE_VERIFY: &str = include_str!("../../tasks/write_file_verify.json");
const BUILTIN_WEB_RESEARCH: &str = include_str!("../../tasks/web_research.json");

#[derive(Debug)]
pub enum TaskLoadError {
    Read { path: PathBuf, source: std::io::Error },
    Parse { path: PathBuf, source: serde_json::Error },
    Invalid { path: PathBuf, reason: String },
}

impl fmt::Display for TaskLoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, source } => {
                write!(f, "failed to read {}: {source}", path.display())
            }
            Self::Parse { path, source } => {
                write!(f, "failed to parse {}: {source}", path.display())
            }
            Self::Invalid { path, reason } => {
                write!(f, "invalid task file {}: {reason}", path.display())
            }
        }
    }
}

impl std::error::Error for TaskLoadError {}

/// 機能塊タスクのレジストリ。
#[derive(Debug, Clone, Default)]
pub struct TaskRegistry {
    tasks: HashMap<String, TaskDefinition>,
}

impl TaskRegistry {
    pub fn builtin() -> Self {
        let mut reg = Self::default();
        reg.register_embedded(BUILTIN_LIST_DIR).expect("list_dir.json");
        reg.register_embedded(BUILTIN_GENERIC).expect("generic.json");
        reg.register_embedded(BUILTIN_WRITE_FILE_VERIFY).expect("write_file_verify.json");
        reg.register_embedded(BUILTIN_WEB_RESEARCH).expect("web_research.json");
        reg
    }

    pub fn load_default() -> Self {
        let mut reg = Self::builtin();
        let dir = workspace_root().join("tasks");
        if dir.is_dir() {
            if let Err(err) = reg.load_dir(&dir) {
                eprintln!("[tasks] load_dir {}: {err}", dir.display());
            }
        }
        reg
    }

    pub fn register(&mut self, def: TaskDefinition) -> Result<(), TaskError> {
        def.validate_definition().map_err(|reason| TaskError::InvalidDefinition {
            id: def.id.clone(),
            reason,
        })?;
        self.tasks.insert(def.id.clone(), def);
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<&TaskDefinition> {
        self.tasks.get(id)
    }

    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.tasks.keys().map(String::as_str)
    }

    /// 計画 LLM 向けカタログ（必須実行順序付き）。
    pub fn catalog_for_planner(&self) -> String {
        self.catalog_for_planner_opts(true)
    }

    /// 計画層向けタスク一覧。`include_web_research` が false のとき `web_research` を除外する。
    pub fn catalog_for_planner_opts(&self, include_web_research: bool) -> String {
        let mut lines = vec![
            "Registered tasks (use task id + params; steps define required method order):".into(),
        ];
        let mut ids: Vec<_> = self.tasks.keys().collect();
        ids.sort();
        for id in ids {
            if *id == "web_research" && !include_web_research {
                continue;
            }
            let def = &self.tasks[id];
            let steps = def
                .ordered_required_steps()
                .iter()
                .map(|s| s.method.as_str())
                .collect::<Vec<_>>()
                .join(" → ");
            let steps_part = if steps.is_empty() {
                "(free execution)".into()
            } else {
                format!("required: {steps}")
            };
            lines.push(format!("- {id}: {} — {steps_part}", def.summary));
        }
        lines.join("\n")
    }

    /// サブタスクの実行方式・ツール手順をコンソール向けに整形する。
    pub fn format_subtask_execution_for_display(&self, subtask: &Subtask) -> String {
        let mut out = String::new();
        if let Some(task_id) = &subtask.task {
            let Some(def) = self.get(task_id) else {
                return format!("task: {task_id} (unknown — not in registry)\n");
            };
            let merged = merge_params(&def.default_params, &subtask.params);
            out.push_str(&format!("task: {task_id} — {}\n", def.summary));
            if self.use_step_driver(subtask) {
                out.push_str("run: step-driver (fixed order)\n");
            } else {
                out.push_str("run: ReAct loop (LLM may choose tools; contract is advisory)\n");
            }
            out.push_str(&def.format_required_execution(&merged));
        } else {
            out.push_str("run: ReAct loop (freeform)\n");
            out.push_str(&format!("goal: {}\n", subtask.goal));
            if !subtask.done_when.is_empty() {
                out.push_str(&format!("done_when: {}\n", subtask.done_when));
            }
            out.push_str("tools: (chosen by LLM from catalog)\n");
        }
        out
    }

    /// 実行 trace から実際に使ったツール名列を表示用に返す。
    pub fn format_trace_tools_used(trace: &crate::action::TurnTrace) -> String {
        if trace.actions.is_empty() {
            return "(none)".into();
        }
        trace
            .actions
            .iter()
            .map(|a| a.tool.as_str())
            .collect::<Vec<_>>()
            .join(" → ")
    }

    /// サブタスクを実行ループ用 mission 文へ（必須実行順序を明示）。
    pub fn render_mission(
        &self,
        original: &str,
        plan: &PlanArtifact,
        subtask: &Subtask,
        progress: &PlanProgress,
    ) -> Result<String, TaskError> {
        let subtask_list = format_subtask_list(plan);

        let body = if let Some(task_id) = &subtask.task {
            let def = self
                .get(task_id)
                .ok_or_else(|| TaskError::UnknownTask { id: task_id.clone() })?;
            let mut merged = merge_params(&def.default_params, &subtask.params);
            ensure_goal_done_when(&mut merged, subtask);
            let mut block = def.format_required_execution(&merged);
            if !subtask.goal.is_empty() {
                block.push_str(&format!("\nContext goal: {}\n", subtask.goal));
            }
            block
        } else {
            format!(
                "Goal: {}\nDone when: {}\n",
                subtask.goal,
                subtask.done_when
            )
        };

        Ok(format!(
            "Original request:\n{original}\n\n\
             Plan summary: {}\n\n\
             All subtasks:\n{subtask_list}\n\
             Prior subtask results:\n{}\n\
             Current subtask: {}\n\n\
             {body}\n\
             Execute required methods in order, then answer. Do not replan.",
            plan.summary,
            progress.format_for_mission(),
            subtask.id,
        ))
    }

    /// 実行 trace がタスクの必須順序を満たすか照合する。
    pub fn audit_subtask(
        &self,
        subtask: &Subtask,
        trace: &TurnTrace,
    ) -> Option<TaskExecutionAudit> {
        let task_id = subtask.task.as_ref()?;
        let def = self.get(task_id)?;
        let params = merge_params(&def.default_params, &subtask.params);
        Some(audit_trace(def, &params, trace))
    }

    pub fn resolve_plan(&self, plan: &mut PlanArtifact) {
        for st in &mut plan.subtasks {
            if let Some(task_id) = &st.task {
                if let Some(def) = self.get(task_id) {
                    if st.goal.is_empty() {
                        st.goal = def.summary.clone();
                    }
                    if st.done_when.is_empty() && !def.done_when.is_empty() {
                        st.done_when = def.done_when.clone();
                    }
                    st.params = merge_params(&def.default_params, &st.params);
                }
            }
        }
    }

    pub fn load_dir(&mut self, dir: &Path) -> Result<(), TaskLoadError> {
        let mut paths: Vec<PathBuf> = fs::read_dir(dir)
            .map_err(|source| TaskLoadError::Read {
                path: dir.to_path_buf(),
                source,
            })?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|e| e == "json"))
            .collect();
        paths.sort();
        for path in paths {
            let text = fs::read_to_string(&path).map_err(|source| TaskLoadError::Read {
                path: path.clone(),
                source,
            })?;
            let def: TaskDefinition = serde_json::from_str(&text).map_err(|source| {
                TaskLoadError::Parse {
                    path: path.clone(),
                    source,
                }
            })?;
            if def.id.is_empty() {
                return Err(TaskLoadError::Invalid {
                    path: path.clone(),
                    reason: "missing id".into(),
                });
            }
            self.register(def).map_err(|e| TaskLoadError::Invalid {
                path: path.clone(),
                reason: e.to_string(),
            })?;
        }
        Ok(())
    }

    fn register_embedded(&mut self, json_text: &str) -> Result<(), TaskLoadError> {
        let def: TaskDefinition = serde_json::from_str(json_text).map_err(|source| {
            TaskLoadError::Parse {
                path: PathBuf::from("<embedded>"),
                source,
            }
        })?;
        if def.id.is_empty() {
            return Err(TaskLoadError::Invalid {
                path: PathBuf::from("<embedded>"),
                reason: "missing id".into(),
            });
        }
        self.register(def).map_err(|e| TaskLoadError::Invalid {
            path: PathBuf::from("<embedded>"),
            reason: e.to_string(),
        })
    }
}

pub(crate) fn merge_params(defaults: &Value, overrides: &Value) -> Value {
    let mut base = match defaults.as_object() {
        Some(m) => m.clone(),
        None => serde_json::Map::new(),
    };
    if let Some(over) = overrides.as_object() {
        for (k, v) in over {
            base.insert(k.clone(), v.clone());
        }
    }
    Value::Object(base)
}

fn ensure_goal_done_when(params: &mut Value, subtask: &Subtask) {
    let Some(map) = params.as_object_mut() else {
        return;
    };
    if !subtask.goal.is_empty() {
        map.insert("goal".into(), Value::String(subtask.goal.clone()));
    }
    if !subtask.done_when.is_empty() {
        map.insert(
            "done_when".into(),
            Value::String(subtask.done_when.clone()),
        );
    }
}

fn format_subtask_list(plan: &PlanArtifact) -> String {
    let mut out = String::new();
    for st in &plan.subtasks {
        let tag = st
            .task
            .as_deref()
            .map(|t| format!("task:{t}"))
            .unwrap_or_else(|| "freeform".into());
        out.push_str(&format!(
            "- id {} [{}]: {} (done when: {})\n",
            st.id, tag, st.goal, st.done_when
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{PlanArtifact, PlanProgress, Subtask};

    #[test]
    fn catalog_hides_web_research_when_disabled() {
        let reg = TaskRegistry::builtin();
        let off = reg.catalog_for_planner_opts(false);
        let on = reg.catalog_for_planner_opts(true);
        assert!(!off.contains("web_research"));
        assert!(on.contains("web_research"));
    }

    #[test]
    fn builtin_has_ordered_steps() {
        let reg = TaskRegistry::builtin();
        assert!(reg.get("web_research").is_some());
        let def = reg.get("write_file_verify").unwrap();
        let methods: Vec<_> = def
            .ordered_required_steps()
            .iter()
            .map(|s| s.method.as_str())
            .collect();
        assert_eq!(methods, vec!["write_file", "read_file"]);
    }

    #[test]
    fn render_mission_lists_required_order() {
        let reg = TaskRegistry::builtin();
        let plan = PlanArtifact {
            summary: "list".into(),
            skip_execution: false,
            subtasks: vec![Subtask {
                id: 1,
                task: Some("list_dir".into()),
                params: serde_json::json!({ "path": "src" }),
                goal: String::new(),
                done_when: String::new(),
            }],
        };
        let st = plan.subtasks[0].clone();
        let m = reg
            .render_mission("list files", &plan, &st, &PlanProgress::default())
            .unwrap();
        assert!(m.contains("Required execution order"));
        assert!(m.contains("1. list_dir"));
    }

    #[test]
    fn format_subtask_execution_shows_steps() {
        let reg = TaskRegistry::builtin();
        let sub = Subtask {
            id: 1,
            task: Some("list_dir".into()),
            params: serde_json::json!({ "path": "src" }),
            goal: String::new(),
            done_when: String::new(),
        };
        let text = reg.format_subtask_execution_for_display(&sub);
        assert!(text.contains("step-driver"));
        assert!(text.contains("1. list_dir"));
    }

    #[test]
    fn catalog_shows_method_chain() {
        let reg = TaskRegistry::builtin();
        let cat = reg.catalog_for_planner();
        assert!(cat.contains("write_file → read_file"));
    }
}
