//! 計画層のデータ契約（内部3層）:
//! - **INPUT**（read）— ホストが固定。LLM は変更しない。
//! - **PROCEDURE**（PlanArtifact subtasks）— LLM が設計する唯一の可変部分。
//! - **OUTPUT**（write）— ホストが固定。LLM は変更しない。

use super::{PlanArtifact, Subtask};

/// 計画・実行が参照すべきデータの読み元。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanReadSource {
    /// ユーザーメッセージのみ（ツールで追加取得しない）。
    UserMessage,
    /// プロンプト内 `【改訂コンテキスト】`（origin / latest）。
    RevisionContext { uid: i64 },
    /// IMAP `get_email`（受信メール UID）。
    ImapEmail { uid: i64 },
    /// 作成フォーム `get_compose_form` のみ。
    ComposeForm,
    /// 作成フォーム + 参照メール IMAP。
    ComposeFormAndImap { uid: i64 },
    /// IMAP サーバ `fetch_mails`（同期・取得・受信）。
    ImapServer,
    /// ローカル DB `list_emails` / `get_email`（同期不要な閲覧）。
    LocalMailDb,
    /// 外部 Web `web_search`。
    ExternalWeb,
}

/// 成果の書き込み先。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanWriteTarget {
    /// チャット回答のみ。
    ChatOnly,
    /// 作成フォーム `set_compose_form`。
    ComposeForm,
    /// 送信待ち DB（エージェントは最終回答で件名/本文を返し、アプリが保存）。
    OutgoingPendingDb { uid: i64 },
    /// 作成フォーム非表示の文案生成（回答抽出 → アプリが未送信として保存）。
    PendingDraftViaAnswer,
    /// ローカルメール DB（fetch_mails / classify / set_email_category）。
    MailDb,
}

/// 1 ターン分の read / write 契約（LLM に推測させない）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanDataContract {
    pub read: PlanReadSource,
    pub write: PlanWriteTarget,
    /// 挨拶・雑談などツール不要ターン。
    pub skip_execution: bool,
}

impl PlanDataContract {
    pub fn chat_only(read: PlanReadSource) -> Self {
        Self {
            read,
            write: PlanWriteTarget::ChatOnly,
            skip_execution: false,
        }
    }

    pub fn trivial_chat() -> Self {
        Self {
            read: PlanReadSource::UserMessage,
            write: PlanWriteTarget::ChatOnly,
            skip_execution: true,
        }
    }

    pub fn reference_uid(&self) -> Option<i64> {
        self.imap_reference_uid()
            .or_else(|| self.outgoing_pending_uid())
    }

    /// IMAP `get_email` / `mail_read` 用 UID（送信待ち DB id は含めない）。
    pub fn imap_reference_uid(&self) -> Option<i64> {
        match &self.read {
            PlanReadSource::ImapEmail { uid }
            | PlanReadSource::ComposeFormAndImap { uid } => Some(*uid),
            _ => None,
        }
    }

    pub fn outgoing_pending_uid(&self) -> Option<i64> {
        match &self.write {
            PlanWriteTarget::OutgoingPendingDb { uid } => Some(*uid),
            _ => None,
        }
    }

    pub fn blocks_imap_mail_read(&self) -> bool {
        matches!(
            self.read,
            PlanReadSource::RevisionContext { .. }
                | PlanReadSource::UserMessage
                | PlanReadSource::ComposeForm
                | PlanReadSource::ImapServer
                | PlanReadSource::LocalMailDb
                | PlanReadSource::ExternalWeb
        )
    }

    /// 挨拶等: 手順層ごとスキップ（LLM 計画ループも不要）。
    pub fn skip_plan_layer(&self) -> bool {
        self.skip_execution
    }

    pub fn is_outgoing_pending_revision(&self) -> bool {
        matches!(self.write, PlanWriteTarget::OutgoingPendingDb { .. })
    }

    /// 計画層内部: 入力境界（固定）。LLM は変更しない。
    pub fn format_input_layer(&self) -> String {
        let mut lines = vec![format!("read: {}", self.read.label())];
        if let Some(uid) = self.imap_reference_uid().or_else(|| self.outgoing_pending_uid()) {
            lines.push(format!("reference_uid: {uid}"));
        }
        lines.join("\n")
    }

    /// 計画層内部: 出力境界（固定）。LLM は変更しない。
    pub fn format_output_layer(&self) -> String {
        self.write.label()
    }

    /// 計画層内部: LLM が埋める手順層の制約。
    pub fn format_procedure_layer(&self) -> String {
        if self.skip_execution {
            return "Emit plan JSON with skip_execution: true and empty steps.".into();
        }
        format!(
            "Hard instruction: read ONLY from INPUT and write ONLY to OUTPUT; plan ONLY the in-between procedure.\n\
             Plan subtasks (procedure) that connect INPUT read → OUTPUT write.\n\
             Allowed task ids: {}\n\
             Output JSON shape:\n\
             {{\"input\":[\"...\"],\"steps\":[{{\"id\":1,\"task\":\"...\",\"params\":{{}},\"goal\":\"...\",\"done_when\":\"...\"}}],\"output\":\"...\",\"skip_execution\":false}}\n\
             Do not add steps that read/write elsewhere.",
            self.allowed_tasks_hint()
        )
    }

    /// 計画 LLM 向け: 入力層 → 手順層(LLM) → 出力層 の3層構造。
    pub fn format_for_planner(&self) -> String {
        [
            "--- Plan layer: INPUT → PROCEDURE (you) → OUTPUT ---".into(),
            String::new(),
            "[INPUT — fixed, do not change]".into(),
            self.format_input_layer(),
            String::new(),
            "[OUTPUT — fixed, do not change]".into(),
            self.format_output_layer(),
            String::new(),
            "[PROCEDURE — your PlanArtifact subtasks]".into(),
            self.format_procedure_layer(),
        ]
        .join("\n")
    }

    /// この契約と両立しない task id（カタログフィルタ用）。
    pub fn excluded_task_ids(&self) -> Vec<&'static str> {
        if self.skip_execution {
            return vec![
                "mail_read",
                "mail_sync",
                "compose_context",
                "compose_context_no_ref",
                "compose_write",
                "web_research",
            ];
        }
        match (&self.read, &self.write) {
            (_, PlanWriteTarget::OutgoingPendingDb { .. })
            | (_, PlanWriteTarget::PendingDraftViaAnswer) => vec![
                "mail_read",
                "mail_sync",
                "compose_context",
                "compose_context_no_ref",
                "compose_write",
                "web_research",
            ],
            (PlanReadSource::ImapEmail { .. }, PlanWriteTarget::ChatOnly)
            | (PlanReadSource::ImapEmail { .. }, PlanWriteTarget::MailDb) => vec![
                "mail_sync",
                "compose_context",
                "compose_context_no_ref",
                "compose_write",
                "web_research",
            ],
            (PlanReadSource::ComposeForm, PlanWriteTarget::ComposeForm)
            | (PlanReadSource::ComposeFormAndImap { .. }, PlanWriteTarget::ComposeForm) => {
                vec!["mail_sync", "web_research"]
            }
            (PlanReadSource::ImapServer, PlanWriteTarget::MailDb) => vec![
                "mail_read",
                "compose_context",
                "compose_context_no_ref",
                "compose_write",
                "web_research",
            ],
            (PlanReadSource::LocalMailDb, PlanWriteTarget::ChatOnly) => vec![
                "mail_sync",
                "mail_read",
                "compose_context",
                "compose_context_no_ref",
                "compose_write",
            ],
            (PlanReadSource::ExternalWeb, PlanWriteTarget::ChatOnly) => vec![
                "mail_sync",
                "mail_read",
                "compose_context",
                "compose_context_no_ref",
                "compose_write",
            ],
            (PlanReadSource::UserMessage, PlanWriteTarget::ChatOnly) => vec![
                "mail_sync",
                "compose_context",
                "compose_write",
                "mail_read",
            ],
            _ => vec![],
        }
    }

    fn allowed_tasks_hint(&self) -> &'static str {
        if self.skip_execution {
            return "skip_execution only";
        }
        match (&self.read, &self.write) {
            (_, PlanWriteTarget::OutgoingPendingDb { .. })
            | (_, PlanWriteTarget::PendingDraftViaAnswer) => "pending_outgoing_save (save_pending_outgoing_mail + reload_sendmail_list)",
            (PlanReadSource::ImapEmail { .. }, PlanWriteTarget::ChatOnly) => {
                "mail_read then generic"
            }
            (PlanReadSource::ImapEmail { .. }, PlanWriteTarget::MailDb) => {
                "mail_read then generic (classify_email / set_email_category)"
            }
            (PlanReadSource::ComposeForm, PlanWriteTarget::ComposeForm)
            | (PlanReadSource::ComposeFormAndImap { .. }, PlanWriteTarget::ComposeForm) => {
                "compose_context then compose_write"
            }
            (PlanReadSource::ImapServer, PlanWriteTarget::MailDb) => "mail_sync only",
            (PlanReadSource::LocalMailDb, PlanWriteTarget::ChatOnly) => {
                "generic (list_emails / get_email as needed)"
            }
            (PlanReadSource::ExternalWeb, PlanWriteTarget::ChatOnly) => {
                "web_research then generic"
            }
            _ => "generic or skip_execution",
        }
    }

    /// 契約違反の plan を正規化する（`resolve_plan` から呼ぶ）。
    pub fn enforce_plan(&self, plan: &mut PlanArtifact) {
        if self.skip_execution {
            plan.skip_execution = true;
            plan.subtasks.clear();
            return;
        }
        if plan.skip_execution {
            return;
        }
        match (&self.read, &self.write) {
            (_, PlanWriteTarget::OutgoingPendingDb { .. })
            | (_, PlanWriteTarget::PendingDraftViaAnswer) => {
                collapse_to_pending_outgoing_save(plan, self.default_pending_outgoing_goal(), self.outgoing_pending_uid());
            }
            (PlanReadSource::ImapServer, PlanWriteTarget::MailDb) => {
                collapse_to_single_task(
                    plan,
                    "mail_sync",
                    "fetch_mails を実行",
                    "同期完了",
                );
            }
            (PlanReadSource::ComposeForm, PlanWriteTarget::ComposeForm)
            | (PlanReadSource::ComposeFormAndImap { .. }, PlanWriteTarget::ComposeForm) => {
                if !matches!(self.read, PlanReadSource::ComposeFormAndImap { .. }) {
                    strip_tasks(plan, &["mail_read"]);
                }
            }
            (PlanReadSource::ImapEmail { .. }, PlanWriteTarget::ChatOnly)
            | (PlanReadSource::ImapEmail { .. }, PlanWriteTarget::MailDb) => {
                strip_tasks(
                    plan,
                    &[
                        "compose_context",
                        "compose_write",
                        "compose_context_no_ref",
                        "mail_sync",
                    ],
                );
                ensure_mail_read_first(plan);
            }
            (PlanReadSource::LocalMailDb, PlanWriteTarget::ChatOnly) => {
                strip_tasks(
                    plan,
                    &[
                        "mail_sync",
                        "mail_read",
                        "compose_context",
                        "compose_write",
                        "compose_context_no_ref",
                    ],
                );
            }
            (PlanReadSource::ExternalWeb, PlanWriteTarget::ChatOnly) => {
                strip_tasks(
                    plan,
                    &[
                        "mail_sync",
                        "mail_read",
                        "compose_context",
                        "compose_write",
                        "compose_context_no_ref",
                    ],
                );
            }
            (_, PlanWriteTarget::ChatOnly) => {
                strip_tasks(
                    plan,
                    &["compose_context", "compose_write", "compose_context_no_ref"],
                );
            }
            _ => {}
        }
    }

    fn default_pending_outgoing_goal(&self) -> String {
        match &self.write {
            PlanWriteTarget::OutgoingPendingDb { .. } => {
                "【改訂コンテキスト】latest を基準に改訂し、save_pending_outgoing_mail で送信待ちキューへ保存する"
                    .into()
            }
            PlanWriteTarget::PendingDraftViaAnswer => {
                "文案を作成し save_pending_outgoing_mail で送信待ちキューへ保存する".into()
            }
            _ => "ユーザーの依頼を満たす".into(),
        }
    }
}

impl PlanReadSource {
    fn label(&self) -> String {
        match self {
            Self::UserMessage => "user_message (prompt only)".into(),
            Self::RevisionContext { uid } => {
                format!("revision_context (UID {uid}; origin/latest in prompt — no get_email)")
            }
            Self::ImapEmail { uid } => format!("imap_email (UID {uid}; use mail_read / get_email)"),
            Self::ComposeForm => "compose_form (get_compose_form)".into(),
            Self::ComposeFormAndImap { uid } => format!(
                "compose_form + imap_email (UID {uid}; get_compose_form then get_email if needed)"
            ),
            Self::ImapServer => "imap_server (fetch_mails — server sync)".into(),
            Self::LocalMailDb => "local_mail_db (list_emails / get_email — no fetch_mails)".into(),
            Self::ExternalWeb => "external_web (web_search)".into(),
        }
    }
}

impl PlanWriteTarget {
    fn label(&self) -> String {
        match self {
            Self::ChatOnly => "chat_only (final answer in chat)".into(),
            Self::ComposeForm => "compose_form (set_compose_form required)".into(),
            Self::OutgoingPendingDb { uid } => format!(
                "outgoing_pending_db (UID {uid}; save_pending_outgoing_mail で上書き保存)"
            ),
            Self::PendingDraftViaAnswer => {
                "pending_draft_via_answer (save_pending_outgoing_mail で新規保存)".into()
            }
            Self::MailDb => "mail_db (fetch_mails / classify_email / set_email_category)".into(),
        }
    }
}

fn strip_tasks(plan: &mut PlanArtifact, blocked: &[&str]) {
    plan.subtasks
        .retain(|st| !st.task.as_deref().is_some_and(|t| blocked.contains(&t)));
    renumber_subtasks(plan);
}

fn collapse_to_single_task(
    plan: &mut PlanArtifact,
    task_id: &str,
    goal: &str,
    done_when: &str,
) {
    plan.subtasks = vec![Subtask {
        id: 1,
        task: Some(task_id.into()),
        params: serde_json::json!({}),
        goal: goal.into(),
        done_when: done_when.into(),
    }];
}

fn collapse_to_pending_outgoing_save(
    plan: &mut PlanArtifact,
    fallback_goal: String,
    update_uid: Option<i64>,
) {
    const SKIP_GOAL_FROM: &[&str] = &["mail_read", "compose_context", "web_research"];
    let goals: Vec<String> = plan
        .subtasks
        .iter()
        .filter(|st| !st.task.as_deref().is_some_and(|t| SKIP_GOAL_FROM.contains(&t)))
        .map(|st| st.goal.clone())
        .filter(|g| !g.trim().is_empty())
        .collect();
    let goal = if goals.is_empty() {
        fallback_goal
    } else {
        goals.join(" → ")
    };
    let mut params = serde_json::json!({});
    if let Some(uid) = update_uid {
        params["id"] = serde_json::json!(uid);
    }
    plan.subtasks = vec![Subtask {
        id: 1,
        task: Some("pending_outgoing_save".into()),
        params,
        goal,
        done_when: "save_pending_outgoing_mail 成功".into(),
    }];
}

fn ensure_mail_read_first(plan: &mut PlanArtifact) {
    if plan
        .subtasks
        .iter()
        .any(|st| st.task.as_deref() == Some("mail_read"))
    {
        return;
    }
    for st in &mut plan.subtasks {
        st.id += 1;
    }
    plan.subtasks.insert(
        0,
        Subtask {
            id: 1,
            task: Some("mail_read".into()),
            params: serde_json::json!({}),
            goal: "get_email で参照メールの全文を確認する".into(),
            done_when: "get_email 成功".into(),
        },
    );
}

fn renumber_subtasks(plan: &mut PlanArtifact) {
    for (i, st) in plan.subtasks.iter_mut().enumerate() {
        st.id = (i + 1) as u32;
    }
}
