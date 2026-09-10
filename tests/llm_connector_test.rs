#[path = "common/mod.rs"]
mod common;

use common::{
    build_react_loop_from_config, config_model_name, load_test_config, llm_chat_from_config,
    skip_if_llm_not_ready,
};
use harness_seed::{parse_agent_step, AgentStep};

#[test]
fn parse_agent_step_variants() {
    assert!(matches!(
        parse_agent_step(r#"{"step":"thought","content":"x"}"#, 1).unwrap(),
        AgentStep::Thought(_)
    ));
    assert!(matches!(
        parse_agent_step(r#"{"step":"answer","content":"done"}"#, 1).unwrap(),
        AgentStep::Answer(a) if a == "done"
    ));
}

/// config.json の設定で Ollama/OpenAI 互換 API にチャットできること。
#[test]
fn llm_connector_chat_from_config() {
    if skip_if_llm_not_ready() {
        return;
    }

    let app = load_test_config().expect("config");
    let answer =
        llm_chat_from_config("こんにちは。1文で返してください。").expect("llm answer");
    assert!(!answer.is_empty());
    eprintln!("chat (model: {}): {answer}", config_model_name(&app));
}

/// config.json + LlmBrain で ReAct 1 ターンが完了すること。
#[test]
fn llm_react_turn_from_config() {
    if skip_if_llm_not_ready() {
        return;
    }

    let app = load_test_config().expect("config");
    let mut react = build_react_loop_from_config().expect("react loop");
    let result = react.run_turn("hello").unwrap();

    assert!(!result.answer.is_empty());
    assert!(result.steps_used >= 1);
    eprintln!(
        "react (model: {}): {}",
        config_model_name(&app),
        result.answer
    );
}
