use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::path::Component;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::action::{Action, Observation};
use crate::brave_search::{search_web, BraveSearchConfig};
use crate::grep::{grep_in_workspace, GrepOptions};
use crate::runtime::RuntimeEnvironment;

pub const HELP_TEXT: &str = "\
利用可能なコマンド:
  help          このヘルプ
  time          現在時刻を取得（ツール経由）
  echo <text>   テキストをエコー（ツール経由）
  quit / exit   終了

LLM モードでは read_file / write_file / run_cmd などでコードの読み書き・実行ができます。

上記以外の入力は ReAct ループ（Thought → Action → Answer）で処理します。";

/// ツール実行ランタイム。
#[derive(Debug, Clone)]
pub struct ToolRuntime {
    next_invoke_id: u64,
    env: RuntimeEnvironment,
    brave_search: Option<BraveSearchConfig>,
}

impl Default for ToolRuntime {
    fn default() -> Self {
        Self::with_environment(RuntimeEnvironment::detect())
    }
}

impl ToolRuntime {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_environment(env: RuntimeEnvironment) -> Self {
        Self::with_environment_and_brave(env, None)
    }

    pub fn with_environment_and_brave(
        env: RuntimeEnvironment,
        brave_search: Option<BraveSearchConfig>,
    ) -> Self {
        Self {
            next_invoke_id: 0,
            env,
            brave_search,
        }
    }

    pub fn set_brave_search(&mut self, brave_search: Option<BraveSearchConfig>) {
        self.brave_search = brave_search;
    }

    pub fn environment(&self) -> &RuntimeEnvironment {
        &self.env
    }

    fn next_id(&mut self) -> u64 {
        self.next_invoke_id += 1;
        self.next_invoke_id
    }

    pub fn execute(&mut self, tool: &str, args: &Value) -> (u64, Observation) {
        let invoke_id = self.next_id();
        let observation = match tool {
            "echo" => Self::run_echo(invoke_id, args),
            "time" => Self::run_time(invoke_id),
            "list_dir" => Self::run_list_dir(invoke_id, args),
            "grep" => Self::run_grep(invoke_id, args),
            "read_file" => Self::run_read_file(invoke_id, args),
            "write_file" => Self::run_write_file(invoke_id, args),
            "run_cmd" => self.run_cmd(invoke_id, args),
            "web_search" => self.run_web_search(invoke_id, args),
            _ => Observation::failure(invoke_id, format!("unknown tool: {tool}")),
        };
        (invoke_id, observation)
    }

    fn run_echo(invoke_id: u64, args: &Value) -> Observation {
        let message = args
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("");
        Observation::success(invoke_id, message.to_string())
    }

    fn run_time(invoke_id: u64) -> Observation {
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Observation::success(invoke_id, format!("unix_epoch_secs={secs}"))
    }

    fn run_list_dir(invoke_id: u64, args: &Value) -> Observation {
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

    fn run_web_search(&self, invoke_id: u64, args: &Value) -> Observation {
        let Some(query) = args.get("query").and_then(Value::as_str) else {
            return Observation::failure(invoke_id, "web_search requires query");
        };
        let count = args.get("count").and_then(Value::as_u64).map(|n| n as u8);
        let Some(cfg) = self.brave_search.as_ref() else {
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

    fn run_grep(invoke_id: u64, args: &Value) -> Observation {
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

    fn run_read_file(invoke_id: u64, args: &Value) -> Observation {
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

    fn run_write_file(invoke_id: u64, args: &Value) -> Observation {
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

    fn run_cmd(&self, invoke_id: u64, args: &Value) -> Observation {
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

        match self.env.run_shell_command(command, &cwd) {
            Ok(output) => Observation::success(invoke_id, format_shell_output(&output)),
            Err(err) => Observation::failure(invoke_id, err),
        }
    }
}

fn format_shell_output(output: &std::process::Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let code = output.status.code().unwrap_or(-1);
    format!(
        "exit_code={code}\n--- stdout ---\n{stdout}--- stderr ---\n{stderr}"
    )
}

/// クレートルート（プロジェクトディレクトリ）。
pub fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// 相対パスをワークスペース内に解決する（`..` による脱出は拒否。未作成ファイルも可）。
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

/// Action を実行して Observation を返す（invoke_id は Action 側を優先）。
pub fn execute_action(runtime: &mut ToolRuntime, action: &Action) -> Observation {
    let (_, mut obs) = runtime.execute(&action.tool, &action.args);
    obs.invoke_id = action.invoke_id;
    obs
}

pub fn tools_catalog() -> HashMap<&'static str, &'static str> {
    HashMap::from([
        ("echo", r#"args: { "message": "<string>" }"#),
        ("time", "args: {}"),
        (
            "list_dir",
            r#"args: { "path": "<optional directory, default .>" }"#,
        ),
        (
            "grep",
            r#"args: { "pattern": "<string>", "path": "<optional dir/file, default .>", "ignore_case": <bool>, "glob": "<optional e.g. *.rs>", "max_results": <number> }"#,
        ),
        ("read_file", r#"args: { "path": "<file path under project>" }"#),
        (
            "write_file",
            r#"args: { "path": "<file path>", "content": "<full file text>" }"#,
        ),
        (
            "run_cmd",
            r#"args: { "command": "<shell command>", "cwd": "<optional dir>" }"#,
        ),
        (
            "web_search",
            r#"args: { "query": "<search string>", "count": <optional 1-20, default from config> } — requires tools.brave_search.api_key (Brave Search API)"#,
        ),
    ])
}

pub fn echo_action(invoke_id: u64, message: &str) -> Action {
    Action::new(invoke_id, "echo", json!({ "message": message }))
}

pub fn time_action(invoke_id: u64) -> Action {
    Action::new(invoke_id, "time", json!({}))
}

pub fn list_dir_action(invoke_id: u64, path: &str) -> Action {
    Action::new(invoke_id, "list_dir", json!({ "path": path }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn list_dir_lists_cwd() {
        let mut rt = ToolRuntime::new();
        let obs = rt.execute("list_dir", &json!({})).1;
        assert!(obs.ok);
        assert!(obs.output.contains("Cargo.toml"));
    }

    #[test]
    fn write_and_read_file_roundtrip() {
        let rel = "tmp/test_tool_roundtrip.txt";
        let abs = resolve_in_workspace(rel).unwrap();
        let _ = fs::remove_file(&abs);

        let mut rt = ToolRuntime::new();
        let write = rt
            .execute(
                "write_file",
                &json!({ "path": rel, "content": "fn main() {}\n" }),
            )
            .1;
        assert!(write.ok, "{}", write.output);

        let read = rt.execute("read_file", &json!({ "path": rel })).1;
        assert!(read.ok);
        assert_eq!(read.output, "fn main() {}\n");

        let _ = fs::remove_file(&abs);
    }

    #[test]
    fn rejects_path_outside_workspace() {
        assert!(resolve_in_workspace("..").is_err());
        assert!(resolve_in_workspace("tmp/../../outside.txt").is_err());
    }

    #[test]
    fn grep_finds_in_src() {
        let mut rt = ToolRuntime::new();
        let obs = rt
            .execute(
                "grep",
                &json!({
                    "pattern": "pub struct ReActLoop",
                    "path": "src",
                    "glob": "*.rs",
                    "max_results": 5
                }),
            )
            .1;
        assert!(obs.ok, "{}", obs.output);
        assert!(obs.output.contains("react.rs"));
    }

    #[test]
    fn run_cmd_echo() {
        let mut rt = ToolRuntime::new();
        #[cfg(windows)]
        let command = "echo hello_cmd";
        #[cfg(not(windows))]
        let command = "echo hello_cmd";
        let obs = rt.execute("run_cmd", &json!({ "command": command })).1;
        assert!(obs.ok, "{}", obs.output);
        assert!(obs.output.contains("hello_cmd"));
    }
}
