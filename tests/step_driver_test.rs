//! 組み込みタスクは ReAct 優先（react_only）。ステップドライバは明示オプトイン。

use harness_seed::{
    LlmBrain, MockLlmConnector, PlanBrainMode, PlanLlmBrain, ReActConfig, ReActLoop, TaskRegistry,
};

#[test]
fn builtin_tasks_are_react_only_with_planner_summaries() {
    let reg = TaskRegistry::builtin();
    for id in ["list_dir", "write_file_verify", "web_research", "generic"] {
        let def = reg.get(id).unwrap_or_else(|| panic!("missing task {id}"));
        assert!(
            def.react_only,
            "{id} should be react_only under ReAct-first policy"
        );
        assert!(
            !def.effective_planner_summary().trim().is_empty(),
            "{id} needs planner_summary (or summary) for candidate selection"
        );
    }
}

#[test]
fn builtin_list_dir_plan_uses_react_not_step_driver() {
    let mut config = ReActConfig::default();
    config.two_phase = true;
    config.use_step_driver = true;
    config.plan_candidate_selection = false;
    config.context_log_path = None;

    let reg = TaskRegistry::builtin();
    let mut react = ReActLoop::new(
        LlmBrain::new(MockLlmConnector),
        PlanBrainMode::Mock(PlanLlmBrain::new(MockLlmConnector, &reg)),
        config,
    );
    let result = react.run_turn("STEP_DRIVER_TEST").unwrap();

    assert_eq!(result.subtask_results.len(), 1);
    assert!(
        !result.subtask_results[0].used_step_driver,
        "builtin list_dir is react_only — execution must not use step driver"
    );
    // Mock exec brain answers without tools; driver path is what previously forced list_dir.
}
