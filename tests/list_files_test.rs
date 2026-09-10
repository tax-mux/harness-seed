#[path = "common/mod.rs"]
mod common;

use common::{
    build_react_loop_from_config, config_model_name, is_llm_error_answer, load_test_config,
    skip_if_llm_not_ready, LIST_FILES_USER_PROMPT,
};

/// ReAct + list_dir でカレントのファイル一覧が返ること。
#[test]
fn list_files_react_turn_with_llm() {
    if skip_if_llm_not_ready() {
        return;
    }

    let app = load_test_config().expect("config");
    let mut react = build_react_loop_from_config().expect("react loop");
    let result = react.run_turn(LIST_FILES_USER_PROMPT).unwrap();

    assert!(
        !is_llm_error_answer(&result.answer),
        "model {} error answer: {}",
        config_model_name(&app),
        result.answer
    );

    let used_list_dir = result.trace.actions.iter().any(|a| a.tool == "list_dir");
    assert!(
        used_list_dir,
        "expected list_dir action in trace: {:?}",
        result.trace.actions
    );

    assert!(
        result.answer.contains("Cargo.toml"),
        "answer should include project files (cwd={:?}): {}",
        std::env::current_dir().ok(),
        result.answer
    );

    eprintln!(
        "list_files react (model: {}): {}",
        config_model_name(&app),
        result.answer
    );
}
