use super::*;

use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_unique_dir(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("harness-seed-config-{label}-{nanos}"));
    fs::create_dir_all(&dir).expect("temp dir");
    dir
}

#[test]
fn resolve_prefers_env_override() {
    let dir = temp_unique_dir("env");
    let user = dir.join("user.json");
    let local = dir.join("local.json");
    fs::write(&user, "{}").unwrap();
    fs::write(&local, "{}").unwrap();
    let override_path = dir.join("override.json");
    let got = resolve_default_config_path(Some(override_path.clone()), &user, &local);
    assert_eq!(got, override_path);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn resolve_prefers_user_config_when_present() {
    let dir = temp_unique_dir("user");
    let user = dir.join("user.json");
    let local = dir.join("local.json");
    fs::write(&user, "{}").unwrap();
    fs::write(&local, "{}").unwrap();
    let got = resolve_default_config_path(None, &user, &local);
    assert_eq!(got, user);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn resolve_falls_back_to_cwd_local_when_user_missing() {
    let dir = temp_unique_dir("cwd");
    let user = dir.join("missing-user.json");
    let local = dir.join("local.json");
    fs::write(&local, "{}").unwrap();
    let got = resolve_default_config_path(None, &user, &local);
    assert_eq!(got, local);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn resolve_returns_user_path_when_neither_file_exists() {
    let dir = temp_unique_dir("neither");
    let user = dir.join("missing-user.json");
    let local = dir.join("missing-local.json");
    let got = resolve_default_config_path(None, &user, &local);
    assert_eq!(got, user);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn user_config_path_uses_xdg_config_home() {
    let dir = temp_unique_dir("xdg");
    let prev_xdg = std::env::var_os("XDG_CONFIG_HOME");
    // SAFETY: test process only; restored below.
    unsafe { std::env::set_var("XDG_CONFIG_HOME", &dir) };
    let got = user_config_path();
    assert_eq!(got, dir.join("harness-seed").join("config.json"));
    match prev_xdg {
        Some(v) => unsafe { std::env::set_var("XDG_CONFIG_HOME", v) },
        None => unsafe { std::env::remove_var("XDG_CONFIG_HOME") },
    }
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn default_log_path_is_inside_user_config_dir() {
    let dir = temp_unique_dir("log-xdg");
    let prev_xdg = std::env::var_os("XDG_CONFIG_HOME");
    unsafe { std::env::set_var("XDG_CONFIG_HOME", &dir) };
    let got = default_log_path();
    assert_eq!(
        got,
        dir.join("harness-seed").join("logs").join("events.jsonl")
    );
    match prev_xdg {
        Some(v) => unsafe { std::env::set_var("XDG_CONFIG_HOME", v) },
        None => unsafe { std::env::remove_var("XDG_CONFIG_HOME") },
    }
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn resolved_context_log_relative_uses_user_config_dir() {
    let dir = temp_unique_dir("log-rel");
    let prev_xdg = std::env::var_os("XDG_CONFIG_HOME");
    unsafe { std::env::set_var("XDG_CONFIG_HOME", &dir) };
    let mut cfg = AppConfig::default();
    cfg.log.context_metrics = Some("logs/events.jsonl".into());
    let got = cfg.resolved_context_log_path().expect("path");
    assert_eq!(
        got,
        dir.join("harness-seed").join("logs").join("events.jsonl")
    );
    match prev_xdg {
        Some(v) => unsafe { std::env::set_var("XDG_CONFIG_HOME", v) },
        None => unsafe { std::env::remove_var("XDG_CONFIG_HOME") },
    }
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn resolved_context_log_empty_disables_and_absolute_kept() {
    let mut cfg = AppConfig::default();
    cfg.log.context_metrics = Some(String::new());
    assert!(cfg.resolved_context_log_path().is_none());

    cfg.log.context_metrics = Some("/tmp/harness-seed-abs.jsonl".into());
    assert_eq!(
        cfg.resolved_context_log_path().unwrap(),
        PathBuf::from("/tmp/harness-seed-abs.jsonl")
    );
}

#[test]
fn loads_ollama_sample() {
    let cfg = AppConfig::load_path("config/samples/config.ollama.json").unwrap();
    assert_eq!(cfg.llm.provider.as_deref(), Some("ollama"));
    assert_eq!(cfg.llm.model.as_deref(), Some("gemma-4-26b:latest"));
    assert_eq!(cfg.react.max_steps, Some(16));
}

#[test]
fn builds_ollama_llm_from_sample() {
    let cfg = AppConfig::load_path("config/samples/config.ollama.json").unwrap();
    let llm = cfg.build_llm_config().unwrap();
    assert_eq!(llm.provider, LlmProvider::Ollama);
    assert_eq!(llm.base_url, "http://127.0.0.1:11434/v1");
}

fn without_llm_max_tokens_env<F: FnOnce()>(f: F) {
    let prev = std::env::var_os("HARNESS_SEED_LLM_MAX_TOKENS");
    let prev_legacy = std::env::var_os("MYHARNESS_LLM_MAX_TOKENS");
    unsafe {
        std::env::remove_var("HARNESS_SEED_LLM_MAX_TOKENS");
        std::env::remove_var("MYHARNESS_LLM_MAX_TOKENS");
    }
    f();
    match prev {
        Some(v) => unsafe { std::env::set_var("HARNESS_SEED_LLM_MAX_TOKENS", v) },
        None => unsafe { std::env::remove_var("HARNESS_SEED_LLM_MAX_TOKENS") },
    }
    match prev_legacy {
        Some(v) => unsafe { std::env::set_var("MYHARNESS_LLM_MAX_TOKENS", v) },
        None => unsafe { std::env::remove_var("MYHARNESS_LLM_MAX_TOKENS") },
    }
}

#[test]
fn llm_max_tokens_from_json() {
    let cfg: AppConfig = serde_json::from_str(r#"{"llm":{"max_tokens":4096}}"#).unwrap();
    assert_eq!(cfg.llm.max_tokens, Some(4096));
}

#[test]
fn llm_max_tokens_defaults_and_clamps() {
    without_llm_max_tokens_env(|| {
        let cfg: AppConfig = serde_json::from_str(r#"{"llm":{"provider":"ollama"}}"#).unwrap();
        assert_eq!(cfg.build_llm_config().unwrap().max_tokens, 16384);

        let cfg: AppConfig =
            serde_json::from_str(r#"{"llm":{"provider":"ollama","max_tokens":1}}"#).unwrap();
        assert_eq!(cfg.build_llm_config().unwrap().max_tokens, 256);

        let cfg: AppConfig =
            serde_json::from_str(r#"{"llm":{"provider":"ollama","max_tokens":100000}}"#).unwrap();
        assert_eq!(cfg.build_llm_config().unwrap().max_tokens, 65536);
    });
}

#[test]
fn nonempty_opt_drops_blank_strings() {
    assert_eq!(nonempty_opt(Some(String::new())), None);
    assert_eq!(nonempty_opt(Some("   ".into())), None);
    assert_eq!(
        nonempty_opt(Some(" http://x ".into())),
        Some("http://x".into())
    );
    assert_eq!(nonempty_opt(None), None);
}

#[test]
fn loads_active_config_json() {
    let cfg = AppConfig::load_path("config/config.json.sample").unwrap();
    assert_eq!(cfg.llm.provider.as_deref(), Some("lmstudio"));
    assert_eq!(cfg.react.max_steps, Some(16));
    assert_eq!(cfg.tools.brave_search.max_results, Some(5));
    assert_eq!(cfg.tools.brave_search.fetch_content, Some(false));
}

#[test]
fn resolves_tool_packs_default_without_brave() {
    let cfg: AppConfig = serde_json::from_str(r#"{}"#).unwrap();
    let packs = cfg.resolved_tool_packs();
    assert!(packs.contains(&ToolPack::Basic));
    assert!(packs.contains(&ToolPack::Coding));
    assert!(!packs.contains(&ToolPack::WebSearch));
}

#[test]
fn resolves_tool_packs_from_switch_object() {
    let json = r#"{"tools":{"packs":{"basic":true,"coding":false}}}"#;
    let cfg: AppConfig = serde_json::from_str(json).unwrap();
    let packs = cfg.resolved_tool_packs();
    assert_eq!(packs, vec![ToolPack::Basic]);
}

#[test]
fn resolves_tool_packs_from_string_switches() {
    let json = r#"{"tools":{"packs":{"basic":"true","coding":"true"}}}"#;
    let cfg: AppConfig = serde_json::from_str(json).unwrap();
    let packs = cfg.resolved_tool_packs();
    assert!(packs.contains(&ToolPack::Basic));
    assert!(packs.contains(&ToolPack::Coding));
}

#[test]
fn resolves_tool_packs_from_legacy_list() {
    let json = r#"{"tools":{"packs":["basic"]}}"#;
    let cfg: AppConfig = serde_json::from_str(json).unwrap();
    let packs = cfg.resolved_tool_packs();
    assert_eq!(packs, vec![ToolPack::Basic]);
}

#[test]
fn auto_appends_web_pack_when_brave_key_set() {
    let json = r#"{"tools":{"packs":{"basic":true,"coding":true},"brave_search":{"api_key":"k"}}}"#;
    let cfg: AppConfig = serde_json::from_str(json).unwrap();
    let packs = cfg.resolved_tool_packs();
    assert!(packs.contains(&ToolPack::WebSearch));
}

#[test]
fn web_switch_false_blocks_auto_append() {
    let json =
        r#"{"tools":{"packs":{"basic":true,"web_search":false},"brave_search":{"api_key":"k"}}}"#;
    let cfg: AppConfig = serde_json::from_str(json).unwrap();
    let packs = cfg.resolved_tool_packs();
    assert!(!packs.contains(&ToolPack::WebSearch));
}

#[test]
fn resolves_brave_search_from_config() {
    let json = r#"{
        "tools": {
            "brave_search": {
                "api_key": "test-key",
                "max_results": 3,
                "fetch_content": true,
                "max_content_chars": 1024
            }
        }
    }"#;
    let cfg: AppConfig = serde_json::from_str(json).unwrap();
    let brave = cfg.resolved_brave_search().unwrap();
    assert_eq!(brave.api_key, "test-key");
    assert_eq!(brave.max_results, 3);
    assert!(brave.fetch_content);
    assert_eq!(brave.max_content_chars, 1024);
}

#[test]
fn loads_gemini_sample() {
    let cfg = AppConfig::load_path("config/samples/config.gemini.json").unwrap();
    assert_eq!(cfg.llm.provider.as_deref(), Some("gemini"));
    assert_eq!(cfg.llm_provider(), LlmProvider::Gemini);
    assert_eq!(cfg.llm.model.as_deref(), Some("gemini-2.5-flash"));
}

#[test]
fn loads_anthropic_sample() {
    let cfg = AppConfig::load_path("config/samples/config.anthropic.json").unwrap();
    assert_eq!(cfg.llm.provider.as_deref(), Some("anthropic"));
    assert_eq!(cfg.llm_provider(), LlmProvider::Anthropic);
    assert_eq!(cfg.llm.model.as_deref(), Some("claude-3-5-sonnet-20241022"));
}

#[test]
fn react_config_omits_two_phase_defaults_true() {
    let cfg: AppConfig = serde_json::from_str(r#"{}"#).unwrap();
    assert!(cfg.react_config(false, false).two_phase);
}

#[test]
fn react_config_explicit_two_phase_false() {
    let json = r#"{"react":{"two_phase":false}}"#;
    let cfg: AppConfig = serde_json::from_str(json).unwrap();
    assert!(!cfg.react_config(false, false).two_phase);
}

#[test]
fn react_config_omits_advance_defaults_false() {
    let cfg: AppConfig = serde_json::from_str(r#"{}"#).unwrap();
    assert_eq!(
        cfg.react_config(false, false).advance.mode,
        crate::advance::AdvanceMode::Off
    );
}

#[test]
fn react_config_explicit_advance_true() {
    let json = r#"{"react":{"advance":{"enabled":true}}}"#;
    let cfg: AppConfig = serde_json::from_str(json).unwrap();
    assert_eq!(
        cfg.react_config(false, false).advance.mode,
        crate::advance::AdvanceMode::Always
    );
}

#[test]
fn react_config_mode_from_plan_wins_over_enabled() {
    let json = r#"{"react":{"advance":{"enabled":true,"mode":"from_plan"}}}"#;
    let cfg: AppConfig = serde_json::from_str(json).unwrap();
    assert_eq!(
        cfg.react_config(false, false).advance.mode,
        crate::advance::AdvanceMode::FromPlan
    );
}
