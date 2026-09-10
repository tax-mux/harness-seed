use harness_seed::{
    normalize_lmstudio_base_url, AppConfig, LlmConnector, LlmConnectorKind, LlmProvider,
};

#[test]
fn builds_lmstudio_llm_from_config() {
    let cfg = AppConfig::load_path("config/samples/config.lmstudio.json").unwrap();
    let llm = cfg.build_llm_config().unwrap();
    assert_eq!(llm.provider, LlmProvider::LmStudio);
    assert_eq!(llm.base_url, "http://127.0.0.1:1234/v1");
    assert_eq!(llm.model, "google/gemma-4-e2b");
}

#[test]
fn lmstudio_connector_kind_from_config() {
    let cfg = AppConfig::load_path("config/samples/config.lmstudio.json").unwrap();
    let llm = cfg.build_llm_config().unwrap();
    let connector = LlmConnectorKind::from_config(llm).unwrap();
    assert_eq!(connector.provider(), LlmProvider::LmStudio);
}

#[test]
fn normalize_lmstudio_url() {
    assert_eq!(
        normalize_lmstudio_base_url("http://localhost:1234/v1/"),
        "http://localhost:1234/v1"
    );
}
