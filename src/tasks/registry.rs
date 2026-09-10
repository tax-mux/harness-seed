mod catalog;

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::action::TurnTrace;
use crate::plan::{
    is_reserved_control_task, strengthen_weak_done_when, PlanArtifact, PlanProgress, Subtask,
};
use crate::tool::workspace_root;

use super::audit::{audit_trace_with_mode, ArgAuditMode, TaskExecutionAudit};
use super::policy::SubtaskToolPolicy;
use super::spec::{apply_template, TaskDefinition, TaskError};

/// 組み込みタスク JSON（`tasks/` ディレクトリと同期すること）。
const BUILTIN_LIST_DIR: &str = include_str!("../../tasks/list_dir.json");
const BUILTIN_GENERIC: &str = include_str!("../../tasks/generic.json");
const BUILTIN_WRITE_FILE_VERIFY: &str = include_str!("../../tasks/write_file_verify.json");
const BUILTIN_WEB_RESEARCH: &str = include_str!("../../tasks/web_research.json");

#[derive(Debug)]
pub enum TaskLoadError {
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    Parse {
        path: PathBuf,
        source: serde_json::Error,
    },
    Invalid {
        path: PathBuf,
        reason: String,
    },
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
        reg.register_embedded(BUILTIN_LIST_DIR)
            .expect("list_dir.json");
        reg.register_embedded(BUILTIN_GENERIC)
            .expect("generic.json");
        reg.register_embedded(BUILTIN_WRITE_FILE_VERIFY)
            .expect("write_file_verify.json");
        reg.register_embedded(BUILTIN_WEB_RESEARCH)
            .expect("web_research.json");
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
        def.validate_definition()
            .map_err(|reason| TaskError::InvalidDefinition {
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
    pub fn format_subtask_execution_for_display(&self, subtask: &Subtask) -> String {
        let mut out = String::new();
        if let Some(task_id) = &subtask.task {
            if is_reserved_control_task(task_id) {
                out.push_str(&format!(
                    "task: {task_id} — control-plane (harness restarts planning)\n"
                ));
                out.push_str(&format!("goal: {}\n", subtask.goal));
                if !subtask.done_when.is_empty() {
                    out.push_str(&format!("done_when: {}\n", subtask.done_when));
                }
                out.push_str("run: plan-layer replan (not an exec tool)\n");
                return out;
            }
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
            let def = self.get(task_id).ok_or_else(|| TaskError::UnknownTask {
                id: task_id.clone(),
            })?;
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
            (block, def.include_user_reference, String::new())
        } else {
            (
                format!("Goal: {}\nDone when: {}\n", subtask.goal, subtask.done_when),
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
             Do not invent control-plane actions (e.g. a replan tool) or work ahead to other subtasks.",
        );
        if !progress.results.is_empty() {
            mission.push('\n');
            mission.push_str(crate::advance::evidence_grounding_rules());
        }

        Ok(mission)
    }

    /// サブタスク用の解決済みツールポリシー（`task` id があるときのみ）。
    pub fn merged_subtask_params(&self, subtask: &Subtask) -> Option<Value> {
        let task_id = subtask.task.as_ref()?;
        let def = self.get(task_id)?;
        Some(merge_params(&def.default_params, &subtask.params))
    }

    /// サブタスク用の解決済みツールポリシー。
    ///
    /// 自由記述 goal から取り出したヒントは、実在するツール名のときだけ allow に使う。
    /// 制御プレーン id や未知トークンを単独 allow にするとカタログが空になるため拒否する。
    pub fn tool_policy_for_subtask(&self, subtask: &Subtask) -> Option<SubtaskToolPolicy> {
        self.tool_policy_for_subtask_with_tools(subtask, None)
    }

    pub fn tool_policy_for_subtask_with_tools(
        &self,
        subtask: &Subtask,
        available_tools: Option<&HashSet<String>>,
    ) -> Option<SubtaskToolPolicy> {
        if let Some(task_id) = subtask.task.as_ref() {
            if is_reserved_control_task(task_id) {
                return None;
            }
            let def = self.get(task_id)?;
            return Some(def.resolved_tool_policy());
        }

        let hinted_tool = hinted_tool_from_freeform_goal(&subtask.goal)?;
        let Some(available) = available_tools else {
            // ツール集合が無いときは phantom allow を作らない（全カタログ）。
            return None;
        };
        if !available.contains(&hinted_tool) {
            return None;
        }
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
        self.audit_subtask_with_mode(subtask, trace, ArgAuditMode::Soft)
    }

    pub fn audit_subtask_with_mode(
        &self,
        subtask: &Subtask,
        trace: &TurnTrace,
        arg_mode: ArgAuditMode,
    ) -> Option<TaskExecutionAudit> {
        let task_id = subtask.task.as_ref()?;
        let def = self.get(task_id)?;
        let params = merge_params(&def.default_params, &subtask.params);
        Some(audit_trace_with_mode(def, &params, trace, arg_mode))
    }

    /// 必須ステップの method が利用可能ツールに無いタスクを列挙する。
    pub fn tasks_missing_tools(
        &self,
        available_tools: &HashSet<String>,
    ) -> Vec<(String, Vec<String>)> {
        let mut out = Vec::new();
        let mut ids: Vec<_> = self.tasks.keys().cloned().collect();
        ids.sort();
        for id in ids {
            let def = &self.tasks[&id];
            let missing = missing_required_tools(def, available_tools);
            if !missing.is_empty() {
                out.push((id, missing));
            }
        }
        out
    }

    pub fn resolve_plan(
        &self,
        plan: &mut PlanArtifact,
        user_input: &str,
        contract: Option<&crate::plan::PlanDataContract>,
    ) {
        self.resolve_plan_with_tools(plan, user_input, contract, None);
    }

    /// `available_tools` があるとき、必須 method が欠ける登録タスクは自由記述へ落とす。
    pub fn resolve_plan_with_tools(
        &self,
        plan: &mut PlanArtifact,
        _user_input: &str,
        contract: Option<&crate::plan::PlanDataContract>,
        available_tools: Option<&HashSet<String>>,
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
                strengthen_weak_done_when(st);
                continue;
            };
            let Some(def) = self.get(&task_id) else {
                if is_reserved_control_task(&task_id) {
                    if st.goal.trim().is_empty() {
                        st.goal = "Revise remaining work based on completed phases.".into();
                    }
                    continue;
                }
                // 未登録 task id（実行層ツール名の誤認など）→ 自由記述サブタスクへ
                demote_to_freeform_unknown_task(st, &task_id);
                inject_reference_id_freeform(st, ref_uid);
                strengthen_weak_done_when(st);
                continue;
            };
            if let Some(available) = available_tools {
                let missing = missing_required_tools(def, available);
                if !missing.is_empty() {
                    demote_to_freeform_missing_tools(st, &task_id, &missing);
                    inject_reference_id_freeform(st, ref_uid);
                    strengthen_weak_done_when(st);
                    continue;
                }
            }
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
            strengthen_weak_done_when(st);
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
            let def: TaskDefinition =
                serde_json::from_str(&text).map_err(|source| TaskLoadError::Parse {
                    path: path.clone(),
                    source,
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
        let def: TaskDefinition =
            serde_json::from_str(json_text).map_err(|source| TaskLoadError::Parse {
                path: PathBuf::from("<embedded>"),
                source,
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
        map.insert("done_when".into(), Value::String(subtask.done_when.clone()));
    }
}

fn task_needs_reference_id(def: &TaskDefinition) -> bool {
    def.default_params.get("uid").is_some()
        || def
            .ordered_required_steps()
            .iter()
            .any(|step| step.args.to_string().contains("{uid}"))
}

fn inject_reference_id_params(subtask: &mut Subtask, ref_id: Option<i64>) {
    let Some(uid) = ref_id else {
        return;
    };
    // overrides（第2引数）が優先される。default uid:0 に上書きされないよう順序に注意。
    subtask.params = merge_params(&subtask.params, &serde_json::json!({ "uid": uid }));
}

fn task_available_with_tools(def: &TaskDefinition, available: &HashSet<String>) -> bool {
    missing_required_tools(def, available).is_empty()
}

fn missing_required_tools(def: &TaskDefinition, available: &HashSet<String>) -> Vec<String> {
    let mut missing = Vec::new();
    for step in def.ordered_required_steps() {
        if !available.contains(step.method.as_str()) && !missing.contains(&step.method) {
            missing.push(step.method.clone());
        }
    }
    missing
}

fn demote_to_freeform_unknown_task(subtask: &mut Subtask, task_id: &str) {
    let hint = format!("Execute with ReAct tools (not a registered task id): {task_id}");
    subtask.goal = if subtask.goal.is_empty() {
        hint
    } else {
        format!("{hint}. {}", subtask.goal)
    };
    subtask.task = None;
    subtask.params = Value::Object(Default::default());
}

fn demote_to_freeform_missing_tools(subtask: &mut Subtask, task_id: &str, missing: &[String]) {
    let hint = format!(
        "Task '{task_id}' requires unavailable tools: {}. Execute with available ReAct tools.",
        missing.join(", ")
    );
    subtask.goal = if subtask.goal.is_empty() {
        hint
    } else {
        format!("{hint} {}", subtask.goal)
    };
    subtask.task = None;
    subtask.params = Value::Object(Default::default());
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
mod tests;
