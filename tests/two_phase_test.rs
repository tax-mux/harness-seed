//! 計画 → 実行の two_phase オーケストレーション（Mock LLM）。

mod common;

use harness_seed::{
    parse_plan, LlmBrain, MockLlmConnector, PlanArtifact, PlanBrainMode, PlanLlmBrain,
    ReActConfig, ReActLoop, TaskRegistry,
};

#[test]
fn mock_two_phase_runs_plan_then_two_subtasks() {
    let mut config = ReActConfig::default();
    config.two_phase = true;
    config.context_log_path = None;

    let reg = TaskRegistry::builtin();
    let mut react = ReActLoop::new(
        LlmBrain::new(MockLlmConnector),
        PlanBrainMode::Mock(PlanLlmBrain::new(MockLlmConnector, &reg)),
        config,
    );
    let result = react.run_turn("do something").unwrap();

    let plan = result.plan.expect("plan artifact");
    assert_eq!(plan.subtasks.len(), 2);
    assert!(!plan.skip_execution);
    assert_eq!(result.subtask_results.len(), 2);
    assert_eq!(result.subtask_results[0].answer, "subtask \"1\" done");
    assert_eq!(result.subtask_results[1].answer, "subtask \"2\" done");
    assert_eq!(result.answer, "subtask \"2\" done");
    assert!(result.steps_used >= 4, "plan(2) + exec(1+1) steps, got {}", result.steps_used);
}

#[test]
fn parse_plan_integration() {
    let raw = r#"{"summary":"x","skip_execution":false,"subtasks":[{"id":1,"goal":"g","done_when":"d"}]}"#;
    let plan = parse_plan(raw).unwrap();
    assert_eq!(plan.subtasks[0].goal, "g");
    assert!(PlanArtifact::single_subtask("hi").needs_execution());
}
