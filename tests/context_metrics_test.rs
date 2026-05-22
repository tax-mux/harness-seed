#[path = "common/mod.rs"]
mod common;

use common::{build_react_loop_from_config, config_model_name, load_test_config, skip_if_llm_not_ready};
use harness_seed::{ContextUsage, TextSize, TokenSource, TurnContextSummary};

#[test]
fn llm_turn_records_context_usage_from_config() {
    if skip_if_llm_not_ready() {
        return;
    }

    let app = load_test_config().expect("config");
    let mut react = build_react_loop_from_config().expect("react loop");
    let result = react.run_turn("hello").unwrap();

    assert!(!result.context.is_empty(), "model: {}", config_model_name(&app));
    assert!(result.context.llm_calls >= 1);
    assert!(result.context.prompt.chars > 0);
    assert!(result.context.completion.chars > 0);
    assert!(!result.trace.context_usages.is_empty());
    eprintln!(
        "context (model: {}): {}",
        config_model_name(&app),
        result.context
    );
}

#[test]
fn text_size_and_token_estimate() {
    let size = TextSize::measure("hello world");
    assert_eq!(size.chars, 11);
    assert_eq!(size.estimated_tokens(), 3);
}

#[test]
fn api_tokens_preferred_in_summary() {
    let usage = ContextUsage::from_parts("prompt", "out", Some(100), Some(20));
    let summary = TurnContextSummary::from_usages(&[usage]);
    assert_eq!(summary.prompt_tokens, 100);
    assert_eq!(summary.completion_tokens, 20);
    assert_eq!(summary.token_source, TokenSource::Api);
}
