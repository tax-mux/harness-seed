//! `config.memory` から [`MemoryBridge`] を組み立てる工場。
//!
//! **local は外部バックエンドで置き換えない。** 既定ではプロセス内 diary を残し、
//! `backends` に列挙したアダプタをその上に重ねる（[`LayeredMemoryBridge`]）。

use serde::Deserialize;
use serde_json::Value;

use crate::config::MemorySection;

use super::layered::LayeredMemoryBridge;
use super::{LocalDiaryBridge, MemoryBridge, NoopBridge};

/// 組み込みで認識するバックエンド名。
pub const PROVIDER_NOOP: &str = "noop";
pub const PROVIDER_LOCAL: &str = "local";
pub const PROVIDER_MEMPALACE: &str = "mempalace";

/// 解決済みレイヤ構成（ログ・テスト用）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryLayerPlan {
    pub local: bool,
    pub backends: Vec<String>,
}

impl MemoryLayerPlan {
    pub fn label(&self) -> String {
        let mut parts = Vec::new();
        if self.local {
            parts.push(PROVIDER_LOCAL.to_string());
        }
        parts.extend(self.backends.iter().cloned());
        if parts.is_empty() {
            PROVIDER_NOOP.to_string()
        } else {
            parts.join("+")
        }
    }
}

/// `memory` セクションからレイヤ計画を決める。
///
/// - 新形式: `local` / `backends` を優先
/// - 旧形式 `provider`:
///   - `noop` → レイヤなし
///   - `local` → local のみ
///   - `mempalace` 等 → **local + そのバックエンド**（置き換えない）
pub fn resolve_memory_layers(section: &MemorySection) -> MemoryLayerPlan {
    let uses_new_shape = section.local.is_some() || !section.backends.is_empty();

    if uses_new_shape {
        return MemoryLayerPlan {
            local: section.local.unwrap_or(true),
            backends: section
                .backends
                .iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty() && s != PROVIDER_LOCAL && s != PROVIDER_NOOP)
                .collect(),
        };
    }

    match section
        .provider
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        None | Some(PROVIDER_NOOP) => MemoryLayerPlan {
            local: false,
            backends: vec![],
        },
        Some(PROVIDER_LOCAL) => MemoryLayerPlan {
            local: true,
            backends: vec![],
        },
        Some(name) => MemoryLayerPlan {
            // 外部を指定しても local は残す
            local: true,
            backends: vec![name.to_string()],
        },
    }
}

/// 設定に応じたブリッジを返す。レイヤが空なら noop。
pub fn build_memory_bridge(section: &MemorySection) -> Box<dyn MemoryBridge> {
    let plan = resolve_memory_layers(section);
    let mut layers: Vec<Box<dyn MemoryBridge>> = Vec::new();

    if plan.local {
        layers.push(Box::new(LocalDiaryBridge::new()));
    }

    for name in &plan.backends {
        match build_backend(section, name) {
            Some(bridge) => layers.push(bridge),
            None => {
                eprintln!("[memory] skip backend {name:?}");
            }
        }
    }

    match layers.len() {
        0 => Box::new(NoopBridge),
        1 => layers.into_iter().next().unwrap(),
        _ => Box::new(LayeredMemoryBridge::new(layers)),
    }
}

fn build_backend(section: &MemorySection, name: &str) -> Option<Box<dyn MemoryBridge>> {
    match name {
        PROVIDER_MEMPALACE => build_mempalace_bridge(section),
        other => {
            eprintln!(
                "[memory] unknown backend {other:?} (known extras: {PROVIDER_MEMPALACE})"
            );
            None
        }
    }
}

/// `memory.providers.<name>` の JSON。無ければ `None`。
pub fn provider_options<'a>(section: &'a MemorySection, name: &str) -> Option<&'a Value> {
    section.providers.get(name)
}

/// プロバイダ固有設定を `T` にデシリアライズする。キーが無ければ `T::default()`。
pub fn deserialize_provider_options<T>(section: &MemorySection, name: &str) -> T
where
    T: for<'de> Deserialize<'de> + Default,
{
    match provider_options(section, name) {
        Some(value) => match serde_json::from_value::<T>(value.clone()) {
            Ok(cfg) => cfg,
            Err(err) => {
                eprintln!("[memory] providers.{name} parse error: {err}; using defaults");
                T::default()
            }
        },
        None => T::default(),
    }
}

#[cfg(feature = "mempalace")]
fn build_mempalace_bridge(section: &MemorySection) -> Option<Box<dyn MemoryBridge>> {
    let mut cfg = deserialize_provider_options::<MempalaceConfigDto>(section, PROVIDER_MEMPALACE);
    if provider_options(section, PROVIDER_MEMPALACE).is_none() {
        if let Some(legacy) = section.mempalace.as_ref() {
            cfg = MempalaceConfigDto::from_legacy(legacy);
        }
    }

    let config = cfg.into_adapter_config();
    match super::MempalaceBridge::connect(config) {
        Ok(bridge) => {
            eprintln!("[memory] mempalace {}", bridge.scope_label());
            Some(Box::new(bridge))
        }
        Err(err) => {
            eprintln!("[memory] mempalace init failed: {err}; layer skipped");
            None
        }
    }
}

#[cfg(not(feature = "mempalace"))]
fn build_mempalace_bridge(_section: &MemorySection) -> Option<Box<dyn MemoryBridge>> {
    eprintln!(
        "[memory] backend {PROVIDER_MEMPALACE:?} requires cargo feature `mempalace`; layer skipped"
    );
    None
}

/// `memory.providers.mempalace` 用 DTO（アダプタ型を config に漏らさない）。
#[cfg(feature = "mempalace")]
#[derive(Debug, Clone, Deserialize)]
struct MempalaceConfigDto {
    #[serde(default = "default_mempalace_url")]
    base_url: String,
    #[serde(default = "default_mempalace_agent")]
    agent_name: String,
    #[serde(default)]
    wing: Option<String>,
    #[serde(default)]
    room: Option<String>,
    #[serde(default = "default_mempalace_timeout")]
    timeout_secs: u64,
    #[serde(default)]
    protocol: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
    /// MCP stdio 実行ファイル（Cursor mcp.json の `command`）。
    #[serde(default)]
    command: Option<String>,
    /// MCP stdio 引数（Cursor mcp.json の `args`）。
    #[serde(default)]
    args: Option<Vec<String>>,
    /// 起動ディレクトリ名をプロジェクトキーにする（既定 true）。
    #[serde(default)]
    wing_from_cwd: Option<bool>,
    /// wing 未作成時にシード drawer で初期化（既定 true）。
    #[serde(default)]
    init_wing_if_missing: Option<bool>,
}

#[cfg(feature = "mempalace")]
impl Default for MempalaceConfigDto {
    fn default() -> Self {
        Self {
            base_url: default_mempalace_url(),
            agent_name: default_mempalace_agent(),
            wing: None,
            room: None,
            timeout_secs: default_mempalace_timeout(),
            protocol: Some("mcp_stdio".into()),
            api_key: None,
            command: None,
            args: None,
            wing_from_cwd: Some(true),
            init_wing_if_missing: Some(true),
        }
    }
}

#[cfg(feature = "mempalace")]
impl MempalaceConfigDto {
    fn from_legacy(legacy: &crate::config::MempalaceSection) -> Self {
        Self {
            base_url: legacy
                .base_url
                .clone()
                .unwrap_or_else(default_mempalace_url),
            agent_name: legacy
                .agent_name
                .clone()
                .unwrap_or_else(default_mempalace_agent),
            wing: legacy.wing.clone(),
            room: legacy.room.clone(),
            timeout_secs: legacy.timeout_secs.unwrap_or_else(default_mempalace_timeout),
            protocol: legacy.protocol.clone(),
            api_key: legacy.api_key.clone(),
            command: None,
            args: None,
            wing_from_cwd: None,
            init_wing_if_missing: None,
        }
    }

    fn into_adapter_config(self) -> mempalace_adapter::MempalaceConfig {
        use mempalace_adapter::{MempalaceConfig, MempalaceProtocol};
        let protocol = match self.protocol.as_deref().unwrap_or("mcp_stdio") {
            "tools_path" | "http" => MempalaceProtocol::ToolsPath,
            "mcp_jsonrpc" | "jsonrpc" | "mcp_http" => MempalaceProtocol::McpJsonrpc,
            _ => MempalaceProtocol::McpStdio,
        };
        MempalaceConfig::from_env_or(MempalaceConfig {
            base_url: self.base_url,
            agent_name: self.agent_name,
            wing: self.wing,
            room: self.room,
            wing_from_cwd: self.wing_from_cwd.unwrap_or(true),
            init_wing_if_missing: self.init_wing_if_missing.unwrap_or(true),
            timeout_secs: self.timeout_secs,
            protocol,
            api_key: self.api_key,
            command: self.command,
            args: self.args,
        })
    }
}

#[cfg(feature = "mempalace")]
fn default_mempalace_url() -> String {
    "http://127.0.0.1:8765".into()
}

#[cfg(feature = "mempalace")]
fn default_mempalace_agent() -> String {
    "harness-seed".into()
}

#[cfg(feature = "mempalace")]
fn default_mempalace_timeout() -> u64 {
    30
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::MemorySection;
    use serde_json::json;

    #[test]
    fn default_is_noop() {
        let plan = resolve_memory_layers(&MemorySection::default());
        assert!(!plan.local);
        assert!(plan.backends.is_empty());
        assert_eq!(plan.label(), "noop");
    }

    #[test]
    fn legacy_local_provider() {
        let section = MemorySection {
            provider: Some("local".into()),
            ..MemorySection::default()
        };
        let plan = resolve_memory_layers(&section);
        assert!(plan.local);
        assert!(plan.backends.is_empty());
    }

    #[test]
    fn legacy_mempalace_keeps_local() {
        let section = MemorySection {
            provider: Some("mempalace".into()),
            ..MemorySection::default()
        };
        let plan = resolve_memory_layers(&section);
        assert!(plan.local);
        assert_eq!(plan.backends, vec!["mempalace"]);
        assert_eq!(plan.label(), "local+mempalace");
    }

    #[test]
    fn new_shape_local_plus_backends() {
        let section = MemorySection {
            local: Some(true),
            backends: vec!["mempalace".into()],
            providers: [(
                "mempalace".into(),
                json!({"base_url": "http://example.test"}),
            )]
            .into_iter()
            .collect(),
            ..MemorySection::default()
        };
        let plan = resolve_memory_layers(&section);
        assert!(plan.local);
        assert_eq!(plan.backends, vec!["mempalace"]);
    }

    #[test]
    fn new_shape_can_disable_local_explicitly() {
        let section = MemorySection {
            local: Some(false),
            backends: vec!["mempalace".into()],
            ..MemorySection::default()
        };
        let plan = resolve_memory_layers(&section);
        assert!(!plan.local);
        assert_eq!(plan.backends, vec!["mempalace"]);
    }

    #[test]
    fn local_layer_builds_and_diaries() {
        let section = MemorySection {
            local: Some(true),
            backends: vec![],
            ..MemorySection::default()
        };
        let mut bridge = build_memory_bridge(&section);
        bridge
            .diary(&super::super::DiaryEntry {
                user_input: "u".into(),
                summary: "s".into(),
                answer: "a".into(),
                phases: vec![],
            })
            .unwrap();
        assert_eq!(bridge.recent_work(3).unwrap().len(), 1);
    }

    #[test]
    fn provider_options_reads_providers_map() {
        let section = MemorySection {
            local: Some(true),
            backends: vec!["mempalace".into()],
            providers: [(
                "mempalace".into(),
                json!({"base_url": "http://example.test", "agent_name": "bot"}),
            )]
            .into_iter()
            .collect(),
            ..MemorySection::default()
        };
        let opts = provider_options(&section, "mempalace").unwrap();
        assert_eq!(opts["base_url"], "http://example.test");
    }
}
