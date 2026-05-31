//! Harness パース（作業指示書 → HarnessState）と内部状態。

use harness_seed::{parse_harness, HarnessStatus, HarnessState, PlanArtifact, TaskRegistry};

#[test]
fn parses_json_plan_into_harness_state() {
    let raw = r#"{
            "input": ["read: ctx"],
            "steps": [{"id": 1, "task": "list_dir", "params": {"path": "."}, "goal": "", "done_when": "ok"}],
            "output": "write: out",
            "skip_execution": false
        }"#;
    let hs = parse_harness(raw, "fallback").unwrap();
    assert_eq!(hs.total_steps, 1);
    assert_eq!(hs.current_step, 1);
    assert_eq!(hs.plan.subtasks[0].task.as_deref(), Some("list_dir"));
    assert!(!hs.work_instructions.is_empty());
}

#[test]
fn parses_numbered_text_steps() {
    let raw = "作業指示書\n1. メールを読む\n2. 下書きを書く\n";
    let hs = parse_harness(raw, "fallback").unwrap();
    assert_eq!(hs.total_steps, 2);
    assert_eq!(hs.plan.subtasks[0].goal, "メールを読む");
    assert_eq!(hs.plan.subtasks[1].goal, "下書きを書く");
}

#[test]
fn text_fallback_to_single_subtask() {
    let hs = parse_harness("just do the thing", "user ask").unwrap();
    assert_eq!(hs.total_steps, 1);
    assert_eq!(hs.plan.subtasks[0].goal, "user ask");
}

#[test]
fn harness_state_tracks_steps() {
    let raw = r#"{
        "summary": "two steps",
        "skip_execution": false,
        "subtasks": [
            {"id": 1, "goal": "a", "done_when": "ok"},
            {"id": 2, "goal": "b", "done_when": "ok"}
        ]
    }"#;
    let mut hs = parse_harness(raw, "fallback").unwrap();
    assert_eq!(hs.total_steps, 2);
    assert_eq!(hs.current_step, 1);
    assert_eq!(hs.status, HarnessStatus::Ready);

    hs.begin_execution();
    assert_eq!(hs.status, HarnessStatus::Executing);
    assert!(hs.advance_after_subtask(1));
    assert_eq!(hs.current_step, 2);
    assert!(!hs.advance_after_subtask(2));
    assert_eq!(hs.status, HarnessStatus::Completed);
}

#[test]
fn format_current_step_for_prompt_lists_contract() {
    let plan = PlanArtifact {
        summary: "work".into(),
        skip_execution: false,
        subtasks: vec![harness_seed::Subtask {
            id: 1,
            task: Some("list_dir".into()),
            params: serde_json::json!({ "path": "src" }),
            goal: String::new(),
            done_when: "listed".into(),
        }],
    };
    let mut hs = HarnessState::new("1. list src", plan);
    hs.begin_execution();
    hs.set_tool_set(vec!["list_dir".into(), "read_file".into()]);
    let text = hs.format_current_step_for_prompt(&TaskRegistry::builtin());
    assert!(text.contains("Step 1/1"));
    assert!(text.contains("task: list_dir"));
    assert!(text.contains("allowed tools"));
    assert!(text.contains("list_dir, read_file"));
}
