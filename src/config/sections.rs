use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::de::Error as DeError;
use serde::{Deserialize, Deserializer};
use serde_json::Value;

use crate::tool::ToolPack;

/// `memory` セクション（外部記憶ブリッジ）。
///
/// **local は外部で置き換えない。** プロセス内 diary（`local`）の上に
/// `backends` のアダプタを重ねる。固有設定は `providers.<名前>`。
#[derive(Debug, Clone, Deserialize, Default)]
pub struct MemorySection {
    /// プロセス内 diary を使うか（`local` / `backends` 指定時の既定は `true`）。
    pub local: Option<bool>,
    /// 追加バックエンド名（例: `["mempalace"]`）。local の**後**に重ねる。
    #[serde(default)]
    pub backends: Vec<String>,
    /// プロバイダ名 → 固有設定 JSON。本体は中身を解釈しない。
    #[serde(default)]
    pub providers: BTreeMap<String, Value>,
    #[serde(default)]
    pub recent_work: MemoryRecentWorkSection,
    #[serde(default)]
    pub search: MemorySearchSection,
    /// 記憶 RAG（アダプタ手前の分岐・検索語）。
    #[serde(default)]
    pub rag: MemoryRagSection,
    /// 計画層 `recall` ステップの上限（既定 2、0 で無効）。
    pub recall_max_rounds: Option<usize>,
    /// 旧形式（後方互換）。`mempalace` 指定時も **local は残す**。
    pub provider: Option<String>,
    /// 後方互換: `providers.mempalace` が無いときだけ参照。
    #[serde(default)]
    pub mempalace: Option<MempalaceSection>,
}

/// 旧形式 `memory.mempalace`（`providers.mempalace` 推奨）。
#[derive(Debug, Clone, Deserialize, Default)]
pub struct MempalaceSection {
    pub base_url: Option<String>,
    pub agent_name: Option<String>,
    pub wing: Option<String>,
    pub room: Option<String>,
    pub timeout_secs: Option<u64>,
    /// `"mcp_stdio"`（既定）| `"tools_path"` | `"mcp_jsonrpc"`。
    pub protocol: Option<String>,
    pub api_key: Option<String>,
    pub command: Option<String>,
    pub args: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct MemoryRecentWorkSection {
    pub enabled: Option<bool>,
    pub max_entries: Option<usize>,
    pub max_chars: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct MemorySearchSection {
    pub enabled: Option<bool>,
    pub top_k: Option<usize>,
    pub max_chars: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct MemoryRagSection {
    /// `"rule"` | `"llm"`（LLM 不可時は rule にフォールバック）。
    pub router: Option<String>,
    /// 知識検索クエリの上限。
    pub max_queries: Option<usize>,
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
    /// 実行層 ReAct ループあたりの `thought` 上限（既定: 1）。
    pub max_thoughts: Option<usize>,
    /// `tasks/*.json` の `steps[]` があり `react_only: false` のときだけステップドライバ可。
    /// 組み込みタスクは `react_only: true`（ReAct が選択）。既定はドライバ無効寄りにしないが、
    /// 契約タスクでも react_only なら ReAct 経路になる。
    pub use_step_driver: Option<bool>,
    /// 計画フェーズでタスク summary から候補を選びコンテキストへ登録する。
    pub plan_candidate_selection: Option<bool>,
    /// 候補 summary カタログの最大件数。
    pub plan_catalog_max_entries: Option<usize>,
    /// 候補 summary カタログの最大文字数。
    pub plan_catalog_max_chars: Option<usize>,
    /// ステップ引数監査: `off`（既定）/ `soft` / `hard`。
    pub arg_audit_mode: Option<String>,
    /// 各 ReAct ステップのプロンプト全文を stderr に出す。
    pub show_prompt: Option<bool>,
    /// 計画層の成果物を stdout に表示する（`two_phase` 時）。
    pub show_plan: Option<bool>,
    /// サブタスクごとの契約ツール・実際のツール列を stdout に表示する。
    pub show_task_execution: Option<bool>,
    /// 各ツールのコマンド・結果を stderr に表示する（`run_cmd` の `$ ...` など）。
    pub show_tool_output: Option<bool>,
    /// Thought / ツール要約を stderr に出す（本文はログへ）。
    pub show_thinking: Option<bool>,
    /// 外側推進ループ（`react.advance`）。
    #[serde(default)]
    pub advance: AdvanceSection,
    /// 同一依存波内サブタスクの並列実行（`two_phase` 時。ステップドライバのみ並列）。
    pub parallel_subtasks: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct AdvanceSection {
    pub enabled: Option<bool>,
    /// `off` / `always` / `from_plan`。指定時は `enabled` より優先。
    pub mode: Option<String>,
    pub max_phases: Option<usize>,
    pub clear_session_each_phase: Option<bool>,
    pub max_note_chars: Option<usize>,
    pub show_phases: Option<bool>,
    /// 判定前に必要な実質証拠（read/grep 等）成功 observation 数。
    pub min_substantive_obs: Option<usize>,
    /// 最終合成後にパス引用を証拠 Paths と照合する。
    pub citation_check: Option<bool>,
    /// 結論前に先行 Claims の否定証拠を一度探す。
    pub claim_check: Option<bool>,
    /// 最終回答の不在主張を trace と照合する。
    pub absence_check: Option<bool>,
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
    /// `~/.cursor/mcp.json` 等から MCP ツールを ReAct に載せる。
    #[serde(default)]
    pub mcp: McpToolsSection,
}

/// MCP サーバーを ReAct ツールとして読み込む設定。
#[derive(Debug, Clone, Deserialize, Default)]
pub struct McpToolsSection {
    /// 既定 `true`（sources が空なら無効扱い）。
    pub enabled: Option<bool>,
    /// `mcpServers` を読む JSON パス（`${HOME}` 展開可）。既定: `~/.cursor/mcp.json`。
    #[serde(default)]
    pub sources: Vec<String>,
    /// 読み込むサーバー名（空なら sources 内の全サーバー）。
    #[serde(default)]
    pub servers: Vec<String>,
    /// 1 リクエストのタイムアウト秒（既定 60）。
    pub timeout_secs: Option<u64>,
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
            max_bytes: self
                .max_bytes
                .unwrap_or(LogRotationConfig::DEFAULT_MAX_BYTES),
            max_files: self
                .max_files
                .unwrap_or(LogRotationConfig::DEFAULT_MAX_FILES),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct LogSection {
    /// コンテキスト計測の JSON Lines ログパス（例: `logs/events.jsonl`）。
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
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    Parse {
        path: PathBuf,
        source: serde_json::Error,
    },
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
