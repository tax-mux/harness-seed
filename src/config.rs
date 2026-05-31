use std::fs;
use std::path::{Path, PathBuf};

use serde::de::Error as DeError;
use serde::{Deserialize, Deserializer};
use serde_json::Value;

use crate::context::{ContextError, PromptBlocks};
use crate::context_log::default_log_path;
use crate::llm::{LlmConfig, LlmProvider};
use crate::brave_search::BraveSearchConfig;
use crate::advance::AdvanceConfig;
use crate::react::ReActConfig;
use crate::session::SessionMemory;
use crate::tool::{default_packs, packs_from_names, ToolPack};

const DEFAULT_CONFIG_PATH: &str = "config/config.json";

/// 実行時設定（`config/config.json`）。ひな形は `config/samples/`。
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
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct LlmSection {
    pub provider: Option<String>,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub model: Option<String>,
    pub timeout_secs: Option<u64>,
    pub json_mode: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ReactSection {
    pub max_steps: Option<usize>,
    pub verbose: Option<bool>,
    pub show_context_metrics: Option<bool>,
    /// REPL 短期記憶に残す直近ターン数（`Previous turns` 注入）。
    pub session_max_turns: Option<usize>,
    /// 計画フェーズ → 実行フェーズの直列オーケストレーション。
    pub two_phase: Option<bool>,
    /// 計画層 ReAct ループの最大ステップ。
    pub max_steps_plan: Option<usize>,
    /// `tasks/*.json` の `steps[]` 契約があるサブタスクを LLM なしで順次実行する。
    pub use_step_driver: Option<bool>,
    /// 各 ReAct ステップのプロンプト全文を stderr に出す。
    pub show_prompt: Option<bool>,
    /// 計画層の成果物を stdout に表示する（`two_phase` 時）。
    pub show_plan: Option<bool>,
    /// サブタスクごとの契約ツール・実際のツール列を stdout に表示する。
    pub show_task_execution: Option<bool>,
    /// 各ツールのコマンド・結果を stderr に表示する（`run_cmd` の `$ ...` など）。
    pub show_tool_output: Option<bool>,
    /// 外側推進ループ（`react.advance`）。
    #[serde(default)]
    pub advance: AdvanceSection,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct AdvanceSection {
    pub enabled: Option<bool>,
    pub max_phases: Option<usize>,
    pub clear_session_each_phase: Option<bool>,
    pub max_note_chars: Option<usize>,
    pub show_phases: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct PromptSection {
    /// ルールファイルまたはディレクトリ（`.md`）のパス。相対パスはクレートルート基準。
    pub rules_paths: Option<Vec<String>>,
}

/// `tools.packs` のスイッチ形式（`{ "basic": true, "coding": true }`）。`"true"` 文字列も可。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ToolPacksConfig {
    #[serde(default, deserialize_with = "deserialize_opt_bool_switch")]
    pub basic: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_opt_bool_switch")]
    pub coding: Option<bool>,
    #[serde(
        default,
        deserialize_with = "deserialize_opt_bool_switch",
        alias = "web"
    )]
    pub web_search: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_opt_bool_switch")]
    pub full: Option<bool>,
}

impl ToolPacksConfig {
    pub fn is_unconfigured(&self) -> bool {
        self.basic.is_none()
            && self.coding.is_none()
            && self.web_search.is_none()
            && self.full.is_none()
    }

    /// 明示的に `true` のパックだけ返す（`full` が true なら `Full` のみ）。
    pub fn enabled_packs(&self) -> Vec<ToolPack> {
        if self.full == Some(true) {
            return vec![ToolPack::Full];
        }
        let mut packs = Vec::new();
        if self.basic == Some(true) {
            packs.push(ToolPack::Basic);
        }
        if self.coding == Some(true) {
            packs.push(ToolPack::Coding);
        }
        if self.web_search == Some(true) {
            packs.push(ToolPack::WebSearch);
        }
        packs
    }
}

/// 旧来の配列形式 `["basic", "coding"]` も読める。
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ToolPacksField {
    Switches(ToolPacksConfig),
    List(Vec<String>),
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ToolsSection {
    /// ツールパックの ON/OFF。未設定・空オブジェクト時は basic+coding（+ Brave キー時 web_search）。
    pub packs: Option<ToolPacksField>,
    #[serde(default)]
    pub brave_search: BraveSearchSection,
}

fn deserialize_opt_bool_switch<'de, D>(deserializer: D) -> Result<Option<bool>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<Value>::deserialize(deserializer)?;
    match value {
        None => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(b)),
        Some(Value::String(s)) => match s.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" | "on" => Ok(Some(true)),
            "false" | "0" | "no" | "off" => Ok(Some(false)),
            other => Err(DeError::custom(format!(
                "expected boolean switch, got string \"{other}\""
            ))),
        },
        Some(other) => Err(DeError::custom(format!(
            "expected boolean switch, got {other}"
        ))),
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct BraveSearchSection {
    /// Brave Search API キー（空なら `BRAVE_SEARCH_API_KEY` 環境変数を参照）。
    pub api_key: Option<String>,
    /// 1 リクエストあたりの最大件数（1–20、既定 5）。
    pub max_results: Option<u8>,
    /// API の snippet が空のとき結果 URL の本文を取得する。
    pub fetch_content: Option<bool>,
    /// 本文取得時の最大文字数。
    pub max_content_chars: Option<usize>,
}

/// コンテキストログのローテーション（`log.rotation`）。
#[derive(Debug, Clone, Deserialize, Default)]
pub struct LogRotationSection {
    /// このサイズ（バイト）を超えたらローテート（既定 10 MiB）。`0` で無効。
    pub max_bytes: Option<u64>,
    /// 保持する世代数（現行 + バックアップ）。既定 5。`0` でローテーション無効。
    pub max_files: Option<u32>,
}

/// 解決済みローテーション設定。
#[derive(Debug, Clone, Copy)]
pub struct LogRotationConfig {
    pub max_bytes: u64,
    pub max_files: u32,
}

impl LogRotationConfig {
    pub const DEFAULT_MAX_BYTES: u64 = 10 * 1024 * 1024;
    pub const DEFAULT_MAX_FILES: u32 = 5;

    pub fn disabled() -> Self {
        Self {
            max_bytes: 0,
            max_files: 0,
        }
    }

    pub fn enabled(&self) -> bool {
        self.max_bytes > 0 && self.max_files > 0
    }
}

impl LogRotationSection {
    pub fn resolve(&self) -> LogRotationConfig {
        LogRotationConfig {
            max_bytes: self.max_bytes.unwrap_or(LogRotationConfig::DEFAULT_MAX_BYTES),
            max_files: self.max_files.unwrap_or(LogRotationConfig::DEFAULT_MAX_FILES),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct LogSection {
    /// コンテキスト計測の JSON Lines ログパス（例: `logs/context.jsonl`）。
    pub context_metrics: Option<String>,
    #[serde(default)]
    pub rotation: Option<LogRotationSection>,
}

impl LogSection {
    pub fn resolved_rotation(&self) -> LogRotationConfig {
        self.rotation
            .as_ref()
            .map(LogRotationSection::resolve)
            .unwrap_or_else(|| LogRotationSection::default().resolve())
    }
}

#[derive(Debug)]
pub enum ConfigError {
    Read { path: PathBuf, source: std::io::Error },
    Parse { path: PathBuf, source: serde_json::Error },
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Read { path, source } => write!(f, "failed to read {}: {source}", path.display()),
            Self::Parse { path, source } => {
                write!(f, "failed to parse {}: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for ConfigError {}

impl AppConfig {
    /// 既定パス `config/config.json` を読み込む（無ければデフォルト値）。
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
    pub fn resolved_context_log_path(&self) -> Option<PathBuf> {
        match self.log.context_metrics.as_deref() {
            Some("") => None,
            Some(path) => Some(resolve_workspace_path(path)),
            None => Some(default_log_path()),
        }
    }

    /// 設定の `prompt.rules_paths` から [`PromptBlocks`] を構築する。
    pub fn load_prompt_blocks(&self) -> Result<PromptBlocks, ContextError> {
        let mut blocks = PromptBlocks::new();
        if let Some(paths) = &self.prompt.rules_paths {
            let resolved: Vec<PathBuf> = paths
                .iter()
                .map(|p| resolve_workspace_path(p))
                .collect();
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
            two_phase: self.react.two_phase.unwrap_or(false),
            max_steps_plan: self.react.max_steps_plan.unwrap_or(4),
            use_step_driver: self.react.use_step_driver.unwrap_or(true),
            show_prompt: cli_show_prompt || self.react.show_prompt.unwrap_or(false),
            show_plan: self.react.show_plan.unwrap_or(true),
            show_task_execution: self.react.show_task_execution.unwrap_or(true),
            show_tool_output: self.react.show_tool_output.unwrap_or(true),
            advance: AdvanceConfig {
                enabled: self.react.advance.enabled.unwrap_or(false),
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
            },
            monitor_plan_html: false,
        }
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
        let timeout_secs = env_u64_seed("HARNESS_SEED_LLM_TIMEOUT_SECS", "MYHARNESS_LLM_TIMEOUT_SECS")
            .or(self.llm.timeout_secs)
            .unwrap_or(120);

        match provider {
            LlmProvider::Ollama => {
                let host = env_string("OLLAMA_HOST")
                    .or_else(|| env_base_url())
                    .or_else(|| env_string("OPENAI_BASE_URL"))
                    .or(self.llm.base_url.clone())
                    .unwrap_or_else(|| "http://127.0.0.1:11434".into());

                let model = env_string("OLLAMA_MODEL")
                    .or_else(|| env_model())
                    .or_else(|| env_string("OPENAI_MODEL"))
                    .or(self.llm.model.clone())
                    .unwrap_or_else(|| "gemma4".into());

                Ok(LlmConfig {
                    provider: LlmProvider::Ollama,
                    api_key: self.resolved_openai_api_key(),
                    base_url: crate::llm::normalize_ollama_base_url(&host),
                    model,
                    timeout: std::time::Duration::from_secs(timeout_secs),
                    json_mode: false,
                })
            }
            LlmProvider::LmStudio => {
                let host = env_string("LM_STUDIO_HOST")
                    .or_else(|| env_string("LMSTUDIO_HOST"))
                    .or_else(|| env_base_url())
                    .or(self.llm.base_url.clone())
                    .unwrap_or_else(|| "http://127.0.0.1:1234".into());

                let model = env_string("LM_STUDIO_MODEL")
                    .or_else(|| env_model())
                    .or(self.llm.model.clone())
                    .unwrap_or_else(|| "google/gemma-4-e2b".into());

                let json_mode = match env_json_mode() {
                    Some(v) if v == "0" || v.eq_ignore_ascii_case("false") => false,
                    Some(_) => true,
                    None => self.llm.json_mode.unwrap_or(false),
                };

                Ok(LlmConfig {
                    provider: LlmProvider::LmStudio,
                    api_key: self.resolved_openai_api_key(),
                    base_url: crate::llm::normalize_lmstudio_base_url(&host),
                    model,
                    timeout: std::time::Duration::from_secs(timeout_secs),
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
                    .or(self.llm.model.clone())
                    .unwrap_or_else(|| "gemini-2.5-flash".into());

                let json_mode = match env_json_mode() {
                    Some(v) if v == "0" || v.eq_ignore_ascii_case("false") => false,
                    Some(_) => true,
                    None => self.llm.json_mode.unwrap_or(false),
                };

                Ok(LlmConfig {
                    provider: LlmProvider::Gemini,
                    api_key: Some(api_key),
                    base_url,
                    model,
                    timeout: std::time::Duration::from_secs(timeout_secs),
                    json_mode,
                })
            }
            LlmProvider::Anthropic => {
                let api_key = self
                    .resolved_anthropic_api_key()
                    .ok_or(crate::llm::ConnectorError::MissingApiKey)?;

                let base_url = env_string("ANTHROPIC_BASE_URL")
                    .or_else(|| env_base_url())
                    .or(self.llm.base_url.clone())
                    .map(|u| crate::llm::normalize_anthropic_base_url(&u))
                    .unwrap_or_else(|| crate::llm::normalize_anthropic_base_url(""));

                let model = env_string("ANTHROPIC_MODEL")
                    .or_else(|| env_model())
                    .or(self.llm.model.clone())
                    .unwrap_or_else(|| "claude-3-5-sonnet-20241022".into());

                let json_mode = match env_json_mode() {
                    Some(v) if v == "0" || v.eq_ignore_ascii_case("false") => false,
                    Some(_) => true,
                    None => self.llm.json_mode.unwrap_or(false),
                };

                Ok(LlmConfig {
                    provider: LlmProvider::Anthropic,
                    api_key: Some(api_key),
                    base_url,
                    model,
                    timeout: std::time::Duration::from_secs(timeout_secs),
                    json_mode,
                })
            }
            LlmProvider::OpenAi => {
                let api_key = self
                    .resolved_openai_api_key()
                    .ok_or(crate::llm::ConnectorError::MissingApiKey)?;

                let base_url = env_string("OPENAI_BASE_URL")
                    .or_else(|| env_base_url())
                    .or(self.llm.base_url.clone())
                    .unwrap_or_else(|| "https://api.openai.com/v1".into());

                let model = env_model()
                    .or_else(|| env_string("OPENAI_MODEL"))
                    .or(self.llm.model.clone())
                    .unwrap_or_else(|| "gpt-4o-mini".into());

                let json_mode = match env_json_mode() {
                    Some(v) if v == "0" || v.eq_ignore_ascii_case("false") => false,
                    Some(_) => true,
                    None => self.llm.json_mode.unwrap_or(true),
                };

                Ok(LlmConfig {
                    provider: LlmProvider::OpenAi,
                    api_key: Some(api_key),
                    base_url: base_url.trim_end_matches('/').to_string(),
                    model,
                    timeout: std::time::Duration::from_secs(timeout_secs),
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

        if let Some(base) = env_string("OPENAI_BASE_URL").or_else(env_base_url)
        {
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
            .or_else(|| {
                match self.resolve_provider() {
                    LlmProvider::Gemini | LlmProvider::Anthropic => None,
                    _ => self.llm.api_key.clone(),
                }
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

pub fn default_config_path() -> PathBuf {
    env_path("HARNESS_SEED_CONFIG")
        .or_else(|| env_path("MYHARNESS_CONFIG"))
        .unwrap_or_else(|| PathBuf::from(DEFAULT_CONFIG_PATH))
}

pub fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var(name).ok().map(PathBuf::from)
}

/// 相対パスをクレートルート（`CARGO_MANIFEST_DIR`）基準に解決する。
fn resolve_workspace_path(path: &str) -> PathBuf {
    let p = PathBuf::from(path);
    if p.is_absolute() {
        return p;
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(p)
}

fn env_string(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|s| !s.is_empty())
}

fn env_u64(name: &str) -> Option<u64> {
    env_string(name).and_then(|s| s.parse().ok())
}

fn env_u64_seed(primary: &str, legacy: &str) -> Option<u64> {
    env_u64(primary).or_else(|| env_u64(legacy))
}

fn env_base_url() -> Option<String> {
    env_string("HARNESS_SEED_BASE_URL").or_else(|| env_string("MYHARNESS_BASE_URL"))
}

fn env_model() -> Option<String> {
    env_string("HARNESS_SEED_MODEL").or_else(|| env_string("MYHARNESS_MODEL"))
}

fn env_json_mode() -> Option<String> {
    env_string("HARNESS_SEED_JSON_MODE").or_else(|| env_string("MYHARNESS_JSON_MODE"))
}

fn env_llm_provider() -> Option<String> {
    env_string("HARNESS_SEED_LLM_PROVIDER").or_else(|| env_string("MYHARNESS_LLM_PROVIDER"))
}

fn env_api_key() -> Option<String> {
    env_string("HARNESS_SEED_API_KEY").or_else(|| env_string("MYHARNESS_API_KEY"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_ollama_sample() {
        let cfg = AppConfig::load_path("config/samples/config.ollama.json").unwrap();
        assert_eq!(cfg.llm.provider.as_deref(), Some("ollama"));
        assert_eq!(cfg.llm.model.as_deref(), Some("gemma4"));
        assert_eq!(cfg.react.max_steps, Some(16));
    }

    #[test]
    fn builds_ollama_llm_from_sample() {
        let cfg = AppConfig::load_path("config/samples/config.ollama.json").unwrap();
        let llm = cfg.build_llm_config().unwrap();
        assert_eq!(llm.provider, LlmProvider::Ollama);
        assert_eq!(llm.base_url, "http://127.0.0.1:11434/v1");
    }

    #[test]
    fn loads_active_config_json() {
        let cfg = AppConfig::load_path("config/config.json").unwrap();
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
        let json = r#"{"tools":{"packs":{"basic":true,"web_search":false},"brave_search":{"api_key":"k"}}}"#;
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
        assert_eq!(
            cfg.llm.model.as_deref(),
            Some("claude-3-5-sonnet-20241022")
        );
    }
}
