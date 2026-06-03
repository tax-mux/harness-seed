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
                block.push_str(def.mission_append.trim());
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
        user_input: &str,
        contract: Option<&crate::plan::PlanDataContract>,
    ) {
        let ref_uid = contract
            .and_then(|c| c.imap_reference_uid())
            .or_else(|| {
                if contract.is_some_and(|c| c.blocks_imap_mail_read()) {
                    None
                } else {
                    extract_reference_uid(user_input)
                }
            });
        let outgoing_pending_ref = contract
            .map(|c| c.is_outgoing_pending_revision())
            .unwrap_or_else(|| has_outgoing_pending_reference(user_input));

        for st in &mut plan.subtasks {
            normalize_planner_task_id(st);
        }

        if let Some(c) = contract {
            c.enforce_plan(plan);
        } else if outgoing_pending_ref {
            normalize_plan_for_outgoing_pending_draft(plan);
        }

        // 参照メールが無いときは、compose_context が持つ default_params(uid:0) と get_email 契約を避ける。
        // LLM は常に compose_context を出しがちなので、ここで置換して不要な get_email 呼び出しをカットする。
        if ref_uid.is_none() {
            for st in &mut plan.subtasks {
                if st.task.as_deref() == Some("compose_context") {
                    st.task = Some("compose_context_no_ref".into());
                    st.params = Value::Object(Default::default());
                }
            }
        }

        for st in &mut plan.subtasks {
            let Some(task_id) = st.task.clone() else {
                inject_reference_uid_freeform(st, ref_uid);
                continue;
            };
            let Some(def) = self.get(&task_id) else {
                // triage-mail の compose 系は `.triage-mail/tasks` 未読込でも task id を維持する
                if matches!(
                    task_id.as_str(),
                    "compose_context"
                        | "compose_context_no_ref"
                        | "compose_write"
                        | "mail_read"
                        | "pending_outgoing_save"
                ) {
                    if matches!(task_id.as_str(), "compose_context" | "mail_read") {
                        inject_reference_uid_params(st, ref_uid);
                    }
                    if st.goal.is_empty() {
                        st.goal = format!("Execute triage task: {task_id}");
                    }
                    continue;
                }
                // LLM が実行層ツール名を task id と誤認した場合 → 自由記述サブタスクへ
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
                inject_reference_uid_freeform(st, ref_uid);
                continue;
            };
            if st.goal.is_empty() {
                st.goal = def.summary.clone();
            }
            if st.done_when.is_empty() && !def.done_when.is_empty() {
                st.done_when = def.done_when.clone();
            }
            st.params = merge_params(&def.default_params, &st.params);
            if task_needs_reference_uid(def) {
                inject_reference_uid_params(st, ref_uid);
            }
        }

        if let Some(uid) = ref_uid {
            for st in &mut plan.subtasks {
                if st.task.as_deref() == Some("compose_context") {
                    inject_reference_uid_params(st, Some(uid));
                }
            }
            if !outgoing_pending_ref && !contract.is_some_and(|c| c.blocks_imap_mail_read()) {
                ensure_mail_read_subtask(plan, user_input, uid);
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

/// `UID: 123` / `UID：123` 行をパースする。
fn parse_uid_line(line: &str) -> Option<i64> {
    let line = line.trim();
    let rest = line
        .strip_prefix("UID:")
        .or_else(|| line.strip_prefix("UID："))?;
    let uid: i64 = rest.trim().parse().ok()?;
    (uid > 0).then_some(uid)
}

/// テキスト中の `UID:` 行を出現順に列挙する。
fn uids_in_text(text: &str) -> Vec<i64> {
    text.lines().filter_map(parse_uid_line).collect()
}

fn reference_block_uid_if_inbox(block: &str) -> Option<i64> {
    let is_outgoing = block.lines().any(|line| {
        let trimmed = line.trim();
        (trimmed.starts_with("種別:") || trimmed.starts_with("種別："))
            && trimmed.contains("送信")
    });
    if is_outgoing {
        return None;
    }
    uids_in_text(block).last().copied()
}

/// 最後の `【参照情報】` ブロック内の UID（マルチターン履歴向け）。
fn last_uid_in_reference_mail_blocks(text: &str) -> Option<i64> {
    const MARKER: &str = "【参照情報】";
    let normalized = text.replace("【参照メール】", MARKER);
    let mut last = None;
    for block in normalized.split(MARKER).skip(1) {
        if let Some(uid) = reference_block_uid_if_inbox(block) {
            last = Some(uid);
        }
    }
    last
}

/// LLM が実行層ツール名を task id と誤認したときの補正。
fn normalize_planner_task_id(subtask: &mut Subtask) {
    let Some(task_id) = subtask.task.clone() else {
        return;
    };
    let mapped = match task_id.as_str() {
        "get_compose_form" => Some("compose_context"),
        "set_compose_form" => Some("compose_write"),
        "get_email" => Some("mail_read"),
        _ => None,
    };
    if let Some(id) = mapped {
        subtask.task = Some(id.into());
    }
}

fn task_needs_reference_uid(def: &TaskDefinition) -> bool {
    def.id == "compose_context"
        || def.ordered_required_steps().iter().any(|step| {
            step.args
                .to_string()
                .contains("{uid}")
        })
}

fn inject_reference_uid_params(subtask: &mut Subtask, ref_uid: Option<i64>) {
    let Some(uid) = ref_uid else {
        return;
    };
    // overrides（第2引数）が優先される。compose_context の default uid:0 に上書きされないよう順序に注意。
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

fn subtask_fetches_reference_mail(st: &Subtask) -> bool {
    matches!(
        st.task.as_deref(),
        Some("mail_read" | "compose_context")
    )
}

/// 送信待ち改訂（sent メニュー）: 計画を pending_outgoing_save 1 件に正規化する。
fn normalize_plan_for_outgoing_pending_draft(plan: &mut PlanArtifact) {
    if plan.skip_execution {
        return;
    }
    const SKIP_GOAL_FROM: &[&str] = &["mail_read", "compose_context", "web_research"];
    let goals: Vec<String> = plan
        .subtasks
        .iter()
        .filter(|st| !st.task.as_deref().is_some_and(|t| SKIP_GOAL_FROM.contains(&t)))
        .map(|st| st.goal.clone())
        .filter(|g| !g.trim().is_empty())
        .collect();
    let goal = if goals.is_empty() {
        "【改訂コンテキスト】を基準に改訂し、save_pending_outgoing_mail で送信待ちキューへ保存する".into()
    } else {
        goals.join(" → ")
    };
    plan.subtasks = vec![Subtask {
        id: 1,
        task: Some("pending_outgoing_save".into()),
        params: serde_json::json!({}),
        goal,
        done_when: "save_pending_outgoing_mail 成功".into(),
    }];
}

fn effective_user_request(text: &str) -> &str {
    if let Some((_, rest)) = text.rsplit_once("Current user request:") {
        rest.trim_start()
    } else {
        strip_leading_system_block(text)
    }
}

fn looks_like_compose_request(text: &str) -> bool {
    let segment = effective_user_request(text);
    let t = segment.to_lowercase();
    if t.trim().is_empty() {
        return false;
    }
    if wants_summary_or_analysis(segment) {
        return false;
    }
    const PHRASES: &[&str] = &[
        "メールを書",
        "メール書",
        "メール作成",
        "文案",
        "ドラフト",
        "下書き",
        "返信案",
        "返信文",
        "返信メール",
        "返信を書",
        "返信して",
        "compose",
        "draft email",
        "write a reply",
        "write an email",
    ];
    if PHRASES.iter().any(|p| t.contains(p)) {
        return true;
    }
    if t.contains("メール") && (t.contains("書い") || t.contains("作成") || t.contains("起草")) {
        return true;
    }
    if (t.contains("返信") || t.contains("reply")) && t.contains("書") {
        return true;
    }
    const TRANSFORM: &[&str] = &[
        "日本語",
        "英語",
        "中国語",
        "翻訳",
        "translate",
        "カジュアル",
        "トーン",
    ];
    if TRANSFORM.iter().any(|p| t.contains(p)) {
        return true;
    }
    (t.contains("参照情報") || t.contains("参照メール"))
        && (t.contains('書') || t.contains("文案") || t.contains("返信") || t.contains("起草"))
}

fn has_outgoing_pending_reference(text: &str) -> bool {
    text.contains("種別: 送信待ちメール")
        || (text.contains("【改訂コンテキスト】") && text.contains("kind: outgoing_pending"))
}

fn wants_summary_or_analysis(text: &str) -> bool {
    let t = text.to_lowercase();
    const PHRASES: &[&str] = &[
        "要約",
        "サマリ",
        "summarize",
        "summary",
        "要点",
        "概要を",
        "内容を教え",
        "何が書いて",
        "読んで説明",
        "読んで教え",
        "分析して",
        "解説して",
        "箇条書きで",
        "リストアップ",
    ];
    PHRASES.iter().any(|p| t.contains(p))
}

/// 要約・Q&A で参照情報があるのに `get_email` ステップが無い計画へ `mail_read` を先頭挿入する。
fn ensure_mail_read_subtask(plan: &mut PlanArtifact, user_input: &str, ref_uid: i64) {
    if plan.skip_execution || plan.subtasks.is_empty() {
        return;
    }
    if !contains_reference_marker(user_input) {
        return;
    }
    if looks_like_compose_request(user_input) {
        return;
    }
    if plan.subtasks.iter().any(subtask_fetches_reference_mail) {
        return;
    }
    for st in &mut plan.subtasks {
        st.id = st.id.saturating_add(1);
    }
    plan.subtasks.insert(
        0,
        Subtask {
            id: 1,
            task: Some("mail_read".into()),
            params: serde_json::json!({ "uid": ref_uid }),
            goal: "get_email で参照情報の全文を確認する".into(),
            done_when: "get_email が成功した".into(),
        },
    );
}

fn inject_reference_uid_freeform(subtask: &mut Subtask, ref_uid: Option<i64>) {
    let Some(uid) = ref_uid else {
        return;
    };
    if subtask.goal.contains("UID:") || subtask.goal.contains(&format!("uid {uid}")) {
        return;
    }
    subtask.goal = if subtask.goal.is_empty() {
        format!("Use get_email with uid {uid} when the referenced mail is needed.")
    } else {
        format!(
            "{} (Referenced mail UID: {uid}; use get_email with this uid if needed.)",
            subtask.goal
        )
    };
}

/// `【参照情報】` / `UID: 123` 形式から参照 UID を抜く（triage-mail チャット添付向け）。
///
/// 会話履歴に複数メールがあるとき、先頭ではなく **今回の依頼** に紐づく UID を優先する。
pub fn extract_reference_uid(text: &str) -> Option<i64> {
    if let Some((_, rest)) = text.rsplit_once("Current user request:") {
        if let Some(uid) = last_uid_in_reference_mail_blocks(rest) {
            return Some(uid);
        }
        // 今回の依頼に UID が無いときだけ履歴側の参照情報へフォールバック
        let head = text.split("Current user request:").next().unwrap_or("");
        if let Some(uid) = last_uid_in_reference_mail_blocks(head) {
            return Some(uid);
        }
    }
    if let Some(uid) = last_uid_in_reference_mail_blocks(text) {
        return Some(uid);
    }
    if contains_reference_marker(text) {
        return None;
    }
    uids_in_text(text).last().copied()
}

fn contains_reference_marker(text: &str) -> bool {
    text.contains("【参照情報】") || text.contains("【参照メール】")
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
                },
                Subtask {
                    id: 2,
                    task: Some("write_file_verify".into()),
                    params: serde_json::json!({}),
                    goal: "write".into(),
                    done_when: "verified".into(),
                },
            ],
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

    #[test]
    fn extract_reference_uid_prefers_current_user_request() {
        let input = "\
Assistant: hello

User: 要約して

【参照情報】
UID: 302562
件名: old

Current user request:
お詫びメールを書いて

【参照情報】
UID: 302023
件名: new
";
        assert_eq!(extract_reference_uid(input), Some(302023));
    }

    #[test]
    fn extract_reference_uid_uses_last_reference_mail_block_without_current_marker() {
        let input = "\
User: 先のメール

【参照情報】
UID: 302562
件名: A

User: 次

【参照情報】
UID: 302023
件名: B
";
        assert_eq!(extract_reference_uid(input), Some(302023));
    }

    #[test]
    fn extract_reference_uid_single_mail() {
        assert_eq!(
            extract_reference_uid("【参照情報】\nUID: 42\n件名: x"),
            Some(42)
        );
    }

    #[test]
    fn extract_reference_uid_ignores_outgoing_pending_reference_block() {
        let input = "\
Current user request:
もうちょっと膨らませて。

【参照情報】
種別: 送信待ちメール
UID: 7
件名: テスト
";
        assert_eq!(extract_reference_uid(input), None);
    }

    #[test]
    fn resolve_plan_injects_uid_into_compose_context_from_current_request() {
        let reg = TaskRegistry::builtin();
        let mut plan = PlanArtifact {
            summary: "compose".into(),
            skip_execution: false,
            subtasks: vec![
                Subtask {
                    id: 1,
                    task: Some("compose_context".into()),
                    params: serde_json::json!({}),
                    goal: String::new(),
                    done_when: String::new(),
                },
                Subtask {
                    id: 2,
                    task: Some("compose_write".into()),
                    params: serde_json::json!({}),
                    goal: String::new(),
                    done_when: String::new(),
                },
            ],
        };
        let input = "\
User: 古い依頼

【参照情報】
UID: 302562

Current user request:
お詫び

【参照情報】
UID: 302023
";
        reg.resolve_plan(&mut plan, input, None);
        assert_eq!(plan.subtasks[0].params["uid"], 302023);
        assert!(plan.subtasks[1].params.get("uid").is_none());
    }

    #[test]
    fn resolve_plan_injects_uid_over_compose_context_default_zero() {
        let mut reg = TaskRegistry::default();
        let def: TaskDefinition = serde_json::from_str(
            r#"{
                "id": "compose_context",
                "summary": "ctx",
                "default_params": { "uid": 0 },
                "steps": [
                    { "order": 1, "method": "get_compose_form", "args": {}, "required": true },
                    { "order": 2, "method": "get_email", "args": { "uid": "{uid}" }, "required": true }
                ]
            }"#,
        )
        .unwrap();
        reg.register(def).unwrap();
        let mut plan = PlanArtifact {
            summary: "c".into(),
            skip_execution: false,
            subtasks: vec![Subtask {
                id: 1,
                task: Some("compose_context".into()),
                params: serde_json::json!({}),
                goal: String::new(),
                done_when: String::new(),
            }],
        };
        reg.resolve_plan(&mut plan, "【参照情報】\nUID: 302711\n", None);
        assert_eq!(plan.subtasks[0].params["uid"], 302711);
    }

    #[test]
    fn resolve_plan_maps_get_compose_form_to_compose_context_and_injects_uid() {
        let reg = TaskRegistry::builtin();
        let mut plan = PlanArtifact {
            summary: "compose".into(),
            skip_execution: false,
            subtasks: vec![Subtask {
                id: 1,
                task: Some("get_compose_form".into()),
                params: serde_json::json!({}),
                goal: String::new(),
                done_when: String::new(),
            }],
        };
        reg.resolve_plan(&mut plan, "【参照情報】\nUID: 99\n", None);
        assert_eq!(plan.subtasks[0].task.as_deref(), Some("compose_context"));
        assert_eq!(plan.subtasks[0].params["uid"], 99);
    }

    #[test]
    fn extract_reference_uid_falls_back_to_history_before_current_request() {
        let input = "\
User: 古い

【参照情報】
UID: 302562

Current user request:
お詫びメールを書いて
";
        assert_eq!(extract_reference_uid(input), Some(302562));
    }

    #[test]
    fn resolve_plan_replaces_compose_context_with_no_ref_when_no_reference_uid() {
        let reg = TaskRegistry::builtin();
        let mut plan = PlanArtifact {
            summary: "compose".into(),
            skip_execution: false,
            subtasks: vec![Subtask {
                id: 1,
                task: Some("compose_context".into()),
                params: serde_json::json!({}),
                goal: String::new(),
                done_when: String::new(),
            }],
        };

        // 参照メールブロックが無いケース → compose_context は compose_context_no_ref に置換される
        reg.resolve_plan(&mut plan, "Current user request:\n自分宛てテストメールを書く\n", None);
        assert_eq!(plan.subtasks[0].task.as_deref(), Some("compose_context_no_ref"));
        assert!(plan.subtasks[0].params.as_object().unwrap().is_empty());
    }

    fn register_mail_read(reg: &mut TaskRegistry) {
        let def: TaskDefinition = serde_json::from_str(
            r#"{
                "id": "mail_read",
                "summary": "参照メールを get_email で全文確認",
                "default_params": { "uid": 0 },
                "steps": [
                    { "order": 1, "method": "get_email", "args": { "uid": "{uid}" }, "required": true }
                ]
            }"#,
        )
        .unwrap();
        reg.register(def).unwrap();
    }

    #[test]
    fn ensure_mail_read_prepended_for_summarize_with_reference() {
        let mut reg = TaskRegistry::builtin();
        register_mail_read(&mut reg);
        let mut plan = PlanArtifact {
            summary: "要約".into(),
            skip_execution: false,
            subtasks: vec![Subtask {
                id: 1,
                task: Some("generic".into()),
                params: serde_json::json!({}),
                goal: "要約を返す".into(),
                done_when: "回答した".into(),
            }],
        };
        reg.resolve_plan(&mut plan, "Current user request:\n要約して\n\n【参照情報】\nUID: 42\n", None);
        assert_eq!(plan.subtasks.len(), 2);
        assert_eq!(plan.subtasks[0].task.as_deref(), Some("mail_read"));
        assert_eq!(plan.subtasks[0].params["uid"], 42);
        assert_eq!(plan.subtasks[1].id, 2);
    }

    #[test]
    fn ensure_mail_read_skipped_for_compose_with_compose_context() {
        let mut reg = TaskRegistry::default();
        let def: TaskDefinition = serde_json::from_str(
            r#"{
                "id": "compose_context",
                "summary": "ctx",
                "default_params": { "uid": 0 },
                "steps": [
                    { "order": 1, "method": "get_compose_form", "args": {}, "required": true },
                    { "order": 2, "method": "get_email", "args": { "uid": "{uid}" }, "required": true }
                ]
            }"#,
        )
        .unwrap();
        reg.register(def).unwrap();
        let mut plan = PlanArtifact {
            summary: "compose".into(),
            skip_execution: false,
            subtasks: vec![Subtask {
                id: 1,
                task: Some("compose_context".into()),
                params: serde_json::json!({}),
                goal: String::new(),
                done_when: String::new(),
            }],
        };
        reg.resolve_plan(
            &mut plan,
            "Current user request:\nお詫びメールを書いて\n\n【参照情報】\nUID: 99\n",
            None,
        );
        assert_eq!(plan.subtasks.len(), 1);
        assert_eq!(plan.subtasks[0].task.as_deref(), Some("compose_context"));
    }

    #[test]
    fn resolve_plan_maps_get_email_to_mail_read() {
        let mut reg = TaskRegistry::builtin();
        register_mail_read(&mut reg);
        let mut plan = PlanArtifact {
            summary: "read".into(),
            skip_execution: false,
            subtasks: vec![Subtask {
                id: 1,
                task: Some("get_email".into()),
                params: serde_json::json!({}),
                goal: String::new(),
                done_when: String::new(),
            }],
        };
        reg.resolve_plan(&mut plan, "【参照情報】\nUID: 7\n", None);
        assert_eq!(plan.subtasks[0].task.as_deref(), Some("mail_read"));
        assert_eq!(plan.subtasks[0].params["uid"], 7);
    }

    #[test]
    fn catalog_for_planner_filters_compose_when_tools_missing() {
        let mut reg = TaskRegistry::builtin();
        register_mail_read(&mut reg);
        let triage_only: HashSet<String> = [
            "fetch_mails",
            "list_emails",
            "get_email",
            "classify_email",
            "set_email_category",
            "list_categories",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        let cat = reg.catalog_for_planner_filtered(&triage_only, false, &[], true);
        assert!(cat.contains("mail_read"));
        assert!(cat.contains("generic"));
        assert!(!cat.contains("- compose_write:"));
        assert!(!cat.contains("- compose_context:"));
    }

    #[test]
    fn resolve_plan_strips_mail_read_for_outgoing_pending_reference() {
        let mut reg = TaskRegistry::default();
        register_mail_read(&mut reg);
        let mut plan = PlanArtifact {
            summary: "revise outgoing".into(),
            skip_execution: false,
            subtasks: vec![
                Subtask {
                    id: 1,
                    task: Some("mail_read".into()),
                    params: serde_json::json!({}),
                    goal: "送信待ちメール（UID: 9）の全文を読み込む".into(),
                    done_when: "get_email 成功".into(),
                },
                Subtask {
                    id: 2,
                    task: None,
                    params: serde_json::json!({}),
                    goal: "日本語に翻訳".into(),
                    done_when: "文案提示".into(),
                },
            ],
        };
        reg.resolve_plan(
            &mut plan,
            "現在メニュー: sent\nCurrent user request:\n日本語にして\n\n【参照情報】\n種別: 送信待ちメール\nUID: 9\n件名: x\n本文:\nhello\n",
            None,
        );
        assert_eq!(plan.subtasks.len(), 1);
        assert_eq!(plan.subtasks[0].task, None);
        assert!(plan.subtasks[0].goal.contains("日本語"));
    }

    #[test]
    fn resolve_plan_normalizes_compose_write_for_outgoing_pending() {
        let reg = TaskRegistry::builtin();
        let mut plan = PlanArtifact {
            summary: "translate".into(),
            skip_execution: false,
            subtasks: vec![
                Subtask {
                    id: 1,
                    task: Some("compose_context".into()),
                    params: serde_json::json!({}),
                    goal: "ctx".into(),
                    done_when: "get_compose_form".into(),
                },
                Subtask {
                    id: 2,
                    task: Some("compose_write".into()),
                    params: serde_json::json!({}),
                    goal: "スペイン語に翻訳して set_compose_form".into(),
                    done_when: "set_compose_form 成功".into(),
                },
            ],
        };
        reg.resolve_plan(
            &mut plan,
            "現在メニュー: sent\n【改訂コンテキスト】\nkind: outgoing_pending\nCurrent user request:\nスペイン語に",
            None,
        );
        assert_eq!(plan.subtasks.len(), 1);
        assert_eq!(plan.subtasks[0].task, None);
        assert!(plan.subtasks[0].goal.contains("スペイン語"));
    }

    #[test]
    fn resolve_plan_strips_unknown_task_id_as_freeform() {
        let reg = TaskRegistry::builtin();
        let mut plan = PlanArtifact {
            summary: "compose".into(),
            skip_execution: false,
            subtasks: vec![Subtask {
                id: 1,
                task: Some("fetch_mails".into()),
                params: serde_json::json!({}),
                goal: "sync".into(),
                done_when: "done".into(),
            }],
        };
        reg.resolve_plan(&mut plan, "UID: 42\n", None);
        let st = &plan.subtasks[0];
        assert!(st.task.is_none());
        assert!(st.goal.contains("fetch_mails"));
        assert!(st.goal.contains("sync"));

        let policy = reg.tool_policy_for_subtask(st).expect("freeform hinted policy");
        assert_eq!(policy.allow, vec!["fetch_mails".to_string()]);
    }
}
