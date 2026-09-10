//! 宣言的 JSON から `run_cmd` 相当のシェルツールを登録する。

use std::path::PathBuf;

use serde::Deserialize;
use serde_json::Value;

use crate::action::Observation;
use crate::tasks::apply_template;
use crate::tool::{resolve_in_workspace, workspace_root, Tool, ToolContext};

/// `tools/*.json` のスキーマ。
#[derive(Debug, Clone, Deserialize)]
pub struct ScriptToolDefinition {
    pub name: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub spec: String,
    pub command: String,
    #[serde(default)]
    pub cwd: Option<String>,
}

impl ScriptToolDefinition {
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("missing name".into());
        }
        if self.command.trim().is_empty() {
            return Err(format!("tool '{}': missing command", self.name));
        }
        Ok(())
    }

    pub fn resolved_spec(&self) -> String {
        if !self.spec.trim().is_empty() {
            return self.spec.trim().to_string();
        }
        if !self.summary.trim().is_empty() {
            return format!("{} — command: {}", self.summary.trim(), self.command.trim());
        }
        format!("args: JSON object — runs: {}", self.command.trim())
    }
}

/// テンプレート付きシェルコマンドを実行する宣言的ツール。
pub struct ScriptTool {
    def: ScriptToolDefinition,
    cwd: PathBuf,
}

impl ScriptTool {
    pub fn from_definition(def: ScriptToolDefinition) -> Result<Self, String> {
        def.validate()?;
        let cwd = match def.cwd.as_deref() {
            Some(path) => resolve_in_workspace(path)?,
            None => workspace_root(),
        };
        Ok(Self { def, cwd })
    }
}

impl Tool for ScriptTool {
    fn name(&self) -> &str {
        &self.def.name
    }

    fn spec(&self) -> &str {
        // spec() returns &str but we compute dynamically — store in def.resolved at load time
        // Use leak pattern or store owned spec in struct
        self.def.spec.as_str()
    }

    fn execute(&self, invoke_id: u64, args: &Value, ctx: &ToolContext) -> Observation {
        let command = apply_template(&self.def.command, args);
        if command.trim().is_empty() {
            return Observation::failure(invoke_id, "empty command after template expansion");
        }
        match ctx.env.run_shell_command(&command, &self.cwd) {
            Ok(output) => Observation::success(invoke_id, format_shell_output(&output)),
            Err(err) => Observation::failure(invoke_id, err),
        }
    }
}

fn format_shell_output(output: &std::process::Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut out = String::new();
    if !stdout.is_empty() {
        out.push_str(stdout.trim_end());
    }
    if !stderr.is_empty() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str("stderr:\n");
        out.push_str(stderr.trim_end());
    }
    if !output.status.success() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&format!("exit code: {:?}", output.status.code()));
    }
    if out.is_empty() {
        out.push_str("(no output)");
    }
    out
}
