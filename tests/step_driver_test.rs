//! two_phase + 登録タスク id でステップドライバが使われること。

mod common;

use harness_seed::{
    LlmBrain, MockLlmConnector, PlanBrainMode, PlanLlmBrain, ReActConfig, ReActLoop, TaskRegistry,
};

#[test]
fn two_phase_list_dir_uses_step_driver() {
    let mut config = ReActConfig::default();
    config.two_phase = true;
    config.use_step_driver = true;
    config.context_log_path = None;

    let reg = TaskRegistry::builtin();
    let mut react = ReActLoop::new(
        LlmBrain::new(MockLlmConnector),
        PlanBrainMode::Mock(PlanLlmBrain::new(MockLlmConnector, &reg)),
        config,
    );
    let result = react.run_turn("STEP_DRIVER_TEST").unwrap();

    assert_eq!(result.subtask_results.len(), 1);
    assert!(result.subtask_results[0].used_step_driver);
    assert_eq!(result.subtask_results[0].steps_used, 1);
    assert!(
        result
            .trace
            .actions
            .iter()
            .any(|a| a.tool == "list_dir"),
        "trace should contain list_dir from driver"
    );
}
