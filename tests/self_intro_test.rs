#[path = "common/mod.rs"]
mod common;

use common::{
    build_react_loop_from_config, config_model_name, is_llm_error_answer, load_test_config,
    llm_chat_from_config, skip_if_llm_not_ready, SELF_INTRO_USER_PROMPT,
};

/// config.json のモデルで自己紹介チャットが返ること。
#[test]
fn self_intro_llm_chat_responds() {
    if skip_if_llm_not_ready() {
        return;
    }

    let app = load_test_config().expect("config");
    let answer = llm_chat_from_config(SELF_INTRO_USER_PROMPT).expect("llm response");
    assert!(
        answer.chars().count() >= 8,
        "model {} answer too short: {answer}",
        config_model_name(&app)
    );
    eprintln!(
        "self_intro chat (model: {}): {answer}",
        config_model_name(&app)
    );
}

/// config.json + ReAct で自己紹介ターンが完了すること。
#[test]
fn self_intro_react_turn_with_llm() {
    if skip_if_llm_not_ready() {
        return;
    }

    let app = load_test_config().expect("config");
    let mut react = build_react_loop_from_config().expect("react loop");
    let result = react.run_turn(SELF_INTRO_USER_PROMPT).unwrap();

    assert!(
        !is_llm_error_answer(&result.answer),
        "model {} error answer: {}",
        config_model_name(&app),
        result.answer
    );
    assert!(result.answer.chars().count() >= 4);
    assert!(result.steps_used >= 1);
    assert!(!result.context.is_empty());
    assert!(result.context.prompt.chars > 0);
    assert!(result.context.completion.chars > 0);

    eprintln!(
        "self_intro react (model: {}): {}",
        config_model_name(&app),
        result.answer
    );
    eprintln!("context: {}", result.context);

    let path = app
        .resolved_context_log_path()
        .expect("context log path");
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        text.contains(SELF_INTRO_USER_PROMPT),
        "context log missing at {} (file empty or not written)",
        path.display()
    );
}
