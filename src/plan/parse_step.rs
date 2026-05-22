use serde::Deserialize;
use serde_json::Value;

use crate::action::AgentStep;

use super::parse::parse_plan;

#[derive(Debug, Deserialize)]
#[serde(tag = "step", rename_all = "snake_case")]
enum PlanStepJson {
    Thought { content: String },
    Action { tool: String, #[serde(default)] args: Value },
    Answer { content: String },
}

#[derive(Debug, PartialEq, Eq)]
pub enum PlanStepParseError {
    Empty,
    InvalidJson(String),
}

impl std::fmt::Display for PlanStepParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "empty plan step output"),
            Self::InvalidJson(e) => write!(f, "invalid JSON: {e}"),
        }
    }
}

/// 計画層 ReAct の 1 ステップを [`AgentStep`] に変換する。
pub fn parse_plan_agent_step(raw: &str) -> Result<AgentStep, PlanStepParseError> {
    let trimmed = strip_code_fence(raw.trim());
    if trimmed.is_empty() {
        return Err(PlanStepParseError::Empty);
    }

    let step: PlanStepJson = serde_json::from_str(trimmed)
        .map_err(|e| PlanStepParseError::InvalidJson(e.to_string()))?;

    Ok(match step {
        PlanStepJson::Thought { content } => AgentStep::Thought(content),
        PlanStepJson::Action { tool, .. } => AgentStep::Thought(format!(
            "plan layer cannot execute tools (attempted: {tool}); reply with thought or answer only"
        )),
        PlanStepJson::Answer { content } => AgentStep::Answer(content),
    })
}

/// 計画層ループ終了時の `Answer` 本文から [`super::PlanArtifact`] を得る。
pub fn plan_artifact_from_answer(
    answer: &str,
    fallback_input: &str,
) -> super::PlanArtifact {
    match parse_plan(answer) {
        Ok(plan) => plan,
        Err(err) => {
            eprintln!("[plan] answer parse error: {err}, falling back to single subtask");
            super::PlanArtifact::single_subtask(fallback_input)
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plan_thought_step() {
        let step = parse_plan_agent_step(r#"{"step":"thought","content":"分解する"}"#).unwrap();
        assert!(matches!(step, AgentStep::Thought(_)));
    }

    #[test]
    fn action_becomes_thought_warning() {
        let step =
            parse_plan_agent_step(r#"{"step":"action","tool":"list_dir","args":{}}"#).unwrap();
        assert!(matches!(step, AgentStep::Thought(t) if t.contains("cannot execute")));
    }
}
