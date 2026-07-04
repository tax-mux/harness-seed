//! プロジェクトのエージェント資産（`.agent` 等）の読み込み。
//!
//! CLI では `config.agent.json` または `--agent-dir` / `--config-agent` で指定する。

mod config_agent;
mod loader;
mod script_tool;

pub use config_agent::{
    apply_workspace_env, load_agent_project_file, resolve_cli_agent_config, AgentConfigError,
    AgentProjectConfig, CliAgentSource, DEFAULT_FILENAME,
};
pub use loader::{apply_agent_project, load_agent_assets, AgentLoadError, AgentLoadReport};
pub use script_tool::{ScriptTool, ScriptToolDefinition};
