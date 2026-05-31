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

fn strip_code_fence(s: &str) -> &str {
    let s = s
        .strip_prefix("```json")
        .or_else(|| s.strip_prefix("```"))
        .unwrap_or(s);
    let s = s.strip_suffix("```").unwrap_or(s);
    s.trim()
}
