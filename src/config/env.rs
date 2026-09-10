pub(super) fn env_string(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|s| !s.is_empty())
}

pub(super) fn nonempty_opt(value: Option<String>) -> Option<String> {
    value.and_then(|s| {
        let t = s.trim();
        if t.is_empty() {
            None
        } else {
            Some(t.to_string())
        }
    })
}

pub(super) fn env_u64(name: &str) -> Option<u64> {
    env_string(name).and_then(|s| s.parse().ok())
}

pub(super) fn env_u64_seed(primary: &str, legacy: &str) -> Option<u64> {
    env_u64(primary).or_else(|| env_u64(legacy))
}

pub(super) fn env_base_url() -> Option<String> {
    env_string("HARNESS_SEED_BASE_URL").or_else(|| env_string("MYHARNESS_BASE_URL"))
}

pub(super) fn env_model() -> Option<String> {
    env_string("HARNESS_SEED_MODEL").or_else(|| env_string("MYHARNESS_MODEL"))
}

pub(super) fn env_json_mode() -> Option<String> {
    env_string("HARNESS_SEED_JSON_MODE").or_else(|| env_string("MYHARNESS_JSON_MODE"))
}

pub(super) fn env_llm_provider() -> Option<String> {
    env_string("HARNESS_SEED_LLM_PROVIDER").or_else(|| env_string("MYHARNESS_LLM_PROVIDER"))
}

pub(super) fn env_api_key() -> Option<String> {
    env_string("HARNESS_SEED_API_KEY").or_else(|| env_string("MYHARNESS_API_KEY"))
}
