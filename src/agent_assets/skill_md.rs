//! `SKILL.md` frontmatter から計画層タスクを生成する（Anthropic 方式）。

use serde_json::json;

use crate::tasks::{TaskDefinition, TaskLoadError};

use super::frontmatter::FrontmatterDoc;

/// `task.json` が無いとき、`SKILL.md` の frontmatter からタスクを組み立てる。
pub fn task_from_skill_md(path: &std::path::Path, doc: &FrontmatterDoc) -> Result<TaskDefinition, TaskLoadError> {
    let name = doc
        .meta
        .get("name")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| TaskLoadError::Invalid {
            path: path.to_path_buf(),
            reason: "SKILL.md frontmatter missing name".into(),
        })?;

    let description = doc
        .meta
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or(name)
        .trim()
        .to_string();

    let harness = doc.meta.get("harness").cloned().unwrap_or(serde_yaml::Value::Null);

    let id = harness
        .get("id")
        .or_else(|| harness.get("task_id"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| slug_to_task_id(name));

    let summary = harness
        .get("summary")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| first_line(&description).to_string());

    let planner_summary = harness
        .get("planner_summary")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| description.clone());

    let done_when = harness
        .get("done_when")
        .and_then(|v| v.as_str())
        .unwrap_or("user request satisfied")
        .to_string();

    let react_only = harness
        .get("react_only")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let default_params = harness
        .get("default_params")
        .cloned()
        .map(yaml_to_json)
        .unwrap_or_else(|| json!({}));

    let tool_policy = harness
        .get("tool_policy")
        .cloned()
        .map(|v| serde_json::from_value(yaml_to_json(v)))
        .transpose()
        .map_err(|e| TaskLoadError::Invalid {
            path: path.to_path_buf(),
            reason: format!("invalid tool_policy: {e}"),
        })?
        .unwrap_or_default();

    Ok(TaskDefinition {
        id,
        summary,
        planner_summary,
        default_params,
        steps: Vec::new(),
        done_when,
        react_only,
        tool_policy,
        mission_append: String::new(),
        include_user_reference: false,
        context_manifest: None,
    })
}

fn slug_to_task_id(name: &str) -> String {
    name.chars()
        .map(|c| if c == '-' { '_' } else { c })
        .collect()
}

fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or(text).trim()
}

fn yaml_to_json(value: serde_yaml::Value) -> serde_json::Value {
    match value {
        serde_yaml::Value::Null => serde_json::Value::Null,
        serde_yaml::Value::Bool(b) => serde_json::Value::Bool(b),
        serde_yaml::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                serde_json::Value::Number(i.into())
            } else if let Some(u) = n.as_u64() {
                serde_json::Value::Number(u.into())
            } else if let Some(f) = n.as_f64() {
                serde_json::Number::from_f64(f)
                    .map(serde_json::Value::Number)
                    .unwrap_or(serde_json::Value::Null)
            } else {
                serde_json::Value::Null
            }
        }
        serde_yaml::Value::String(s) => serde_json::Value::String(s),
        serde_yaml::Value::Sequence(seq) => {
            serde_json::Value::Array(seq.into_iter().map(yaml_to_json).collect())
        }
        serde_yaml::Value::Mapping(map) => {
            let mut out = serde_json::Map::new();
            for (k, v) in map {
                if let Some(key) = k.as_str() {
                    out.insert(key.to_string(), yaml_to_json(v));
                }
            }
            serde_json::Value::Object(out)
        }
        serde_yaml::Value::Tagged(tagged) => yaml_to_json(tagged.value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::frontmatter::parse_frontmatter;
    use crate::tasks::TaskDefinition;

    #[test]
    fn builds_task_from_skill_frontmatter() {
        let text = r#"---
name: demo-skill
description: When user asks demo
harness:
  planner_summary: Pick for demo requests only
  done_when: demo complete
  tool_policy:
    allow: [run_cmd]
---
# Demo
"#;
        let doc = parse_frontmatter(text).unwrap();
        let task = task_from_skill_md(std::path::Path::new("SKILL.md"), &doc).unwrap();
        assert_eq!(task.id, "demo_skill");
        assert_eq!(task.planner_summary, "Pick for demo requests only");
        assert!(task.resolved_tool_policy().is_allowed("run_cmd"));
    }
}
