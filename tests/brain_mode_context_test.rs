//! CLI と同じ `BrainMode` 経路でコンテキスト計測が流れること。

#[path = "common/mod.rs"]
mod common;

use common::{load_test_config, skip_if_llm_not_ready};
use harness_seed::{BrainMode, PlanBrainMode, ReActLoop, SimpleRuleBrain};
use std::path::PathBuf;

#[test]
fn rule_brain_mode_produces_no_context() {
    let mut react = ReActLoop::new(
        BrainMode::Rule(SimpleRuleBrain::new()),
        PlanBrainMode::rule(),
        harness_seed::ReActConfig {
            context_log_path: Some(
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("logs/test-rule-brain.jsonl"),
            ),
            ..harness_seed::ReActConfig::default()
        },
    );
    let result = react.run_turn("help").unwrap();
    assert!(result.context.is_empty());
}

/// `main` と同じ `BrainMode::from_cli` → `ReActLoop` 経路。
#[test]
fn brain_mode_llm_records_context() {
    if skip_if_llm_not_ready() {
        return;
    }

    let app = load_test_config().expect("config");
    let brain = BrainMode::from_cli(&app, false, false).expect("llm brain");
    assert!(matches!(brain, BrainMode::Llm(_)));

    let react_config = app.react_config(false, false);
    let mut react = ReActLoop::new(brain, PlanBrainMode::rule(), react_config);
    let result = react.run_turn("hello").unwrap();

    assert!(
        !result.context.is_empty(),
        "BrainMode must forward poll_context_usage; context was empty"
    );
}
