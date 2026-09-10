//! 実行時ディレクトリの `config.agent.json`（または CLI 指定）を解決する。

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

pub const DEFAULT_FILENAME: &str = "config.agent.json";

/// プロジェクトのエージェント資産レイアウト（`config.agent.json`）。
#[derive(Debug, Clone, Deserialize)]
pub struct AgentProjectConfig {
    /// ファイル系ツールのワークスペース（`list_dir` / `run_cmd` 等）。既定: 設定ファイルと同じディレクトリ。
    #[serde(default = "default_dot")]
    pub workspace: String,
    /// rules / skills / tools を含むディレクトリ。既定: `.agent`。
    #[serde(default = "default_agent_dir")]
    pub agent_dir: String,
    /// スコープ付き画像・recalled ファイル一覧 JSON（ワークスペース相対）。
    #[serde(default)]
    pub context_manifest: Option<String>,
}

fn default_dot() -> String {
    ".".into()
}

fn default_agent_dir() -> String {
    ".agent".into()
}

impl Default for AgentProjectConfig {
    fn default() -> Self {
        Self {
            workspace: default_dot(),
            agent_dir: default_agent_dir(),
            context_manifest: None,
        }
    }
}

impl AgentProjectConfig {
    pub fn resolve_paths(&self, base_dir: &Path) -> Result<ResolvedAgentPaths, AgentConfigError> {
        let workspace = resolve_under_base(base_dir, &self.workspace)?;
        let agent_dir = resolve_under_base(base_dir, &self.agent_dir)?;
        Ok(ResolvedAgentPaths {
            workspace,
            agent_dir,
        })
    }

    pub fn resolved_context_manifest_path(
        &self,
        base_dir: &Path,
    ) -> Result<Option<PathBuf>, AgentConfigError> {
        match &self.context_manifest {
            None => Ok(None),
            Some(rel) if rel.trim().is_empty() => Ok(None),
            Some(rel) => Ok(Some(resolve_under_base(base_dir, rel)?)),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ResolvedAgentPaths {
    pub workspace: PathBuf,
    pub agent_dir: PathBuf,
}

#[derive(Debug)]
pub enum AgentConfigError {
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    Parse {
        path: PathBuf,
        source: serde_json::Error,
    },
    Path {
        path: PathBuf,
        reason: String,
    },
}

impl fmt::Display for AgentConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, source } => {
                write!(f, "failed to read {}: {source}", path.display())
            }
            Self::Parse { path, source } => {
                write!(f, "failed to parse {}: {source}", path.display())
            }
            Self::Path { path, reason } => {
                write!(f, "{}: {reason}", path.display())
            }
        }
    }
}

impl std::error::Error for AgentConfigError {}

/// CLI 引数からエージェント設定ファイルを解決する。
///
/// 優先順: `--config-agent` → `--agent-dir`（合成）→ `./config.agent.json`。
pub fn resolve_cli_agent_config(args: &[String], cwd: &Path) -> Option<CliAgentSource> {
    for (i, arg) in args.iter().enumerate() {
        if arg == "--config-agent" {
            if let Some(path) = args.get(i + 1) {
                return Some(CliAgentSource::ConfigFile(PathBuf::from(path)));
            }
        }
        if arg == "--agent-dir" {
            if let Some(path) = args.get(i + 1) {
                return Some(CliAgentSource::AgentDirOnly(PathBuf::from(path)));
            }
        }
    }
    let default_path = cwd.join(DEFAULT_FILENAME);
    if default_path.is_file() {
        return Some(CliAgentSource::ConfigFile(default_path));
    }
    None
}

#[derive(Debug, Clone)]
pub enum CliAgentSource {
    ConfigFile(PathBuf),
    /// `--agent-dir` のみ。`workspace` は設定ファイルの親（通常 cwd）。
    AgentDirOnly(PathBuf),
}

impl CliAgentSource {
    pub fn base_dir(&self, cwd: &Path) -> PathBuf {
        match self {
            Self::ConfigFile(path) => path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(cwd)
                .to_path_buf(),
            Self::AgentDirOnly(_) => cwd.to_path_buf(),
        }
    }

    pub fn load(&self, cwd: &Path) -> Result<(AgentProjectConfig, PathBuf), AgentConfigError> {
        match self {
            Self::ConfigFile(path) => {
                let config = load_agent_project_file(path)?;
                Ok((config, path.clone()))
            }
            Self::AgentDirOnly(agent_dir) => {
                let config = AgentProjectConfig {
                    workspace: ".".into(),
                    agent_dir: agent_dir.to_string_lossy().into_owned(),
                    context_manifest: None,
                };
                let synthetic = cwd.join(DEFAULT_FILENAME);
                Ok((config, synthetic))
            }
        }
    }
}

pub fn load_agent_project_file(path: &Path) -> Result<AgentProjectConfig, AgentConfigError> {
    let text = fs::read_to_string(path).map_err(|source| AgentConfigError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    serde_json::from_str(&text).map_err(|source| AgentConfigError::Parse {
        path: path.to_path_buf(),
        source,
    })
}

pub fn apply_workspace_env(workspace: &Path) {
    if let Some(value) = workspace.to_str() {
        if !value.is_empty() {
            // SAFETY: CLI 起動直後・他スレッド未使用の前提。
            unsafe { std::env::set_var("HARNESS_WORKSPACE", value) };
        }
    }
}

fn resolve_under_base(base_dir: &Path, rel: &str) -> Result<PathBuf, AgentConfigError> {
    let path = if Path::new(rel).is_absolute() {
        PathBuf::from(rel)
    } else {
        base_dir.join(rel)
    };
    path.canonicalize().map_err(|source| AgentConfigError::Path {
        path: path.clone(),
        reason: source.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn resolve_cli_prefers_config_agent_flag() {
        let args = vec![
            "--config-agent".into(),
            "/tmp/proj/config.agent.json".into(),
        ];
        let src = resolve_cli_agent_config(&args, Path::new("/cwd")).unwrap();
        match src {
            CliAgentSource::ConfigFile(p) => assert_eq!(p, PathBuf::from("/tmp/proj/config.agent.json")),
            _ => panic!("expected config file"),
        }
    }

    #[test]
    fn resolve_cli_agent_dir_shortcut() {
        let args = vec!["--agent-dir".into(), ".agent".into()];
        let src = resolve_cli_agent_config(&args, Path::new("/cwd")).unwrap();
        match src {
            CliAgentSource::AgentDirOnly(p) => assert_eq!(p, PathBuf::from(".agent")),
            _ => panic!("expected agent dir"),
        }
    }

    #[test]
    fn load_agent_project_file_parses_defaults() {
        let dir = std::env::temp_dir().join(format!("hs-agent-cfg-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.agent.json");
        fs::write(&path, r#"{ "agent_dir": "my-agent" }"#).unwrap();
        let cfg = load_agent_project_file(&path).unwrap();
        assert_eq!(cfg.agent_dir, "my-agent");
        assert_eq!(cfg.workspace, ".");
        let _ = fs::remove_dir_all(&dir);
    }
}
