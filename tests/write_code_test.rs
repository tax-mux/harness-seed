#[path = "common/mod.rs"]
mod common;

use common::{
    build_react_loop_from_config, config_model_name, is_llm_error_answer, load_test_config,
    skip_if_llm_not_ready, WRITE_CODE_USER_PROMPT,
};
use harness_seed::resolve_in_workspace;

/// ReAct + write_file / read_file でソースを書き込めること。
#[test]
fn write_code_react_turn_with_llm() {
    if skip_if_llm_not_ready() {
        return;
    }

    let rel = "tmp/agent_hello.rs";
    let abs = resolve_in_workspace(rel).unwrap();
    let _ = std::fs::remove_file(&abs);

    let app = load_test_config().expect("config");
    let mut react = build_react_loop_from_config().expect("react loop");
    let result = react.run_turn(WRITE_CODE_USER_PROMPT).unwrap();

    assert!(
        !is_llm_error_answer(&result.answer),
        "model {} error: {}",
        config_model_name(&app),
        result.answer
    );

    let used_write = result.trace.actions.iter().any(|a| a.tool == "write_file");
    assert!(used_write, "trace: {:?}", result.trace.actions);

    let content = std::fs::read_to_string(&abs).unwrap_or_default();
    assert!(
        content.contains("fn main"),
        "file content: {content}"
    );

    eprintln!(
        "write_code (model: {}): {}\nfile:\n{content}",
        config_model_name(&app),
        result.answer
    );

    let _ = std::fs::remove_file(&abs);
}
