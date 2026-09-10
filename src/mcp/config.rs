//! MCP 設定の読み込み（Cursor `mcp.json` / OpenCode `mcp` 互換）。

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use serde::Deserialize;
use serde_json::Value;

use crate::config::McpToolsSection;

#[derive(Debug, Clone)]
pub struct McpServerConfig {
    pub name: String,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub url: Option<String>,
    pub headers: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct McpJsonRoot {
    #[serde(rename = "mcpServers", default)]
    mcp_servers: BTreeMap<String, McpJsonServer>,
}

#[derive(Debug, Deserialize)]
struct McpJsonServer {
    #[serde(default)]
    command: CommandField,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    url: Option<String>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    enabled: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(untagged)]
enum CommandField {
    #[default]
    None,
    One(String),
    Many(Vec<String>),
}

/// `${HOME}` / `~` を展開する。
pub fn expand_path(raw: &str) -> PathBuf {
    let trimmed = expand_env_str(raw).trim().to_string();
    if let Some(rest) = trimmed.strip_prefix("${HOME}") {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        return PathBuf::from(home).join(rest.trim_start_matches('/'));
    }
    if let Some(rest) = trimmed.strip_prefix('~') {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        return PathBuf::from(home).join(rest.trim_start_matches('/'));
    }
    PathBuf::from(trimmed)
}

/// `${VAR}` を環境変数で展開する（未設定なら空文字）。
pub fn expand_env_str(raw: &str) -> String {
    let mut result = String::new();
    let mut rest = raw;
    while let Some(start) = rest.find("${") {
        result.push_str(&rest[..start]);
        rest = &rest[start + 2..];
        if let Some(end) = rest.find('}') {
            let var = rest[..end].trim();
            let val = std::env::var(var).unwrap_or_default();
            result.push_str(&val);
            rest = &rest[end + 1..];
        } else {
            result.push_str("${");
            break;
        }
    }
    result.push_str(rest);
    result
}

pub fn resolved_sources(section: &McpToolsSection) -> Vec<PathBuf> {
    if section.sources.is_empty() {
        return vec![
            expand_path("${HOME}/.config/seed-agent/mcp.opencode.json"),
            expand_path("~/.cursor/mcp.json"),
        ];
    }
    section.sources.iter().map(|s| expand_path(s)).collect()
}

pub fn load_mcp_servers(section: &McpToolsSection) -> Vec<McpServerConfig> {
    if section.enabled == Some(false) {
        return vec![];
    }
    let allow: Option<Vec<String>> = if section.servers.is_empty() {
        None
    } else {
        Some(section.servers.clone())
    };

    let mut merged: BTreeMap<String, McpServerConfig> = BTreeMap::new();
    for path in resolved_sources(section) {
        if !path.is_file() {
            eprintln!("[mcp] skip missing source: {}", path.display());
            continue;
        }
        let text = match fs::read_to_string(&path) {
            Ok(t) => t,
            Err(err) => {
                eprintln!("[mcp] read {}: {err}", path.display());
                continue;
            }
        };
        let root: McpJsonRoot = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(err) => {
                eprintln!("[mcp] parse {}: {err}", path.display());
                continue;
            }
        };
        for (name, srv) in root.mcp_servers {
            if srv.enabled == Some(false) {
                continue;
            }
            if let Some(ref allow) = allow {
                if !allow.iter().any(|a| a == &name) {
                    continue;
                }
            }
            if let Some(cfg) = parse_server_entry(name.clone(), srv) {
                if let Some(existing) = merged.get_mut(&name) {
                    merge_server(existing, cfg);
                } else {
                    merged.insert(name, cfg);
                }
            }
        }
    }
    merged.into_values().collect()
}

fn merge_server(existing: &mut McpServerConfig, overlay: McpServerConfig) {
    if overlay.command.is_some() {
        existing.command = overlay.command;
    }
    if !overlay.args.is_empty() {
        existing.args = overlay.args;
    }
    if overlay.url.is_some() {
        existing.url = overlay.url;
    }
    existing.env.extend(overlay.env);
    existing.headers.extend(overlay.headers);
}

fn parse_server_entry(name: String, srv: McpJsonServer) -> Option<McpServerConfig> {
    let (command, args) = normalize_command(&srv.command, srv.args).unwrap_or((None, vec![]));
    let has_stdio = command.is_some();
    let has_url = srv.url.is_some();
    let mut env = BTreeMap::new();
    for (k, v) in srv.env.into_iter().chain(srv.environment) {
        env.insert(k, expand_env_str(&v));
    }
    let mut headers = BTreeMap::new();
    for (k, v) in srv.headers {
        headers.insert(k, expand_env_str(&v));
    }
    if !has_stdio && !has_url {
        if env.is_empty() && headers.is_empty() {
            eprintln!("[mcp] skip {name}: no command or url");
            return None;
        }
        return Some(McpServerConfig {
            name,
            command: None,
            args: vec![],
            env,
            url: None,
            headers,
        });
    }
    Some(McpServerConfig {
        name,
        command: command.map(|c| expand_env_str(&c)),
        args: args.into_iter().map(|a| expand_env_str(&a)).collect(),
        env,
        url: srv.url.map(|u| expand_env_str(&u)),
        headers,
    })
}

fn normalize_command(command: &CommandField, extra_args: Vec<String>) -> Option<(Option<String>, Vec<String>)> {
    match command {
        CommandField::None => {
            if extra_args.is_empty() {
                None
            } else {
                Some((None, extra_args))
            }
        }
        CommandField::One(c) => Some((Some(c.clone()), extra_args)),
        CommandField::Many(v) if v.is_empty() => None,
        CommandField::Many(v) => {
            let mut parts = v.clone();
            let cmd = parts.remove(0);
            parts.extend(extra_args);
            Some((Some(cmd), parts))
        }
    }
}

pub fn timeout_for(section: &McpToolsSection) -> std::time::Duration {
    std::time::Duration::from_secs(section.timeout_secs.unwrap_or(60).max(5))
}

pub fn qualified_tool_name(server: &str, tool: &str) -> String {
    let s = sanitize_id(server);
    let t = sanitize_id(tool);
    format!("mcp_{s}_{t}")
}

fn sanitize_id(raw: &str) -> String {
    raw.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

pub fn spec_from_schema(schema: &Value, description: Option<&str>) -> String {
    if let Some(desc) = description.filter(|d| !d.is_empty()) {
        return format!("args: {desc}");
    }
    if let Some(desc) = schema.get("description").and_then(|d| d.as_str()) {
        if !desc.is_empty() {
            return format!("args: {desc}");
        }
    }
    if let Some(props) = schema.get("properties").and_then(|p| p.as_object()) {
        let keys: Vec<String> = props.keys().cloned().collect();
        if !keys.is_empty() {
            return format!("args: {{ {} }}", keys.join(", "));
        }
    }
    "args: {}".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qualified_name_sanitizes() {
        assert_eq!(
            qualified_tool_name("mcp-redmine", "redmine_issues"),
            "mcp_mcp_redmine_redmine_issues"
        );
    }

    #[test]
    fn expands_home() {
        let p = expand_path("${HOME}/.cursor/mcp.json");
        assert!(p.to_string_lossy().contains(".cursor"));
    }

    #[test]
    fn normalizes_command_array() {
        let field = CommandField::Many(vec![
            "npx".into(),
            "-y".into(),
            "@pavelsmith/redmine-mcp".into(),
        ]);
        let (cmd, args) = normalize_command(&field, vec![]).unwrap();
        assert_eq!(cmd.as_deref(), Some("npx"));
        assert_eq!(args, vec!["-y", "@pavelsmith/redmine-mcp"]);
    }
}
