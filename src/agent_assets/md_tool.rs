//! `tools/*.md` frontmatter から宣言的シェルツールを読み込む（Anthropic 方式）。

use std::path::Path;

use super::frontmatter::{parse_frontmatter, FrontmatterDoc};
use super::script_tool::{ScriptTool, ScriptToolDefinition};

pub fn tool_from_markdown(path: &Path, text: &str) -> Result<ScriptTool, String> {
    let doc = parse_frontmatter(text).ok_or_else(|| "missing YAML frontmatter".to_string())?;
    let def = definition_from_doc(&doc)?;
    ScriptTool::from_definition(def).map_err(|e| format!("{}: {e}", path.display()))
}

fn definition_from_doc(doc: &FrontmatterDoc) -> Result<ScriptToolDefinition, String> {
    let name = required_str(&doc.meta, "name")?;
    let command = required_str(&doc.meta, "command")?;
    let summary = optional_str(&doc.meta, "description")
        .or_else(|| optional_str(&doc.meta, "summary"))
        .unwrap_or_default();
    let spec = optional_str(&doc.meta, "spec").unwrap_or_default();
    let cwd = optional_str(&doc.meta, "cwd");
    Ok(ScriptToolDefinition {
        name,
        summary,
        spec,
        command,
        cwd,
    })
}

fn required_str(meta: &serde_yaml::Value, key: &str) -> Result<String, String> {
    meta.get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("missing {key}"))
}

fn optional_str(meta: &serde_yaml::Value, key: &str) -> Option<String> {
    meta.get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::Tool;

    #[test]
    fn parses_tool_markdown() {
        let text = r#"---
name: agent_echo
description: Echo text
command: echo {text}
spec: 'args: { "text": "..." }'
---
"#;
        let tool = tool_from_markdown(Path::new("echo.md"), text).unwrap();
        assert_eq!(tool.name(), "agent_echo");
    }
}
