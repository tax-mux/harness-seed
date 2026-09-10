use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::advance::{AdvanceConfig, AdvanceMode};
use crate::brave_search::BraveSearchConfig;
use crate::context::{ContextError, PromptBlocks};
use crate::context_log::default_log_path;
use crate::llm::{LlmConfig, LlmProvider};
use crate::memory::{build_memory_bridge, MemoryBridge, MemoryRuntimeConfig};
use crate::react::ReActConfig;
use crate::session::SessionMemory;
use crate::tool::{default_packs, packs_from_names, ToolPack};

mod env;
mod sections;

pub use sections::*;

use env::{
    env_api_key, env_base_url, env_json_mode, env_llm_provider, env_model, env_string,
    env_u64_seed, nonempty_opt,
};

const DEFAULT_CONFIG_PATH: &str = "config/config.json";
const USER_CONFIG_DIR: &str = "harness-seed";
const USER_CONFIG_FILE: &str = "config.json";

/// 実行時設定。既定は XDG の `~/.config/harness-seed/config.json`（無ければ cwd の `config/config.json`）。
/// ひな形は `config/config.json.sample` と `config/samples/`。
#[derive(Debug, Clone, Deserialize, Default)]
pub struct AppConfig {
    #[serde(default)]
    pub llm: LlmSection,
    #[serde(default)]
    pub react: ReactSection,
    #[serde(default)]
    pub log: LogSection,
    #[serde(default)]
    pub prompt: PromptSection,
    #[serde(default)]
    pub tools: ToolsSection,
    #[serde(default)]
    pub memory: MemorySection,
}

impl AppConfig {
    /// 既定パス（ユーザ設定 → cwd フォールバック）を読み込む（無ければデフォルト値）。
    pub fn load_default() -> Result<Self, ConfigError> {
        Self::load_path(default_config_path())
    }

    pub fn load_path(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let path = path.as_ref().to_path_buf();
        if !path.exists() {
            eprintln!(
                "config: {} not found, using built-in defaults",
                path.display()
            );
            return Ok(Self::default());
        }

        let text = fs::read_to_string(&path).map_err(|source| ConfigError::Read {
            path: path.clone(),
            source,
        })?;
        serde_json::from_str(&text).map_err(|source| ConfigError::Parse { path, source })
    }

    /// コンテキスト計測ログパス（未設定時は [`DEFAULT_CONTEXT_LOG_REL`]、空文字で無効）。
    ///
    /// 相対パスは [`user_config_dir`]（`…/harness-seed/`）基準。クレートルートではない。
    pub fn resolved_context_log_path(&self) -> Option<PathBuf> {
        match self.log.context_metrics.as_deref() {
            Some("") => None,
            Some(path) => Some(resolve_user_config_rel(path)),
            None => Some(default_log_path()),
        }
    }

    /// 設定の `prompt.rules_paths` から [`PromptBlocks`] を構築する。
    pub fn load_prompt_blocks(&self) -> Result<PromptBlocks, ContextError> {
        let mut blocks = PromptBlocks::new();
        if let Some(paths) = &self.prompt.rules_paths {
            let resolved: Vec<PathBuf> = paths.iter().map(|p| resolve_workspace_path(p)).collect();
            if !resolved.is_empty() {
                blocks.load_rules_from_paths(&resolved)?;
            }
        }
        Ok(blocks)
    }

    pub fn react_config(&self, cli_verbose: bool, cli_show_prompt: bool) -> ReActConfig {
        ReActConfig {
            max_steps: self.react.max_steps.unwrap_or(16),
            verbose: cli_verbose || self.react.verbose.unwrap_or(false),
            show_context_metrics: self.react.show_context_metrics.unwrap_or(true),
            context_log_path: self.resolved_context_log_path(),
            log_rotation: self.log.resolved_rotation(),
            session_max_turns: self
                .react
                .session_max_turns
                .unwrap_or(SessionMemory::DEFAULT_MAX_TURNS),
            // CLI / AppConfig: 省略時は計画→実行。ライブラリの ReActConfig::default は false のまま。
            two_phase: self.react.two_phase.unwrap_or(true),
            max_steps_plan: self.react.max_steps_plan.unwrap_or(8),
            max_thoughts: self.react.max_thoughts.unwrap_or(1).max(1),
            use_step_driver: self.react.use_step_driver.unwrap_or(true),
            plan_candidate_selection: self.react.plan_candidate_selection.unwrap_or(true),
            plan_catalog_max_entries: self.react.plan_catalog_max_entries.unwrap_or(40).max(1),
            plan_catalog_max_chars: self.react.plan_catalog_max_chars.unwrap_or(8_000).max(256),
            arg_audit_mode: self
                .react
                .arg_audit_mode
                .as_deref()
                .map(crate::tasks::ArgAuditMode::parse)
                .unwrap_or_default(),
            show_prompt: cli_show_prompt || self.react.show_prompt.unwrap_or(false),
            show_plan: self.react.show_plan.unwrap_or(true),
            show_task_execution: self.react.show_task_execution.unwrap_or(true),
            show_tool_output: self.react.show_tool_output.unwrap_or(true),
            show_thinking: self.react.show_thinking.unwrap_or(true),
            parallel_subtasks: self.react.parallel_subtasks.unwrap_or(false),
            advance: AdvanceConfig {
                mode: resolve_advance_mode(&self.react.advance),
                max_phases: self.react.advance.max_phases.unwrap_or(8).max(1),
                clear_session_each_phase: self
                    .react
                    .advance
                    .clear_session_each_phase
                    .unwrap_or(true),
                max_note_chars: self
                    .react
                    .advance
                    .max_note_chars
                    .unwrap_or(1500)
                    .clamp(200, 16_000),
                show_phases: self.react.advance.show_phases.unwrap_or(true),
                min_substantive_obs: self.react.advance.min_substantive_obs.unwrap_or(3).max(1),
                citation_check: self.react.advance.citation_check.unwrap_or(true),
                claim_check: self.react.advance.claim_check.unwrap_or(true),
                claim_check_max_steps: self
                    .react
                    .advance
                    .claim_check_max_steps
                    .unwrap_or(6)
                    .clamp(2, 32),
                claim_check_sterile_run_cmd_limit: self
                    .react
                    .advance
                    .claim_check_sterile_run_cmd_limit
                    .unwrap_or(2)
                    .clamp(1, 8),
                absence_check: self.react.advance.absence_check.unwrap_or(true),
            },
            monitor_plan_html: false,
            memory: self.memory_runtime_config(),
        }
    }

    /// `memory` セクションから実行時注入設定を組み立てる。
    pub fn memory_runtime_config(&self) -> MemoryRuntimeConfig {
        let defaults = MemoryRuntimeConfig::default();
        MemoryRuntimeConfig {
            recent_work_enabled: self
                .memory
                .recent_work
                .enabled
                .unwrap_or(defaults.recent_work_enabled),
            recent_work_max_entries: self
                .memory
                .recent_work
                .max_entries
                .unwrap_or(defaults.recent_work_max_entries)
                .max(1),
            recent_work_max_chars: self
                .memory
                .recent_work
                .max_chars
                .unwrap_or(defaults.recent_work_max_chars)
                .clamp(80, 16_000),
            search_enabled: self
                .memory
                .search
                .enabled
                .unwrap_or(defaults.search_enabled),
            search_top_k: self
                .memory
                .search
                .top_k
                .unwrap_or(defaults.search_top_k)
                .max(1),
            search_max_chars: self
                .memory
                .search
                .max_chars
                .unwrap_or(defaults.search_max_chars)
                .clamp(80, 32_000),
            recall_max_rounds: self
                .memory
                .recall_max_rounds
                .unwrap_or(defaults.recall_max_rounds),
            rag_router: self
                .memory
                .rag
                .router
                .clone()
                .unwrap_or(defaults.rag_router),
            rag_max_queries: self
                .memory
                .rag
                .max_queries
                .unwrap_or(defaults.rag_max_queries)
                .max(1),
        }
    }

    /// `memory` レイヤ構成に応じたブリッジ（工場は [`crate::memory::build_memory_bridge`]）。
    pub fn memory_bridge(&self) -> Box<dyn MemoryBridge> {
        build_memory_bridge(&self.memory)
    }

    /// 解決済みレイヤ表示名（例: `local` / `local+mempalace` / `noop`）。
    pub fn memory_provider_name(&self) -> String {
        crate::memory::resolve_memory_layers(&self.memory).label()
    }

    pub fn llm_provider(&self) -> LlmProvider {
        self.resolve_provider()
    }

    /// 有効なツールパック一覧。`tools.packs` 未設定時は basic + coding（+ Brave キー時 web_search）。
    pub fn resolved_tool_packs(&self) -> Vec<ToolPack> {
        let include_web = self.resolved_brave_search().is_some();
        let mut packs = match &self.tools.packs {
            None => default_packs(include_web),
            Some(ToolPacksField::List(names)) if names.is_empty() => default_packs(include_web),
            Some(ToolPacksField::List(names)) => packs_from_names(names),
            Some(ToolPacksField::Switches(cfg)) if cfg.is_unconfigured() => {
                default_packs(include_web)
            }
            Some(ToolPacksField::Switches(cfg)) => cfg.enabled_packs(),
        };
        if packs.is_empty() {
            return packs;
        }
        let web_explicit = match &self.tools.packs {
            Some(ToolPacksField::Switches(cfg)) => cfg.web_search,
            _ => None,
        };
        if include_web
            && web_explicit != Some(false)
            && !packs.contains(&ToolPack::Full)
            && !packs.contains(&ToolPack::WebSearch)
        {
            packs.push(ToolPack::WebSearch);
        }
        packs
    }

    /// Brave Web Search 用設定。API キーが無いときは `None`（`web_search` ツールは失敗応答）。
    pub fn resolved_brave_search(&self) -> Option<BraveSearchConfig> {
        let api_key = self
            .tools
            .brave_search
            .api_key
            .as_deref()
            .map(str::trim)
            .filter(|k| !k.is_empty())
            .map(str::to_string)
            .or_else(|| env_string("BRAVE_SEARCH_API_KEY"))?;

        Some(BraveSearchConfig {
            api_key,
            max_results: self
                .tools
                .brave_search
                .max_results
                .unwrap_or(5)
                .clamp(1, 20),
            fetch_content: self.tools.brave_search.fetch_content.unwrap_or(false),
            max_content_chars: self
                .tools
                .brave_search
                .max_content_chars
                .unwrap_or(2048)
                .clamp(256, 32_768),
        })
    }

    /// 設定で LLM 頭脳を使うか（`llm.provider` または API キー）。
    pub fn uses_llm(&self) -> bool {
        self.llm.provider.is_some()
            || self.resolved_openai_api_key().is_some()
            || self.resolved_gemini_api_key().is_some()
            || self.resolved_anthropic_api_key().is_some()
    }

    pub fn llm_available(&self) -> bool {
        matches!(
            self.llm_provider(),
            LlmProvider::Ollama | LlmProvider::LmStudio
        ) || self.resolved_gemini_api_key().is_some()
            || self.resolved_anthropic_api_key().is_some()
            || self.resolved_openai_api_key().is_some()
    }

    pub fn build_llm_config(&self) -> Result<LlmConfig, crate::llm::ConnectorError> {
        let provider = self.resolve_provider();
        let timeout_secs = env_u64_seed(
            "HARNESS_SEED_LLM_TIMEOUT_SECS",
            "MYHARNESS_LLM_TIMEOUT_SECS",
        )
        .or(self.llm.timeout_secs)
        .unwrap_or(120);
        let max_tokens = env_u64_seed("HARNESS_SEED_LLM_MAX_TOKENS", "MYHARNESS_LLM_MAX_TOKENS")
            .or(self.llm.max_tokens)
            .unwrap_or(16_384)
            .clamp(256, 65_536) as u32;

        match provider {
            LlmProvider::Ollama => {
                let host = env_string("OLLAMA_HOST")
                    .or_else(|| env_base_url())
                    .or_else(|| env_string("OPENAI_BASE_URL"))
                    .or_else(|| nonempty_opt(self.llm.base_url.clone()))
                    .unwrap_or_else(|| "http://127.0.0.1:11434".into());

                let model = env_string("OLLAMA_MODEL")
                    .or_else(|| env_model())
                    .or_else(|| env_string("OPENAI_MODEL"))
                    .or_else(|| nonempty_opt(self.llm.model.clone()))
                    .unwrap_or_else(|| "gemma4".into());

                let base_url = crate::llm::normalize_ollama_base_url(&host);
                crate::llm::require_absolute_http_base(&base_url)?;

                Ok(LlmConfig {
                    provider: LlmProvider::Ollama,
                    api_key: self.resolved_openai_api_key(),
                    base_url,
                    model,
                    timeout: std::time::Duration::from_secs(timeout_secs),
                    max_tokens,
                    json_mode: false,
                })
            }
            LlmProvider::LmStudio => {
                let host = env_string("LM_STUDIO_HOST")
                    .or_else(|| env_string("LMSTUDIO_HOST"))
                    .or_else(|| env_base_url())
                    .or_else(|| nonempty_opt(self.llm.base_url.clone()))
                    .unwrap_or_else(|| "http://127.0.0.1:1234".into());

                let model = env_string("LM_STUDIO_MODEL")
                    .or_else(|| env_model())
                    .or_else(|| nonempty_opt(self.llm.model.clone()))
                    .unwrap_or_else(|| "google/gemma-4-e2b".into());

                let json_mode = match env_json_mode() {
                    Some(v) if v == "0" || v.eq_ignore_ascii_case("false") => false,
                    Some(_) => true,
                    None => self.llm.json_mode.unwrap_or(false),
                };

                let base_url = crate::llm::normalize_lmstudio_base_url(&host);
                crate::llm::require_absolute_http_base(&base_url)?;

                Ok(LlmConfig {
                    provider: LlmProvider::LmStudio,
                    api_key: self.resolved_openai_api_key(),
                    base_url,
                    model,
                    timeout: std::time::Duration::from_secs(timeout_secs),
                    max_tokens,
                    json_mode,
                })
            }
            LlmProvider::Gemini => {
                let api_key = self
                    .resolved_gemini_api_key()
                    .ok_or(crate::llm::ConnectorError::MissingApiKey)?;

                let base_url = crate::llm::resolve_gemini_base_url(
                    self.llm.base_url.as_deref(),
                    env_string("GEMINI_BASE_URL"),
                );

                let model = env_string("GEMINI_MODEL")
                    .or_else(|| env_model())
                    .or_else(|| nonempty_opt(self.llm.model.clone()))
                    .unwrap_or_else(|| "gemini-2.5-flash".into());

                let json_mode = match env_json_mode() {
                    Some(v) if v == "0" || v.eq_ignore_ascii_case("false") => false,
                    Some(_) => true,
                    None => self.llm.json_mode.unwrap_or(false),
                };

                crate::llm::require_absolute_http_base(&base_url)?;

                Ok(LlmConfig {
                    provider: LlmProvider::Gemini,
                    api_key: Some(api_key),
                    base_url,
                    model,
                    timeout: std::time::Duration::from_secs(timeout_secs),
                    max_tokens,
                    json_mode,
                })
            }
            LlmProvider::Anthropic => {
                let api_key = self
                    .resolved_anthropic_api_key()
                    .ok_or(crate::llm::ConnectorError::MissingApiKey)?;

                let base_url = env_string("ANTHROPIC_BASE_URL")
                    .or_else(|| env_base_url())
                    .or_else(|| nonempty_opt(self.llm.base_url.clone()))
                    .map(|u| crate::llm::normalize_anthropic_base_url(&u))
                    .unwrap_or_else(|| crate::llm::normalize_anthropic_base_url(""));

                let model = env_string("ANTHROPIC_MODEL")
                    .or_else(|| env_model())
                    .or_else(|| nonempty_opt(self.llm.model.clone()))
                    .unwrap_or_else(|| "claude-3-5-sonnet-20241022".into());

                let json_mode = match env_json_mode() {
                    Some(v) if v == "0" || v.eq_ignore_ascii_case("false") => false,
                    Some(_) => true,
                    None => self.llm.json_mode.unwrap_or(false),
                };

                crate::llm::require_absolute_http_base(&base_url)?;

                Ok(LlmConfig {
                    provider: LlmProvider::Anthropic,
                    api_key: Some(api_key),
                    base_url,
                    model,
                    timeout: std::time::Duration::from_secs(timeout_secs),
                    max_tokens,
                    json_mode,
                })
            }
            LlmProvider::OpenAi => {
                let api_key = self
                    .resolved_openai_api_key()
                    .ok_or(crate::llm::ConnectorError::MissingApiKey)?;

                let base_url = env_string("OPENAI_BASE_URL")
                    .or_else(|| env_base_url())
                    .or_else(|| nonempty_opt(self.llm.base_url.clone()))
                    .unwrap_or_else(|| "https://api.openai.com/v1".into());

                let model = env_model()
                    .or_else(|| env_string("OPENAI_MODEL"))
                    .or_else(|| nonempty_opt(self.llm.model.clone()))
                    .unwrap_or_else(|| "gpt-4o-mini".into());

                let json_mode = match env_json_mode() {
                    Some(v) if v == "0" || v.eq_ignore_ascii_case("false") => false,
                    Some(_) => true,
                    None => self.llm.json_mode.unwrap_or(true),
                };

                let base_url = base_url.trim_end_matches('/').to_string();
                crate::llm::require_absolute_http_base(&base_url)?;

                Ok(LlmConfig {
                    provider: LlmProvider::OpenAi,
                    api_key: Some(api_key),
                    base_url,
                    model,
                    timeout: std::time::Duration::from_secs(timeout_secs),
                    max_tokens,
                    json_mode,
                })
            }
        }
    }

    fn resolve_provider(&self) -> LlmProvider {
        if let Some(name) = env_llm_provider() {
            if let Some(p) = LlmProvider::parse(&name) {
                return p;
            }
        }

        if let Some(name) = self.llm.provider.as_deref() {
            if let Some(p) = LlmProvider::parse(name) {
                return p;
            }
        }

        if std::env::var("OLLAMA_HOST").is_ok() || std::env::var("OLLAMA_MODEL").is_ok() {
            return LlmProvider::Ollama;
        }

        if std::env::var("LM_STUDIO_HOST").is_ok() || std::env::var("LM_STUDIO_MODEL").is_ok() {
            return LlmProvider::LmStudio;
        }

        if let Some(base) = env_string("OPENAI_BASE_URL").or_else(env_base_url) {
            if base.contains("11434") {
                return LlmProvider::Ollama;
            }
            if base.contains("1234") {
                return LlmProvider::LmStudio;
            }
        }

        if self
            .llm
            .base_url
            .as_ref()
            .is_some_and(|u| u.contains("11434"))
        {
            return LlmProvider::Ollama;
        }

        if self
            .llm
            .base_url
            .as_ref()
            .is_some_and(|u| u.contains("1234"))
        {
            return LlmProvider::LmStudio;
        }

        if env_string("GEMINI_API_KEY").is_some() || env_string("GEMINI_MODEL").is_some() {
            return LlmProvider::Gemini;
        }

        if env_string("ANTHROPIC_API_KEY").is_some() || env_string("ANTHROPIC_MODEL").is_some() {
            return LlmProvider::Anthropic;
        }

        LlmProvider::OpenAi
    }

    fn resolved_openai_api_key(&self) -> Option<String> {
        env_string("OPENAI_API_KEY")
            .or_else(|| env_api_key())
            .or_else(|| env_string("OLLAMA_API_KEY"))
            .or_else(|| env_string("LM_STUDIO_API_KEY"))
            .or_else(|| match self.resolve_provider() {
                LlmProvider::Gemini | LlmProvider::Anthropic => None,
                _ => self.llm.api_key.clone(),
            })
            .filter(|k| !k.is_empty())
    }

    fn resolved_gemini_api_key(&self) -> Option<String> {
        env_string("GEMINI_API_KEY")
            .or_else(|| {
                if self.resolve_provider() == LlmProvider::Gemini {
                    self.llm.api_key.clone()
                } else {
                    None
                }
            })
            .filter(|k| !k.is_empty())
    }

    fn resolved_anthropic_api_key(&self) -> Option<String> {
        env_string("ANTHROPIC_API_KEY")
            .or_else(|| env_string("CLAUDE_API_KEY"))
            .or_else(|| {
                if self.resolve_provider() == LlmProvider::Anthropic {
                    self.llm.api_key.clone()
                } else {
                    None
                }
            })
            .filter(|k| !k.is_empty())
    }
}

fn resolve_advance_mode(section: &AdvanceSection) -> AdvanceMode {
    if let Some(raw) = section.mode.as_deref() {
        return AdvanceMode::parse(raw);
    }
    match section.enabled {
        Some(true) => AdvanceMode::Always,
        Some(false) | None => AdvanceMode::Off,
    }
}

/// 設定ファイルの既定パス。
///
/// 優先順:
/// 1. `HARNESS_SEED_CONFIG` / `MYHARNESS_CONFIG`
/// 2. ユーザ設定（[`user_config_path`]）が存在するとき
/// 3. cwd の `config/config.json` が存在するとき（後方互換）
/// 4. いずれも無ければユーザ設定パス（[`AppConfig::load_path`] がビルトイン既定へ）
pub fn default_config_path() -> PathBuf {
    let env_override = env_path("HARNESS_SEED_CONFIG").or_else(|| env_path("MYHARNESS_CONFIG"));
    resolve_default_config_path(
        env_override,
        &user_config_path(),
        Path::new(DEFAULT_CONFIG_PATH),
    )
}

/// `$XDG_CONFIG_HOME/harness-seed`（未設定時は `~/.config/harness-seed`）。
pub fn user_config_dir() -> PathBuf {
    config_home().join(USER_CONFIG_DIR)
}

/// `$XDG_CONFIG_HOME/harness-seed/config.json`（未設定時は `~/.config/harness-seed/config.json`）。
pub fn user_config_path() -> PathBuf {
    user_config_dir().join(USER_CONFIG_FILE)
}

/// 相対パスを [`user_config_dir`] 基準に解決する（絶対パスはそのまま）。
pub fn resolve_user_config_rel(path: &str) -> PathBuf {
    let p = PathBuf::from(path);
    if p.is_absolute() {
        return p;
    }
    user_config_dir().join(p)
}

/// テスト可能なパス解決（ファイル存在でユーザ設定と cwd を切り替える）。
pub(crate) fn resolve_default_config_path(
    env_override: Option<PathBuf>,
    user_path: &Path,
    cwd_local: &Path,
) -> PathBuf {
    if let Some(path) = env_override {
        return path;
    }
    if user_path.is_file() {
        return user_path.to_path_buf();
    }
    if cwd_local.is_file() {
        return cwd_local.to_path_buf();
    }
    user_path.to_path_buf()
}

fn config_home() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        let trimmed = xdg.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".config")
}

pub fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var(name).ok().map(PathBuf::from)
}

/// 相対パスを [`workspace_root`] 基準に解決する。
pub fn resolve_workspace_path(path: &str) -> PathBuf {
    let p = PathBuf::from(path);
    if p.is_absolute() {
        return p;
    }
    crate::tool::workspace_root().join(p)
}

#[cfg(test)]
mod tests;
