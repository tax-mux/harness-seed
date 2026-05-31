//! 組み込みツール実装（[`super::traits::Tool`]）。

use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::action::Observation;
use crate::brave_search::search_web;
use crate::grep::{grep_in_workspace, GrepOptions};

use super::traits::{Tool, ToolContext};

// --- workspace helpers (crate-private) ---

/// ファイル系ツールのルート。`HARNESS_WORKSPACE` または `TRIAGE_ROOT` で上書き可能。
pub fn workspace_root() -> PathBuf {
    for name in ["HARNESS_WORKSPACE", "TRIAGE_ROOT"] {
        if let Ok(root) = std::env::var(name) {
            let root = root.trim();
            if !root.is_empty() {
                return PathBuf::from(root);
            }
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

pub fn resolve_in_workspace(path: &str) -> Result<PathBuf, String> {
    let root = workspace_root()
        .canonicalize()
        .map_err(|e| format!("workspace root invalid: {e}"))?;

    if path.is_empty() || path == "." {
        return Ok(root);
    }

    let mut out = root.clone();
    for comp in Path::new(path).components() {
        match comp {
            Component::ParentDir => {
                if !out.pop() || !out.starts_with(&root) {
                    return Err(format!("path outside workspace: {path}"));
                }
            }
            Component::Normal(name) => out.push(name),
            Component::CurDir => {}
            Component::RootDir | Component::Prefix(_) => {
                return Err(format!("absolute path not allowed: {path}"));
            }
        }
    }
    Ok(out)
}

fn format_shell_output(output: &std::process::Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let code = output.status.code().unwrap_or(-1);
    format!(
        "exit_code={code}\n--- stdout ---\n{stdout}--- stderr ---\n{stderr}"
    )
}

// --- tools ---

pub struct EchoTool;

impl Tool for EchoTool {
    fn name(&self) -> &str {
        "echo"
    }

    fn spec(&self) -> &str {
        r#"args: { "message": "<string>" }"#
    }

    fn execute(&self, invoke_id: u64, args: &Value, _ctx: &ToolContext) -> Observation {
        let message = args
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("");
        Observation::success(invoke_id, message.to_string())
    }
}

pub struct TimeTool;

impl Tool for TimeTool {
    fn name(&self) -> &str {
        "time"
    }

    fn spec(&self) -> &str {
        "args: {}"
    }

    fn execute(&self, invoke_id: u64, _args: &Value, _ctx: &ToolContext) -> Observation {
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Observation::success(invoke_id, format!("unix_epoch_secs={secs}"))
    }
}

pub struct ListDirTool;

impl Tool for ListDirTool {
    fn name(&self) -> &str {
        "list_dir"
    }

    fn spec(&self) -> &str {
        r#"args: { "path": "<optional directory, default .>" }"#
    }

    fn execute(&self, invoke_id: u64, args: &Value, _ctx: &ToolContext) -> Observation {
        let path = args
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or(".");
        match resolve_in_workspace(path) {
            Ok(abs) => match std::fs::read_dir(&abs) {
                Ok(entries) => {
                    let mut names: Vec<String> = entries
                        .filter_map(|e| e.ok())
                        .map(|e| {
                            let name = e.file_name().to_string_lossy().into_owned();
                            if e.path().is_dir() {
                                format!("{name}/")
                            } else {
                                name
                            }
                        })
                        .collect();
                    names.sort();
                    Observation::success(invoke_id, names.join("\n"))
                }
                Err(err) => Observation::failure(invoke_id, format!("list_dir failed: {err}")),
            },
            Err(err) => Observation::failure(invoke_id, err),
        }
    }
}

pub struct GrepTool;

impl Tool for GrepTool {
    fn name(&self) -> &str {
        "grep"
    }

    fn spec(&self) -> &str {
        r#"args: { "pattern": "<string>", "path": "<optional dir/file, default .>", "ignore_case": <bool>, "glob": "<optional e.g. *.rs>", "max_results": <number> }"#
    }

    fn execute(&self, invoke_id: u64, args: &Value, _ctx: &ToolContext) -> Observation {
        let Some(pattern) = args.get("pattern").and_then(Value::as_str) else {
            return Observation::failure(invoke_id, "grep requires pattern");
        };
        let path = args.get("path").and_then(Value::as_str).unwrap_or(".");
        let ignore_case = args
            .get("ignore_case")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let max_results = args
            .get("max_results")
            .and_then(Value::as_u64)
            .map(|n| n as usize)
            .unwrap_or(200)
            .max(1)
            .min(2000);
        let glob = args
            .get("glob")
            .and_then(Value::as_str)
            .map(str::to_string);

        match resolve_in_workspace(path) {
            Ok(abs) => {
                let root = match workspace_root().canonicalize() {
                    Ok(r) => r,
                    Err(e) => {
                        return Observation::failure(
                            invoke_id,
                            format!("workspace root invalid: {e}"),
                        );
                    }
                };
                let opts = GrepOptions {
                    pattern: pattern.to_string(),
                    ignore_case,
                    max_results,
                    glob,
                };
                match grep_in_workspace(&root, &abs, &opts) {
                    Ok(out) => Observation::success(invoke_id, out),
                    Err(err) => Observation::failure(invoke_id, err),
                }
            }
            Err(err) => Observation::failure(invoke_id, err),
        }
    }
}

pub struct ReadFileTool;

impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }

    fn spec(&self) -> &str {
        r#"args: { "path": "<file path under project>" }"#
    }

    fn execute(&self, invoke_id: u64, args: &Value, _ctx: &ToolContext) -> Observation {
        let Some(path) = args.get("path").and_then(Value::as_str) else {
            return Observation::failure(invoke_id, "read_file requires path");
        };
        match resolve_in_workspace(path) {
            Ok(abs) => match std::fs::read_to_string(&abs) {
                Ok(text) => Observation::success(invoke_id, text),
                Err(err) => Observation::failure(invoke_id, format!("read_file failed: {err}")),
            },
            Err(err) => Observation::failure(invoke_id, err),
        }
    }
}

pub struct WriteFileTool;

impl Tool for WriteFileTool {
    fn name(&self) -> &str {
        "write_file"
    }

    fn spec(&self) -> &str {
        r#"args: { "path": "<file path>", "content": "<full file text>" }"#
    }

    fn execute(&self, invoke_id: u64, args: &Value, _ctx: &ToolContext) -> Observation {
        let Some(path) = args.get("path").and_then(Value::as_str) else {
            return Observation::failure(invoke_id, "write_file requires path");
        };
        let content = args
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or("");
        match resolve_in_workspace(path) {
            Ok(abs) => {
                if let Some(parent) = abs.parent() {
                    if let Err(err) = std::fs::create_dir_all(parent) {
                        return Observation::failure(
                            invoke_id,
                            format!("write_file mkdir failed: {err}"),
                        );
                    }
                }
                match std::fs::write(&abs, content) {
                    Ok(()) => Observation::success(
                        invoke_id,
                        format!("wrote {} bytes to {}", content.len(), path),
                    ),
                    Err(err) => {
                        Observation::failure(invoke_id, format!("write_file failed: {err}"))
                    }
                }
            }
            Err(err) => Observation::failure(invoke_id, err),
        }
    }
}

pub struct RunCmdTool;

impl Tool for RunCmdTool {
    fn name(&self) -> &str {
        "run_cmd"
    }

    fn spec(&self) -> &str {
        r#"args: { "command": "<shell command>", "cwd": "<optional dir>" }"#
    }

    fn execute(&self, invoke_id: u64, args: &Value, ctx: &ToolContext) -> Observation {
        let Some(command) = args.get("command").and_then(Value::as_str) else {
            return Observation::failure(invoke_id, "run_cmd requires command");
        };
        if command.trim().is_empty() {
            return Observation::failure(invoke_id, "run_cmd: empty command");
        }

        let cwd = match args.get("cwd").and_then(Value::as_str) {
            Some(p) => match resolve_in_workspace(p) {
                Ok(abs) => abs,
                Err(err) => return Observation::failure(invoke_id, err),
            },
            None => workspace_root(),
        };

        match ctx.env.run_shell_command(command, &cwd) {
            Ok(output) => Observation::success(invoke_id, format_shell_output(&output)),
            Err(err) => Observation::failure(invoke_id, err),
        }
    }
}

pub struct WebSearchTool;

impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }

    fn spec(&self) -> &str {
        r#"args: { "query": "<search string>", "count": <optional 1-20, default from config> } — requires tools.brave_search.api_key (Brave Search API)"#
    }

    fn execute(&self, invoke_id: u64, args: &Value, ctx: &ToolContext) -> Observation {
        let Some(query) = args.get("query").and_then(Value::as_str) else {
            return Observation::failure(invoke_id, "web_search requires query");
        };
        let count = args.get("count").and_then(Value::as_u64).map(|n| n as u8);
        let Some(cfg) = ctx.brave_search.as_ref() else {
            return Observation::failure(
                invoke_id,
                "web_search: configure tools.brave_search.api_key in config.json (or BRAVE_SEARCH_API_KEY)",
            );
        };
        match search_web(cfg, query, count) {
            Ok(text) => Observation::success(invoke_id, text),
            Err(err) => Observation::failure(invoke_id, err.to_string()),
        }
    }
}
