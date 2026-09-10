//! 計画成果物の整形（表示・mission）。

use harness_seed::{
    format_mission, format_plan_for_display, PlanArtifact, PlanProgress, Subtask, TaskRegistry,
};
use serde_json::json;

#[test]
fn format_plan_for_display_lists_subtasks() {
    let plan = PlanArtifact {
        summary: "do work".into(),
        skip_execution: false,
        subtasks: vec![Subtask {
            id: 1,
            task: Some("list_dir".into()),
            params: json!({ "path": "src" }),
            goal: String::new(),
            done_when: "listed".into(),
        }],
    };
    let reg = TaskRegistry::builtin();
    let text = format_plan_for_display(&plan, &reg);
    assert!(text.contains("--- Plan ---"));
    assert!(text.contains("task:list_dir"));
    assert!(text.contains("done_when: listed"));
    assert!(text.contains("list_dir("));
}

#[test]
fn format_mission_includes_subtask_id() {
    let plan = PlanArtifact::single_subtask("list files");
    let st = plan.subtasks[0].clone();
    let reg = TaskRegistry::builtin();
    let m = format_mission(&reg, "list files", &plan, &st, &PlanProgress::default());
    assert!(m.contains("## Subtask"));
    assert!(m.contains("id: 1"));
    assert!(!m.contains("All subtasks"));
    assert!(!m.contains("Plan summary"));
}
