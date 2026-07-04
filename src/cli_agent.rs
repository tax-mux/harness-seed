//! harness-seed CLI 起動時のエージェント資産（`config.agent.json`）適用。

use std::path::Path;
use std::path::PathBuf;

use crate::agent_assets::{
    apply_agent_project, apply_workspace_env, resolve_cli_agent_config, AgentLoadReport,
};
use crate::context::PromptBlocks;
use crate::tasks::TaskRegistry;
use crate::tool::Tool;

pub struct AgentCliSetup {
    pub config_label: PathBuf,
    pub report: AgentLoadReport,
    pub script_tools: Vec<Box<dyn Tool>>,
}

/// ワークスペース環境変数だけ先に適用する（`load_prompt_blocks` より前に呼ぶ）。
pub fn prepare_cli_agent_workspace(
    args: &[String],
    cwd: &Path,
) -> Result<Option<PathBuf>, crate::agent_assets::AgentLoadError> {
    let Some(source) = resolve_cli_agent_config(args, cwd) else {
        return Ok(None);
    };
    let base = source.base_dir(cwd);
    let (config, label) = source.load(cwd)?;
    let paths = config.resolve_paths(&base)?;
    apply_workspace_env(&paths.workspace);
    Ok(Some(label))
}

/// `config.agent.json` / `--agent-dir` / `--config-agent` を解決し、あれば資産を読み込む。
pub fn setup_cli_agent(
    args: &[String],
    cwd: &Path,
    blocks: &mut PromptBlocks,
    task_registry: &mut TaskRegistry,
) -> Result<Option<AgentCliSetup>, crate::agent_assets::AgentLoadError> {
    let Some(source) = resolve_cli_agent_config(args, cwd) else {
        return Ok(None);
    };

    let base = source.base_dir(cwd);
    let (config, config_label) = source.load(cwd)?;
    let (report, script_tools) = apply_agent_project(&config, &base, blocks, task_registry)?;

    Ok(Some(AgentCliSetup {
        config_label,
        report,
        script_tools,
    }))
}

pub fn log_agent_setup(setup: &AgentCliSetup) {
    eprintln!(
        "agent: {} (workspace: {}, rules: {} file(s), skill tasks: {}, skill docs: {}, script tools: {})",
        setup.config_label.display(),
        setup.report.workspace.display(),
        setup.report.rules_files,
        setup.report.skill_tasks,
        setup.report.skill_docs,
        setup.report.script_tools,
    );
}

/// CLI フラグの値引数を消費するか。
pub fn cli_flag_takes_value(arg: &str) -> bool {
    matches!(arg, "--config" | "--config-agent" | "--agent-dir")
}

/// plan-zone 等でユーザー入力とみなさないグローバルフラグ。
pub fn is_cli_global_flag(arg: &str) -> bool {
    matches!(
        arg,
        "--no-llm"
            | "--llm"
            | "-v"
            | "--verbose"
            | "--show-prompt"
            | "--json"
            | "--no-monitor"
            | "--config-agent"
            | "--agent-dir"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_flag_takes_value_includes_agent_flags() {
        assert!(cli_flag_takes_value("--config-agent"));
        assert!(cli_flag_takes_value("--agent-dir"));
    }
}
