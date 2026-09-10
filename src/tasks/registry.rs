use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::action::TurnTrace;
use crate::plan::{PlanArtifact, PlanProgress, Subtask};
use crate::tool::workspace_root;

use super::audit::{audit_trace, TaskExecutionAudit};
use super::policy::SubtaskToolPolicy;
use super::spec::{apply_template, apply_template_value, TaskDefinition, TaskError};

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
        self.catalog_for_planner_filtered(&HashSet::new(), include_web_research, &[], false)
    }

    /// 実行層に登録済みのツール名に基づき、計画可能な task id だけを載せる。
    /// `available_tools` が空かつ `require_all_tools` が false のときは全タスク（従来どおり）。
    pub fn catalog_for_planner_filtered(
        &self,
        available_tools: &HashSet<String>,
        include_web_research: bool,
        exclude_task_ids: &[&str],
        require_all_tools: bool,
    ) -> String {
        let filter_by_tools = require_all_tools || !available_tools.is_empty();
        let mut lines = vec![
            "Registered tasks for this session (use only task ids listed here):".into(),
        ];
        if filter_by_tools {
            let mut names: Vec<_> = available_tools.iter().map(String::as_str).collect();
            names.sort();
            lines.push(format!(
                "Available execution tools: {}",
                if names.is_empty() {
                    "(none)".into()
                } else {
                    names.join(", ")
                }
            ));
        }
        let mut ids: Vec<_> = self.tasks.keys().collect();
        ids.sort();
        for id in ids {
            if exclude_task_ids.contains(&id.as_str()) {
                continue;
            }
            if *id == "web_research" && !include_web_research {
                continue;
            }
            let def = &self.tasks[id];
            if filter_by_tools && !task_available_with_tools(def, available_tools) {
                continue;
            }
            let steps = def
                .ordered_required_steps()
                .iter()
                .map(|s| s.method.as_str())
                .collect::<Vec<_>>()
                .join(" → ");
            let steps_part = if steps.is_empty() {
                "(free execution — pick tools from catalog above)".into()
            } else {
                format!("required: {steps}")
            };
            lines.push(format!("- {id}: {} — {steps_part}", def.summary));
        }
        if lines.len() <= 1 {
            lines.push("- generic: (free execution)".into());
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

    /// サブタスクを実行ループ用 mission 文へ（**現在サブタスクのみ**を渡す）。
    pub fn render_mission(
        &self,
        original: &str,
        _plan: &PlanArtifact,
        subtask: &Subtask,
        progress: &PlanProgress,
    ) -> Result<String, TaskError> {
        let (body, include_user_reference, mission_append) = if let Some(task_id) = &subtask.task {
            let def = self
                .get(task_id)
                .ok_or_else(|| TaskError::UnknownTask { id: task_id.clone() })?;
            let mut merged = merge_params(&def.default_params, &subtask.params);
            ensure_goal_done_when(&mut merged, subtask);
            let mut block = def.format_required_execution(&merged);
            let policy = def.resolved_tool_policy();
            if !policy.allow.is_empty() || !policy.deny.is_empty() {
                block.push_str(&policy.format_for_mission());
            }
            if !def.mission_append.trim().is_empty() {
                block.push_str("\n");
                block.push_str(&apply_template(def.mission_append.trim(), &merged));
                block.push('\n');
            }
            (
                block,
                def.include_user_reference,
                String::new(),
            )
        } else {
            (
                format!(
                    "Goal: {}\nDone when: {}\n",
                    subtask.goal, subtask.done_when
                ),
                true,
                String::new(),
            )
        };

        let mut mission = format!(
            "## Subtask\n{}\n\n\
             ## Task contract\n{body}\n\n\
             ## Prior subtask results\n{}",
            format_subtask_node(subtask),
            progress.format_for_mission(),
        );

        if include_user_reference {
            let reference = strip_leading_system_block(original);
            if !reference.trim().is_empty() {
                mission.push_str("\n\n## User request (reference)\n");
                mission.push_str(reference.trim());
                mission.push('\n');
            }
        }

        if !mission_append.is_empty() {
            mission.push_str("\n\n");
            mission.push_str(mission_append.trim());
            mission.push('\n');
        }

        mission.push_str(
            "\nComplete ONLY this subtask. Execute required methods in order, then answer. \
             Do not replan or work ahead to other subtasks.",
        );

        Ok(mission)
    }

    /// サブタスク用の解決済みツールポリシー（`task` id があるときのみ）。
    pub fn merged_subtask_params(&self, subtask: &Subtask) -> Option<Value> {
        let task_id = subtask.task.as_ref()?;
        let def = self.get(task_id)?;
        Some(merge_params(&def.default_params, &subtask.params))
    }

    /// サブタスク用の解決済みツールポリシー（`task` id があるときのみ）。
    pub fn tool_policy_for_subtask(&self, subtask: &Subtask) -> Option<SubtaskToolPolicy> {
        if let Some(task_id) = subtask.task.as_ref() {
            let def = self.get(task_id)?;
            return Some(def.resolved_tool_policy());
        }

        let hinted_tool = hinted_tool_from_freeform_goal(&subtask.goal)?;
        Some(SubtaskToolPolicy {
            allow: vec![hinted_tool],
            deny: Vec::new(),
        })
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

    pub fn resolve_plan(
        &self,
        plan: &mut PlanArtifact,
        _user_input: &str,
        contract: Option<&crate::plan::PlanDataContract>,
    ) {
        let ref_uid = contract.and_then(|c| {
            if c.blocks_reference_fetch {
                None
            } else {
                c.reference_id
            }
        });

        if let Some(c) = contract {
            c.enforce_plan(plan);
        }

        for st in &mut plan.subtasks {
            let Some(task_id) = st.task.clone() else {
                inject_reference_id_freeform(st, ref_uid);
                continue;
            };
            let Some(def) = self.get(&task_id) else {
                // 未登録 task id（実行層ツール名の誤認など）→ 自由記述サブタスクへ
                let hint = format!(
                    "Execute with ReAct tools (not a registered task id): {task_id}"
                );
                st.goal = if st.goal.is_empty() {
                    hint
                } else {
                    format!("{hint}. {}", st.goal)
                };
                st.task = None;
                st.params = Value::Object(Default::default());
                inject_reference_id_freeform(st, ref_uid);
                continue;
            };
            if st.goal.is_empty() {
                st.goal = def.summary.clone();
            }
            if st.done_when.is_empty() && !def.done_when.is_empty() {
                st.done_when = def.done_when.clone();
            }
            st.params = merge_params(&def.default_params, &st.params);
            if task_needs_reference_id(def) {
                inject_reference_id_params(st, ref_uid);
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

fn task_needs_reference_id(def: &TaskDefinition) -> bool {
    def.default_params.get("uid").is_some()
        || def.ordered_required_steps().iter().any(|step| {
            step.args.to_string().contains("{uid}")
        })
}

fn inject_reference_id_params(subtask: &mut Subtask, ref_id: Option<i64>) {
    let Some(uid) = ref_id else {
        return;
    };
    // overrides（第2引数）が優先される。default uid:0 に上書きされないよう順序に注意。
    subtask.params = merge_params(&subtask.params, &serde_json::json!({ "uid": uid }));
}

fn task_available_with_tools(def: &TaskDefinition, available: &HashSet<String>) -> bool {
    let steps = def.ordered_required_steps();
    if steps.is_empty() {
        return true;
    }
    steps
        .iter()
        .all(|step| available.contains(step.method.as_str()))
}

fn inject_reference_id_freeform(subtask: &mut Subtask, ref_id: Option<i64>) {
    let Some(uid) = ref_id else {
        return;
    };
    if subtask.goal.contains("UID:") || subtask.goal.contains(&format!("uid {uid}")) {
        return;
    }
    subtask.goal = if subtask.goal.is_empty() {
        format!("Use reference id {uid} when the referenced item is needed.")
    } else {
        format!(
            "{} (Reference id: {uid}; use it with the appropriate tool if needed.)",
            subtask.goal
        )
    };
}

fn format_subtask_node(subtask: &Subtask) -> String {
    let task = subtask
        .task
        .as_deref()
        .unwrap_or("(freeform — no registered task id)");
    format!(
        "id: {}\ntask: {}\nparams: {}\ngoal: {}\ndone_when: {}",
        subtask.id, task, subtask.params, subtask.goal, subtask.done_when
    )
}

fn hinted_tool_from_freeform_goal(goal: &str) -> Option<String> {
    const MARKER: &str = "Execute with ReAct tools (not a registered task id):";
    let (_, rest) = goal.split_once(MARKER)?;
    let token = rest
        .trim_start()
        .split(|c: char| c.is_whitespace() || matches!(c, '.' | ',' | ';' | ')' | '('))
        .next()
        .unwrap_or("")
        .trim_matches(|c: char| matches!(c, '`' | '\'' | '"'));
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

/// 計画層向けヒントなど、先頭の `[システム…]` ブロックを除いたユーザ依頼本文。
fn strip_leading_system_block(text: &str) -> &str {
    let trimmed = text.trim_start();
    if trimmed.starts_with('[') {
        if let Some(rest) = trimmed.split_once("\n\n") {
            return rest.1.trim_start();
        }
    }
    trimmed
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
    fn render_mission_is_scoped_to_current_subtask_only() {
        let reg = TaskRegistry::builtin();
        let plan = PlanArtifact {
            summary: "end goal".into(),
            skip_execution: false,
            subtasks: vec![
                Subtask {
                    id: 1,
                    task: Some("list_dir".into()),
                    params: serde_json::json!({ "path": "src" }),
                    goal: "list".into(),
                    done_when: "listed".into(),
                                    depends_on: vec![],
},
                Subtask {
                    id: 2,
                    task: Some("write_file_verify".into()),
                    params: serde_json::json!({}),
                    goal: "write".into(),
                    done_when: "verified".into(),
                                    depends_on: vec![],
},
            ],
            knowledge_sufficient: None,
        };
        let st = plan.subtasks[0].clone();
        let m = reg
            .render_mission("user asks for much", &plan, &st, &PlanProgress::default())
            .unwrap();
        assert!(m.contains("id: 1"));
        assert!(!m.contains("id: 2"));
        assert!(!m.contains("end goal"));
        assert!(!m.contains("All subtasks"));
        assert!(!m.contains("write_file_verify"));
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
                            depends_on: vec![],
}],
            knowledge_sufficient: None,
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
                    depends_on: vec![],
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

    #[test]
    fn resolve_plan_strips_unknown_task_id_as_freeform() {
        let reg = TaskRegistry::builtin();
        let mut plan = PlanArtifact {
            summary: "work".into(),
            skip_execution: false,
            subtasks: vec![Subtask {
                id: 1,
                task: Some("not_a_registered_task".into()),
                params: serde_json::json!({}),
                goal: "do it".into(),
                done_when: "done".into(),
                            depends_on: vec![],
}],
            knowledge_sufficient: None,
        };
        reg.resolve_plan(&mut plan, "user input", None);
        let st = &plan.subtasks[0];
        assert!(st.task.is_none());
        assert!(st.goal.contains("not_a_registered_task"));
        assert!(st.goal.contains("do it"));

        let policy = reg.tool_policy_for_subtask(st).expect("freeform hinted policy");
        assert_eq!(policy.allow, vec!["not_a_registered_task".to_string()]);
    }

    #[test]
    fn resolve_plan_injects_reference_id_into_task_params() {
        let mut reg = TaskRegistry::default();
        let def: TaskDefinition = serde_json::from_str(
            r#"{
                "id": "fetch_item",
                "summary": "fetch by uid",
                "default_params": { "uid": 0 },
                "steps": [
                    { "order": 1, "method": "read_file", "args": { "path": "{uid}" }, "required": true }
                ]
            }"#,
        )
        .unwrap();
        reg.register(def).unwrap();
        let mut plan = PlanArtifact {
            summary: "fetch".into(),
            skip_execution: false,
            subtasks: vec![Subtask {
                id: 1,
                task: Some("fetch_item".into()),
                params: serde_json::json!({}),
                goal: String::new(),
                done_when: String::new(),
                            depends_on: vec![],
}],
            knowledge_sufficient: None,
        };
        let contract = crate::plan::PlanDataContract::new(
            "read: item",
            "write: chat",
            "fetch_item",
        )
        .with_reference_id(Some(42));
        reg.resolve_plan(&mut plan, "user", Some(&contract));
        assert_eq!(plan.subtasks[0].params["uid"], 42);
    }

    #[test]
    fn resolve_plan_host_enforce_runs_before_param_merge() {
        let mut reg = TaskRegistry::default();
        let def: TaskDefinition = serde_json::from_str(
            r#"{
                "id": "save_item",
                "summary": "save",
                "default_params": {},
                "steps": [
                    { "order": 1, "method": "write_file", "args": { "path": "x" }, "required": true }
                ]
            }"#,
        )
        .unwrap();
        reg.register(def).unwrap();
        let mut plan = PlanArtifact {
            summary: "x".into(),
            skip_execution: false,
            subtasks: vec![
                Subtask {
                    id: 1,
                    task: Some("fetch_item".into()),
                    params: serde_json::json!({}),
                    goal: "load".into(),
                    done_when: "loaded".into(),
                                    depends_on: vec![],
},
                Subtask {
                    id: 2,
                    task: Some("save_item".into()),
                    params: serde_json::json!({}),
                    goal: "persist changes".into(),
                    done_when: "saved".into(),
                                    depends_on: vec![],
},
            ],
            knowledge_sufficient: None,
        };
        let contract = crate::plan::PlanDataContract::new("in", "out", "save_item").with_enforce(
            |plan| {
                let goals: Vec<String> = plan
                    .subtasks
                    .iter()
                    .filter(|st| st.task.as_deref() != Some("fetch_item"))
                    .map(|st| st.goal.clone())
                    .collect();
                plan.subtasks = vec![Subtask {
                    id: 1,
                    task: Some("save_item".into()),
                    params: serde_json::json!({ "id": 9 }),
                    goal: goals.join(" → "),
                    done_when: "saved".into(),
                                    depends_on: vec![],
}];
            },
        );
        reg.resolve_plan(&mut plan, "user", Some(&contract));
        assert_eq!(plan.subtasks.len(), 1);
        assert_eq!(plan.subtasks[0].task.as_deref(), Some("save_item"));
        assert_eq!(plan.subtasks[0].params["id"], 9);
        assert!(plan.subtasks[0].goal.contains("persist"));
    }

    #[test]
    fn resolve_plan_blocks_reference_fetch_skips_injection() {
        let mut reg = TaskRegistry::default();
        let def: TaskDefinition = serde_json::from_str(
            r#"{
                "id": "fetch_item",
                "summary": "fetch by uid",
                "default_params": { "uid": 0 },
                "steps": []
            }"#,
        )
        .unwrap();
        reg.register(def).unwrap();
        let mut plan = PlanArtifact {
            summary: "fetch".into(),
            skip_execution: false,
            subtasks: vec![Subtask {
                id: 1,
                task: Some("fetch_item".into()),
                params: serde_json::json!({}),
                goal: String::new(),
                done_when: String::new(),
                            depends_on: vec![],
}],
            knowledge_sufficient: None,
        };
        let contract = crate::plan::PlanDataContract::new("in", "out", "fetch_item")
            .with_reference_id(Some(42))
            .with_blocks_reference_fetch(true);
        reg.resolve_plan(&mut plan, "user", Some(&contract));
        assert_eq!(plan.subtasks[0].params["uid"], 0);
    }
}
