use serde::Deserialize;
use serde_json::Value;

use crate::llm::extract_json_objects;

use super::{PlanArtifact, Subtask};

#[derive(Debug, PartialEq, Eq)]
pub enum PlanParseError {
    Empty,
    InvalidJson(String),
    NoSubtasks,
}

impl std::fmt::Display for PlanParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "empty plan output"),
            Self::InvalidJson(e) => write!(f, "invalid JSON: {e}"),
            Self::NoSubtasks => write!(f, "plan has no subtasks and skip_execution is false"),
        }
    }
}

#[derive(Debug, Deserialize)]
struct SubtaskJson {
    id: u32,
    #[serde(default)]
    task: Option<String>,
    #[serde(default)]
    params: Value,
    #[serde(default)]
    goal: String,
    #[serde(default)]
    done_when: String,
}

#[derive(Debug, Deserialize)]
struct PlanJson {
    #[serde(default)]
    summary: String,
    #[serde(default)]
    skip_execution: bool,
    #[serde(default)]
    subtasks: Vec<SubtaskJson>,
}

#[derive(Debug, Deserialize)]
struct PlanFlowJson {
    #[serde(default)]
    input: Vec<String>,
    #[serde(default)]
    steps: Vec<SubtaskJson>,
    #[serde(default)]
    output: String,
    #[serde(default)]
    skip_execution: bool,
}

/// LLM の生テキストから [`PlanArtifact`] を復元する。
pub fn parse_plan(raw: &str) -> Result<PlanArtifact, PlanParseError> {
    let trimmed = strip_code_fence(raw.trim());
    if trimmed.is_empty() {
        return Err(PlanParseError::Empty);
    }

    match parse_plan_value(trimmed) {
        Ok(plan) => Ok(plan),
        Err(first_err) => {
            let repaired = repair_json_for_llm(trimmed);
            if repaired != trimmed {
                if let Ok(plan) = parse_plan_value(&repaired) {
                    return Ok(plan);
                }
            }
            for chunk in extract_json_objects(trimmed) {
                if let Ok(plan) = parse_plan_value(&chunk) {
                    return Ok(plan);
                }
                let repaired_chunk = repair_json_for_llm(&chunk);
                if let Ok(plan) = parse_plan_value(&repaired_chunk) {
                    return Ok(plan);
                }
            }
            Err(first_err)
        }
    }
}

fn parse_plan_value(trimmed: &str) -> Result<PlanArtifact, PlanParseError> {
    let value: Value =
        serde_json::from_str(trimmed).map_err(|e| PlanParseError::InvalidJson(e.to_string()))?;

    let (summary, skip_execution, raw_subtasks): (String, bool, Vec<SubtaskJson>) =
        if value.get("steps").is_some() || value.get("input").is_some() || value.get("output").is_some() {
            let flow: PlanFlowJson = serde_json::from_value(value)
                .map_err(|e| PlanParseError::InvalidJson(e.to_string()))?;
            let summary = if !flow.output.trim().is_empty() {
                flow.output
            } else if flow.input.is_empty() {
                "planned task".into()
            } else {
                format!("from input: {}", flow.input.join(" | "))
            };
            (summary, flow.skip_execution, flow.steps)
        } else {
            let plan: PlanJson = serde_json::from_value(value)
                .map_err(|e| PlanParseError::InvalidJson(e.to_string()))?;
            let summary = if plan.summary.is_empty() {
                "planned task".into()
            } else {
                plan.summary
            };
            (summary, plan.skip_execution, plan.subtasks)
        };

    let subtasks: Vec<Subtask> = raw_subtasks
        .into_iter()
        .map(|s| Subtask {
            id: s.id,
            task: s.task,
            params: s.params,
            goal: s.goal,
            done_when: if s.done_when.is_empty() {
                "criterion met".into()
            } else {
                s.done_when
            },
        })
        .collect();

    if !skip_execution && subtasks.is_empty() {
        return Err(PlanParseError::NoSubtasks);
    }

    Ok(PlanArtifact {
        summary,
        skip_execution,
        subtasks,
    })
}

/// LLM 計画 JSON の典型破損（文字列内の生改行・不正 `\` エスケープ）を修復する。
fn repair_json_for_llm(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 64);
    let mut in_string = false;
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if !in_string {
            out.push(c);
            if c == '"' {
                in_string = true;
            }
            continue;
        }

        if c == '\\' {
            match chars.peek().copied() {
                Some('"')
                | Some('\\')
                | Some('/')
                | Some('b')
                | Some('f')
                | Some('n')
                | Some('r')
                | Some('t')
                | Some('u') => {
                    out.push(c);
                }
                Some('\n') | Some('\r') | None => {
                    out.push_str("\\\\");
                }
                Some(_) => {
                    out.push_str("\\\\");
                }
            }
            continue;
        }

        if c == '"' {
            in_string = false;
            out.push(c);
            continue;
        }

        if c == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            out.push_str("\\n");
            continue;
        }
        if c == '\n' {
            out.push_str("\\n");
            continue;
        }
        if c == '\t' {
            out.push_str("\\t");
            continue;
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod repair_tests {
    use super::*;

    #[test]
    fn repairs_unescaped_newlines_in_goal() {
        let raw = r#"{
  "summary": "返信ドラフト",
  "skip_execution": false,
  "subtasks": [
    {
      "id": 1,
      "task": "compose_context",
      "params": {},
      "goal": "参照メールを確認
返信案を作成",
      "done_when": "確認完了"
    }
  ]
}"#;
        let plan = parse_plan(raw).expect("repaired plan");
        assert_eq!(plan.subtasks.len(), 1);
        assert!(plan.subtasks[0].goal.contains('\n'));
    }

    #[test]
    fn repairs_trailing_backslash_before_newline() {
        let raw = r#"{
  "summary": "x",
  "skip_execution": false,
  "subtasks": [
    {"id": 1, "task": "compose_write", "goal": "path\C:\
next", "done_when": "done"}
  ]
}"#;
        let plan = parse_plan(raw).expect("repaired invalid escape");
        assert_eq!(plan.subtasks[0].id, 1);
    }
}

fn strip_code_fence(s: &str) -> &str {
    let s = s
        .strip_prefix("```json")
        .or_else(|| s.strip_prefix("```"))
        .unwrap_or(s);
    let s = s.strip_suffix("```").unwrap_or(s);
    s.trim()
}
