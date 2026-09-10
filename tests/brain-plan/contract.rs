//! 計画層データ契約（`PlanDataContract`）— ドメイン非依存の枠。

use harness_seed::{PlanArtifact, PlanDataContract, Subtask};

#[test]
fn trivial_chat_skips_execution() {
    let c = PlanDataContract::trivial_chat();
    let mut plan = PlanArtifact {
        summary: "hi".into(),
        skip_execution: false,
        subtasks: vec![Subtask {
            id: 1,
            task: Some("list_dir".into()),
            params: serde_json::json!({}),
            goal: "list".into(),
            done_when: "done".into(),
                    depends_on: vec![],
}],
        knowledge_sufficient: None,
        user_reply: None,
    };
    c.enforce_plan(&mut plan);
    assert!(plan.skip_execution);
    assert!(plan.subtasks.is_empty());
}

#[test]
fn format_for_planner_shows_three_layers() {
    let c = PlanDataContract::new(
        "read: user_message",
        "write: chat_only",
        "generic or skip_execution",
    )
    .with_excluded_task_ids(["web_research"]);
    let text = c.format_for_planner();
    assert!(text.contains("INPUT → PROCEDURE (you) → OUTPUT"));
    assert!(text.contains("[INPUT — fixed, do not change]"));
    assert!(text.contains("read: user_message"));
    assert!(text.contains("[OUTPUT — fixed, do not change]"));
    assert!(text.contains("write: chat_only"));
    assert!(text.contains("[PROCEDURE — your PlanArtifact subtasks]"));
    assert!(text.contains("generic or skip_execution"));
}

#[test]
fn host_enforce_collapses_plan() {
    let c = PlanDataContract::new("read: source", "write: sink", "save_item").with_enforce(|plan| {
        let goals: Vec<String> = plan
            .subtasks
            .iter()
            .filter(|st| st.task.as_deref() != Some("load_item"))
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
    });
    let mut plan = PlanArtifact {
        summary: "x".into(),
        skip_execution: false,
        subtasks: vec![
            Subtask {
                id: 1,
                task: Some("load_item".into()),
                params: serde_json::json!({}),
                goal: "load".into(),
                done_when: "loaded".into(),
                            depends_on: vec![],
},
            Subtask {
                id: 2,
                task: Some("save_item".into()),
                params: serde_json::json!({}),
                goal: "persist".into(),
                done_when: "done".into(),
                            depends_on: vec![],
},
        ],
        knowledge_sufficient: None,
        user_reply: None,
    };
    c.enforce_plan(&mut plan);
    assert_eq!(plan.subtasks.len(), 1);
    assert_eq!(plan.subtasks[0].task.as_deref(), Some("save_item"));
    assert_eq!(plan.subtasks[0].params["id"], 9);
    assert!(plan.subtasks[0].goal.contains("persist"));
}

#[test]
fn blocks_reference_fetch_is_host_flag() {
    let c = PlanDataContract::new("in", "out", "hint")
        .with_reference_id(Some(9))
        .with_blocks_reference_fetch(true);
    assert_eq!(c.reference_id, Some(9));
    assert!(c.blocks_reference_fetch);
}
