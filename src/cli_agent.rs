//! harness-seed CLI 起動時のエージェント資産（`config.agent.json`）適用。
//!
//! ここはヘルパーであり、登録の正本は [`crate::seed::SeedBuilder`] である。

use std::path::Path;
use std::path::PathBuf;

use crate::agent_assets::{
    apply_workspace_env, resolve_cli_agent_config, AgentLoadError, AgentLoadReport,
};
use crate::seed::SeedBuilder;

pub struct AgentCliSetup {
    pub config_label: PathBuf,
    pub report: AgentLoadReport,
}

/// ワークスペース環境変数だけ先に適用する（`load_prompt_blocks` より前に呼ぶ）。
pub fn prepare_cli_agent_workspace(
    args: &[String],
    cwd: &Path,
) -> Result<Option<PathBuf>, AgentLoadError> {
    let Some(source) = resolve_cli_agent_config(args, cwd) else {
        return Ok(None);
    };
    let base = source.base_dir(cwd);
    let (config, label) = source.load(cwd)?;
    let paths = config.resolve_paths(&base)?;
    apply_workspace_env(&paths.workspace);
    Ok(Some(label))
}

/// CLI 引数から agent 設定を解決し、あれば [`SeedBuilder`] へマージする。
pub fn merge_cli_agent(
    args: &[String],
    cwd: &Path,
    builder: SeedBuilder,
) -> Result<(SeedBuilder, Option<AgentCliSetup>), AgentLoadError> {
    let Some(source) = resolve_cli_agent_config(args, cwd) else {
        return Ok((builder, None));
    };

    let base = source.base_dir(cwd);
    let (config, config_label) = source.load(cwd)?;
    let builder = builder.merge_agent_project(&config, &base)?;
    let report = builder
        .agent_report()
        .cloned()
        .unwrap_or_default();

    Ok((
        builder,
        Some(AgentCliSetup {
            config_label,
            report,
        }),
    ))
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
