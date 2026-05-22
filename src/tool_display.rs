//! ツール実行のコマンド・結果をターミナルに表示する。

use crate::action::{Action, Observation};

const MAX_OUTPUT_CHARS: usize = 8_000;

/// `run_cmd` などの実行内容を stderr に出す（`react.show_tool_output`）。
pub fn eprintln_tool_execution(action: &Action, observation: &Observation) {
    eprintln!("\n--- tool: {} ---", action.tool);
    eprintln_tool_args(action);
    let status = if observation.ok { "ok" } else { "err" };
    eprintln_tool_output_body(&observation.output);
    eprintln!("--- end tool ({status}) ---\n");
}

fn eprintln_tool_args(action: &Action) {
    match action.tool.as_str() {
        "run_cmd" => {
            if let Some(cmd) = action.args.get("command").and_then(|v| v.as_str()) {
                eprintln!("$ {cmd}");
            }
            if let Some(cwd) = action.args.get("cwd").and_then(|v| v.as_str()) {
                if !cwd.is_empty() {
                    eprintln!("  (cwd: {cwd})");
                }
            }
        }
        "web_search" => {
            if let Some(q) = action.args.get("query").and_then(|v| v.as_str()) {
                eprintln!("query: {q}");
            }
            if let Some(c) = action.args.get("count") {
                eprintln!("count: {c}");
            }
        }
        "read_file" | "write_file" | "list_dir" | "grep" => {
            if let Ok(s) = serde_json::to_string(&action.args) {
                eprintln!("args: {s}");
            }
        }
        _ => {
            if let Ok(s) = serde_json::to_string(&action.args) {
                eprintln!("args: {s}");
            }
        }
    }
}

fn eprintln_tool_output_body(output: &str) {
    if output.is_empty() {
        eprintln!("(no output)");
        return;
    }
    let truncated = output.chars().count() > MAX_OUTPUT_CHARS;
    let shown: String = if truncated {
        output.chars().take(MAX_OUTPUT_CHARS).collect()
    } else {
        output.to_string()
    };
    for line in shown.lines() {
        eprintln!("{line}");
    }
    if truncated {
        eprintln!("... (output truncated, {} chars total)", output.chars().count());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn formats_run_cmd_without_panic() {
        let action = Action::new(1, "run_cmd", json!({ "command": "node -v" }));
        let obs = Observation::success(1, "v22.20.0\n");
        eprintln_tool_execution(&action, &obs);
    }
}
