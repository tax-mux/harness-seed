use serde::Deserialize;
use serde_json::Value;

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

/// LLM の生テキストから [`PlanArtifact`] を復元する。
pub fn parse_plan(raw: &str) -> Result<PlanArtifact, PlanParseError> {
    let trimmed = strip_code_fence(raw.trim());
    if trimmed.is_empty() {
        return Err(PlanParseError::Empty);
    }

    let plan: PlanJson = serde_json::from_str(trimmed)
        .map_err(|e| PlanParseError::InvalidJson(e.to_string()))?;

    let subtasks: Vec<Subtask> = plan
        .subtasks
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

    let summary = if plan.summary.is_empty() {
        "planned task".into()
    } else {
        plan.summary
    };

    if !plan.skip_execution && subtasks.is_empty() {
        return Err(PlanParseError::NoSubtasks);
    }

    Ok(PlanArtifact {
        summary,
        skip_execution: plan.skip_execution,
        subtasks,
    })
}

fn strip_code_fence(s: &str) -> &str {
    let s = s
        .strip_prefix("```json")
        .or_else(|| s.strip_prefix("```"))
        .unwrap_or(s);
    let s = s.strip_suffix("```").unwrap_or(s);
    s.trim()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plan_with_subtasks() {
        let raw = r#"{
            "summary": "list and summarize",
            "skip_execution": false,
            "subtasks": [
                {"id": 1, "goal": "list dir", "done_when": "have listing"},
                {"id": 2, "goal": "summarize", "done_when": "answer ready"}
            ]
        }"#;
        let plan = parse_plan(raw).unwrap();
        assert_eq!(plan.subtasks.len(), 2);
        assert!(!plan.skip_execution);
    }

    #[test]
    fn parses_skip_execution() {
        let raw = r#"{"summary":"hi","skip_execution":true,"subtasks":[]}"#;
        let plan = parse_plan(raw).unwrap();
        assert!(plan.skip_execution);
        assert!(plan.subtasks.is_empty());
    }

    #[test]
    fn parses_subtask_with_task_id() {
        let raw = r#"{
            "summary": "list",
            "skip_execution": false,
            "subtasks": [{"id": 1, "task": "list_dir", "params": {"path": "."}}]
        }"#;
        let plan = parse_plan(raw).unwrap();
        assert_eq!(plan.subtasks[0].task.as_deref(), Some("list_dir"));
    }
}
