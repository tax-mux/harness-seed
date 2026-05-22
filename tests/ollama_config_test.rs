use std::time::Duration;

use harness_seed::{
    normalize_ollama_base_url, LlmConfig, LlmConnector, LlmProvider, OpenAiConnector,
};

#[test]
fn ollama_config_uses_local_defaults() {
    let config = LlmConfig {
        provider: LlmProvider::Ollama,
        api_key: None,
        base_url: normalize_ollama_base_url("http://127.0.0.1:11434"),
        model: "gemma4".into(),
        timeout: Duration::from_secs(120),
        json_mode: false,
    };
    assert_eq!(config.provider, LlmProvider::Ollama);
    assert_eq!(config.base_url, "http://127.0.0.1:11434/v1");
    assert!(!config.json_mode);
}

#[test]
fn ollama_connector_reports_provider() {
    let config = LlmConfig {
        provider: LlmProvider::Ollama,
        api_key: None,
        base_url: "http://127.0.0.1:11434/v1".into(),
        model: "gemma4".into(),
        timeout: Duration::from_secs(120),
        json_mode: false,
    };
    let connector = OpenAiConnector::new(config).unwrap();
    assert_eq!(connector.config().provider, LlmProvider::Ollama);
    assert_eq!(connector.provider(), LlmProvider::Ollama);
}

#[test]
fn normalize_preserves_v1_suffix() {
    assert_eq!(
        normalize_ollama_base_url("http://192.168.1.10:11434/v1"),
        "http://192.168.1.10:11434/v1"
    );
}
