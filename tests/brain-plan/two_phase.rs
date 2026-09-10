//! two_phase オーケストレーション（計画層 + 実行層の直列）。

use harness_seed::{
    LlmBrain, MockLlmConnector, PlanBrainMode, PlanLlmBrain, ReActConfig, ReActLoop, TaskRegistry,
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
    assert!(
        result.steps_used >= 4,
        "plan(2) + exec(1+1) steps, got {}",
        result.steps_used
    );

    let harness = result.harness.expect("harness state");
    assert_eq!(harness.total_steps, 2);
    assert_eq!(harness.status, harness_seed::HarnessStatus::Completed);
}
