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

/// 思考表示用の一行（ツール本文は出さない）。
pub fn eprintln_tool_summary(label: &str, action: &Action, observation: &Observation) {
    let status = if observation.ok { "ok" } else { "err" };
    let args = compact_args(action);
    if args.is_empty() {
        eprintln!("[{label}] {status} {}", action.tool);
    } else {
        eprintln!("[{label}] {status} {} {args}", action.tool);
    }
}

/// ツール失敗時の本文を stderr に出す（成功時の stdout は出さない）。
pub fn eprintln_tool_error(label: &str, action: &Action, observation: &Observation) {
    if observation.ok {
        return;
    }
    eprintln_tool_summary(label, action, observation);
    let body = observation.output.trim();
    if body.is_empty() {
        eprintln!("[{label}] error: (no message)");
        return;
    }
    const MAX_ERR: usize = 2_000;
    let truncated = body.chars().count() > MAX_ERR;
    let shown: String = if truncated {
        body.chars().take(MAX_ERR).collect()
    } else {
        body.to_string()
    };
    for line in shown.lines() {
        eprintln!("[{label}] error: {line}");
    }
    if truncated {
        eprintln!(
            "[{label}] error: ... (truncated, {} chars total)",
            body.chars().count()
        );
    }
}

pub fn eprintln_thought(label: &str, thought: &str) {
    let body = thought.trim();
    if body.is_empty() {
        return;
    }
    eprintln!("[{label}] thought: {body}");
}

/// 計画層の LLM Answer（計画 JSON 等）を stdout へ。`react.show_thinking` 用。
pub fn println_plan_llm_output(kind: &str, body: &str) {
    let body = body.trim();
    if body.is_empty() {
        return;
    }
    const MAX: usize = 6_000;
    let truncated = body.chars().count() > MAX;
    let shown: String = if truncated {
        body.chars().take(MAX).collect()
    } else {
        body.to_string()
    };
    for line in shown.lines() {
        println!("[plan] {kind}: {line}");
    }
    if truncated {
        println!(
            "[plan] {kind}: ... (truncated, {} chars total)",
            body.chars().count()
        );
    }
}

fn compact_args(action: &Action) -> String {
    let preview = match action.tool.as_str() {
        "run_cmd" => action
            .args
            .get("command")
            .and_then(|v| v.as_str())
            .map(|s| s.chars().take(80).collect::<String>()),
        "web_search" => action
            .args
            .get("query")
            .and_then(|v| v.as_str())
            .map(|s| s.chars().take(80).collect::<String>()),
        _ => serde_json::to_string(&action.args)
            .ok()
            .map(|s| s.chars().take(120).collect::<String>()),
    };
    preview.unwrap_or_default()
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
        eprintln!(
            "... (output truncated, {} chars total)",
            output.chars().count()
        );
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

    #[test]
    fn plan_llm_output_truncates_long_body() {
        let long = "あ".repeat(6_050);
        // smoke: must not panic
        println_plan_llm_output("answer", &long);
        println_plan_llm_output("thought", "short plan note");
    }
}
