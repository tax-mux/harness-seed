//! 計画層データ契約（`PlanDataContract`）。

use harness_seed::{
    PlanArtifact, PlanDataContract, PlanReadSource, PlanWriteTarget, Subtask,
};

#[test]
fn outgoing_contract_excludes_compose_and_mail_read() {
    let c = PlanDataContract {
        read: PlanReadSource::RevisionContext { uid: 9 },
        write: PlanWriteTarget::OutgoingPendingDb { uid: 9 },
        skip_execution: false,
    };
    let ex = c.excluded_task_ids();
    assert!(ex.contains(&"mail_read"));
    assert!(ex.contains(&"compose_write"));
}

#[test]
fn enforce_outgoing_collapses_to_generic() {
    let c = PlanDataContract {
        read: PlanReadSource::RevisionContext { uid: 9 },
        write: PlanWriteTarget::OutgoingPendingDb { uid: 9 },
        skip_execution: false,
    };
    let mut plan = PlanArtifact {
        summary: "x".into(),
        skip_execution: false,
        subtasks: vec![Subtask {
            id: 1,
            task: Some("compose_write".into()),
            params: serde_json::json!({}),
            goal: "スペイン語に翻訳".into(),
            done_when: "done".into(),
        }],
    };
    c.enforce_plan(&mut plan);
    assert_eq!(plan.subtasks.len(), 1);
    assert_eq!(plan.subtasks[0].task.as_deref(), Some("generic"));
    assert!(plan.subtasks[0].goal.contains("スペイン語"));
}

#[test]
fn enforce_mail_sync_collapses_to_mail_sync_task() {
    let c = PlanDataContract {
        read: PlanReadSource::ImapServer,
        write: PlanWriteTarget::MailDb,
        skip_execution: false,
    };
    let mut plan = PlanArtifact {
        summary: "sync".into(),
        skip_execution: false,
        subtasks: vec![Subtask {
            id: 1,
            task: Some("mail_read".into()),
            params: serde_json::json!({}),
            goal: "read".into(),
            done_when: "done".into(),
        }],
    };
    c.enforce_plan(&mut plan);
    assert_eq!(plan.subtasks[0].task.as_deref(), Some("mail_sync"));
}

#[test]
fn trivial_chat_skips_execution() {
    let c = PlanDataContract::trivial_chat();
    let mut plan = PlanArtifact {
        summary: "hi".into(),
        skip_execution: false,
        subtasks: vec![Subtask {
            id: 1,
            task: Some("mail_sync".into()),
            params: serde_json::json!({}),
            goal: "sync".into(),
            done_when: "done".into(),
        }],
    };
    c.enforce_plan(&mut plan);
    assert!(plan.skip_execution);
    assert!(plan.subtasks.is_empty());
}

#[test]
fn format_for_planner_shows_three_layers() {
    let c = PlanDataContract {
        read: PlanReadSource::RevisionContext { uid: 9 },
        write: PlanWriteTarget::OutgoingPendingDb { uid: 9 },
        skip_execution: false,
    };
    let text = c.format_for_planner();
    assert!(text.contains("INPUT → PROCEDURE (you) → OUTPUT"));
    assert!(text.contains("[INPUT — fixed, do not change]"));
    assert!(text.contains("revision_context"));
    assert!(text.contains("[OUTPUT — fixed, do not change]"));
    assert!(text.contains("outgoing_pending_db"));
    assert!(text.contains("[PROCEDURE — your PlanArtifact subtasks]"));
    assert!(text.contains("generic only"));
}

#[test]
fn imap_reference_uid_excludes_outgoing_revision() {
    let c = PlanDataContract {
        read: PlanReadSource::RevisionContext { uid: 9 },
        write: PlanWriteTarget::OutgoingPendingDb { uid: 9 },
        skip_execution: false,
    };
    assert_eq!(c.imap_reference_uid(), None);
    assert_eq!(c.outgoing_pending_uid(), Some(9));
}
