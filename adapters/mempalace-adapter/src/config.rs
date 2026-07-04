use std::path::PathBuf;
use std::time::Duration;

use serde::Deserialize;

use crate::types::MempalaceError;

/// mempalace 接続設定。
#[derive(Debug, Clone, Deserialize)]
pub struct MempalaceConfig {
    /// HTTP モード用（`tools_path` / `mcp_jsonrpc`）。
    #[serde(default = "default_base_url")]
    pub base_url: String,
    /// エージェント名。`wing_{project}` 内の **room** 名になる（エージェント固有 diary）。
    #[serde(default = "default_agent_name")]
    pub agent_name: String,
    /// プロジェクト wing（`wing_{project}`）。未設定かつ `wing_from_cwd` なら起動ディレクトリ名から生成。
    pub wing: Option<String>,
    /// エージェント room の明示 override（未設定なら `agent_name`）。
    pub room: Option<String>,
    /// `wing` 未指定時、cwd / `HARNESS_WORKSPACE` のディレクトリ名をプロジェクトキーにする。
    #[serde(default = "default_true")]
    pub wing_from_cwd: bool,
    /// 対象 wing が palace に無いとき `add_drawer` で初期シードする。
    #[serde(default = "default_true")]
    pub init_wing_if_missing: bool,
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    /// 接続方式（既定: Cursor と同じ MCP stdio）。
    #[serde(default)]
    pub protocol: MempalaceProtocol,
    /// Bearer トークン（HTTP モード任意）。
    pub api_key: Option<String>,
    /// MCP stdio の実行ファイル（既定: `python` / `py`）。
    pub command: Option<String>,
    /// MCP stdio の引数（既定: `["-m", "mempalace.mcp_server"]`）。
    pub args: Option<Vec<String>>,
}

fn default_true() -> bool {
    true
}

fn default_base_url() -> String {
    "http://127.0.0.1:8765".into()
}

fn default_agent_name() -> String {
    "harness-seed".into()
}

fn default_timeout_secs() -> u64 {
    30
}

impl Default for MempalaceConfig {
    fn default() -> Self {
        Self {
            base_url: default_base_url(),
            agent_name: default_agent_name(),
            wing: None,
            room: None,
            wing_from_cwd: true,
            init_wing_if_missing: true,
            timeout_secs: default_timeout_secs(),
            protocol: MempalaceProtocol::default(),
            api_key: None,
            command: None,
            args: None,
        }
    }
}

impl MempalaceConfig {
    pub fn from_env_or(this: Self) -> Self {
        let mut cfg = this;
        if let Ok(url) = std::env::var("HARNESS_SEED_MEMPALACE_URL")
            .or_else(|_| std::env::var("MEMPALACE_BASE_URL"))
        {
            if !url.trim().is_empty() {
                cfg.base_url = url;
            }
        }
        if let Ok(name) = std::env::var("HARNESS_SEED_MEMPALACE_AGENT")
            .or_else(|_| std::env::var("MEMPALACE_AGENT_NAME"))
        {
            if !name.trim().is_empty() {
                cfg.agent_name = name;
            }
        }
        if let Ok(cmd) = std::env::var("HARNESS_SEED_MEMPALACE_COMMAND")
            .or_else(|_| std::env::var("MEMPALACE_COMMAND"))
        {
            if !cmd.trim().is_empty() {
                cfg.command = Some(cmd);
            }
        }
        if let Ok(wing) = std::env::var("HARNESS_SEED_MEMPALACE_WING")
            .or_else(|_| std::env::var("MEMPALACE_WING"))
        {
            if !wing.trim().is_empty() {
                cfg.wing = Some(wing);
            }
        }
        if cfg.api_key.is_none() {
            cfg.api_key = std::env::var("MEMPALACE_API_KEY")
                .or_else(|_| std::env::var("HARNESS_SEED_MEMPALACE_API_KEY"))
                .ok()
                .filter(|s| !s.trim().is_empty());
        }
        cfg.apply_cwd_scope();
        cfg
    }

    /// 起動ディレクトリ（または明示 `wing`）でプロジェクト wing を確定する。
    ///
    /// - **wing** = `wing_{project}`（プロジェクト共有）
    /// - **room** = `agent_name`（エージェント固有。検索は wing 全体＝他エージェントと共有）
    pub fn apply_cwd_scope(&mut self) {
        if self.wing.as_ref().is_some_and(|w| w.starts_with("wing_")) {
            return;
        }

        let project = if let Some(w) = self.wing.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty())
        {
            let w = w.strip_prefix("wing_").unwrap_or(w);
            sanitize_wing_name(w)
        } else if self.wing_from_cwd {
            match wing_name_from_launch_dir() {
                Some(p) => p,
                None => return,
            }
        } else {
            return;
        };

        if project.is_empty() {
            return;
        }
        self.wing = Some(project_wing_name(&project));
    }

    /// エージェント固有 room（diary の書き込み先）。
    pub fn agent_room(&self) -> String {
        self.room
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(sanitize_wing_name)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| sanitize_wing_name(&self.agent_name))
    }

    pub fn scope_label(&self) -> String {
        let wing = self.wing.as_deref().unwrap_or("(all)");
        format!("wing={wing} room={}", self.agent_room())
    }

    pub fn validate(&self) -> Result<(), MempalaceError> {
        if self.agent_name.trim().is_empty() {
            return Err(MempalaceError::Config("agent_name is empty".into()));
        }
        match self.protocol {
            MempalaceProtocol::McpStdio => {
                let cmd = self.mcp_command();
                if cmd.trim().is_empty() {
                    return Err(MempalaceError::Config("MCP command is empty".into()));
                }
            }
            MempalaceProtocol::ToolsPath | MempalaceProtocol::McpJsonrpc => {
                if self.base_url.trim().is_empty() {
                    return Err(MempalaceError::Config("base_url is empty".into()));
                }
            }
        }
        Ok(())
    }

    pub fn timeout(&self) -> Duration {
        Duration::from_secs(self.timeout_secs.max(1))
    }

    pub fn mcp_command(&self) -> String {
        if let Some(cmd) = &self.command {
            if !cmd.trim().is_empty() {
                return cmd.clone();
            }
        }
        default_python_command()
    }

    pub fn mcp_args(&self) -> Vec<String> {
        if let Some(args) = &self.args {
            if !args.is_empty() {
                return args.clone();
            }
        }
        vec!["-m".into(), "mempalace.mcp_server".into()]
    }
}

fn default_python_command() -> String {
    if cfg!(windows) {
        // Prefer the same interpreter Cursor mcp.json often uses.
        let candidates = [
            r"C:\Python312\python.exe",
            r"C:\Python311\python.exe",
            "python",
            "py",
        ];
        for c in candidates {
            if c.contains('\\') {
                if std::path::Path::new(c).is_file() {
                    return c.to_string();
                }
            } else {
                return c.to_string();
            }
        }
        "python".into()
    } else {
        "python3".into()
    }
}

/// 起動ディレクトリ名を wing 名にする（`HARNESS_WORKSPACE` があればそちら優先）。
pub fn wing_name_from_launch_dir() -> Option<String> {
    let root = launch_dir()?;
    let name = root.file_name()?.to_string_lossy();
    let sanitized = sanitize_wing_name(&name);
    if sanitized.is_empty() {
        None
    } else {
        Some(sanitized)
    }
}

fn launch_dir() -> Option<PathBuf> {
    if let Ok(ws) = std::env::var("HARNESS_WORKSPACE") {
        let p = PathBuf::from(ws.trim());
        if !p.as_os_str().is_empty() {
            return Some(if p.is_absolute() {
                p
            } else {
                std::env::current_dir().ok()?.join(p)
            });
        }
    }
    std::env::current_dir().ok()
}

/// プロジェクト共有 wing 名（`wing_{project}`）。
pub fn project_wing_name(project: &str) -> String {
    let p = sanitize_wing_name(project).to_lowercase().replace(' ', "_");
    if p.starts_with("wing_") {
        p
    } else {
        format!("wing_{p}")
    }
}

/// mempalace の wing/room 名に使える文字だけ残す。
pub fn sanitize_wing_name(raw: &str) -> String {
    let mut out = String::new();
    for ch in raw.trim().chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
            out.push(ch);
        } else if ch == ' ' {
            out.push('_');
        }
        if out.len() >= 64 {
            break;
        }
    }
    out.trim_matches(|c| c == '.' || c == '_' || c == '-').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_path_chars() {
        assert_eq!(sanitize_wing_name("harness-seed"), "harness-seed");
        assert_eq!(sanitize_wing_name("my project"), "my_project");
        assert!(!sanitize_wing_name("a/b\\c").contains('/'));
        assert!(!sanitize_wing_name("a/b\\c").contains('\\'));
    }

    #[test]
    fn apply_cwd_scope_uses_explicit_wing() {
        let mut cfg = MempalaceConfig::default();
        cfg.wing = Some("OpenHarness".into());
        cfg.agent_name = "harness-seed".into();
        cfg.apply_cwd_scope();
        assert_eq!(cfg.wing.as_deref(), Some("wing_openharness"));
        assert_eq!(cfg.agent_room(), "harness-seed");
        assert_eq!(cfg.agent_name, "harness-seed");
    }

    #[test]
    fn apply_cwd_scope_idempotent() {
        let mut cfg = MempalaceConfig::default();
        cfg.wing = Some("proj".into());
        cfg.apply_cwd_scope();
        let once = cfg.wing.clone();
        cfg.apply_cwd_scope();
        assert_eq!(cfg.wing, once);
        assert_eq!(cfg.wing.as_deref(), Some("wing_proj"));
    }

    #[test]
    fn project_wing_name_prefixes() {
        assert_eq!(project_wing_name("harness-seed"), "wing_harness-seed");
        assert_eq!(project_wing_name("wing_already"), "wing_already");
    }

    #[test]
    fn wing_from_path() {
        let name = sanitize_wing_name(
            std::path::Path::new("/tmp/foo/harness-seed")
                .file_name()
                .unwrap()
                .to_str()
                .unwrap(),
        );
        assert_eq!(name, "harness-seed");
    }
}

/// 接続方式。
#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MempalaceProtocol {
    /// Cursor と同じ: `python -m mempalace.mcp_server` を stdio で起動。
    #[default]
    McpStdio,
    /// `POST {base}/tools/{name}` + arguments body.
    ToolsPath,
    /// MCP JSON-RPC `tools/call` を `POST {base}` に送る。
    McpJsonrpc,
}
