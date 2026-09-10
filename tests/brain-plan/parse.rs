//! 計画 JSON（`parse_plan`）のパース。

use harness_seed::{parse_plan, PlanArtifact};

#[test]
fn parses_plan_with_subtasks() {
    let raw = r#"{
            "summary": "list and summarize",
            "skip_execution": false,
            "subtasks": [
                {"id": 1, "goal": "list dir", "done_when": "have listing"},
                {"id": 2, "goal": "summarize", "done_when": "answer ready"}
            ]
        }"#;
    let plan = parse_plan(raw).unwrap();
    assert_eq!(plan.subtasks.len(), 2);
    assert!(!plan.skip_execution);
}

#[test]
fn parses_skip_execution() {
    let raw = r#"{"summary":"hi","skip_execution":true,"subtasks":[]}"#;
    let plan = parse_plan(raw).unwrap();
    assert!(plan.skip_execution);
    assert!(plan.subtasks.is_empty());
}

#[test]
fn parses_subtask_with_task_id() {
    let raw = r#"{
            "summary": "list",
            "skip_execution": false,
            "subtasks": [{"id": 1, "task": "list_dir", "params": {"path": "."}}]
        }"#;
    let plan = parse_plan(raw).unwrap();
    assert_eq!(plan.subtasks[0].task.as_deref(), Some("list_dir"));
}

#[test]
fn parses_input_steps_output_shape() {
    let raw = r#"{
            "input": ["read: revision_context (UID 9)"],
            "steps": [{"id": 1, "task": "generic", "params": {}, "goal": "日本語化", "done_when": "件名本文を返した"}],
            "output": "write: outgoing_pending_db (UID 9)",
            "skip_execution": false
        }"#;
    let plan = parse_plan(raw).unwrap();
    assert_eq!(plan.summary, "write: outgoing_pending_db (UID 9)");
    assert_eq!(plan.subtasks.len(), 1);
    assert_eq!(plan.subtasks[0].task.as_deref(), Some("generic"));
}

#[test]
fn single_subtask_needs_execution() {
    let raw = r#"{"summary":"x","skip_execution":false,"subtasks":[{"id":1,"goal":"g","done_when":"d"}]}"#;
    let plan = parse_plan(raw).unwrap();
    assert_eq!(plan.subtasks[0].goal, "g");
    assert!(PlanArtifact::single_subtask("hi").needs_execution());
}
