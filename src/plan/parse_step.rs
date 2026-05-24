use serde::Deserialize;
use serde_json::Value;

use crate::action::AgentStep;
use crate::llm::{extract_json_objects, salvage_answer_step_content};

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

fn merge_plan_step(
    parsed: Result<AgentStep, PlanStepParseError>,
    thought: &mut Option<String>,
    answer: &mut Option<String>,
    last_err: &mut Option<PlanStepParseError>,
) {
    match parsed {
        Ok(AgentStep::Answer(a)) => *answer = Some(a),
        Ok(AgentStep::Thought(t)) => {
            if thought.is_none() {
                *thought = Some(t);
            }
        }
        Ok(AgentStep::Action(_)) => {}
        Err(e) => *last_err = Some(e),
    }
}

fn merge_plan_chunk(
    chunk: &str,
    thought: &mut Option<String>,
    answer: &mut Option<String>,
    last_err: &mut Option<PlanStepParseError>,
) {
    merge_plan_step(
        parse_one_plan_json(chunk),
        thought,
        answer,
        last_err,
    );
    if answer.is_none() {
        if let Some(body) = salvage_answer_step_content(chunk) {
            *answer = Some(body);
        }
    }
}

fn parse_one_plan_json(text: &str) -> Result<AgentStep, PlanStepParseError> {
    let step: PlanStepJson = serde_json::from_str(text)
        .map_err(|e| PlanStepParseError::InvalidJson(e.to_string()))?;

    Ok(match step {
        PlanStepJson::Thought { content } => AgentStep::Thought(content),
        PlanStepJson::Action { tool, .. } => AgentStep::Thought(format!(
            "plan layer cannot execute tools (attempted: {tool}); reply with thought or answer only"
        )),
        PlanStepJson::Answer { content } => AgentStep::Answer(content),
    })
}

/// 計画層 ReAct の 1 ステップを [`AgentStep`] に変換する。
///
/// 1 行 1 JSON が理想。複数オブジェクト出力時は **answer > thought > action** の優先度で 1 件選ぶ。
pub fn parse_plan_agent_step(raw: &str) -> Result<AgentStep, PlanStepParseError> {
    let trimmed = strip_code_fence(raw.trim());
    if trimmed.is_empty() {
        return Err(PlanStepParseError::Empty);
    }

    if let Ok(step) = parse_one_plan_json(trimmed) {
        return Ok(step);
    }

    let mut thought = None;
    let mut answer = None;
    let mut last_err = None;

    for chunk in extract_json_objects(trimmed) {
        merge_plan_chunk(&chunk, &mut thought, &mut answer, &mut last_err);
    }

    if answer.is_none() {
        merge_plan_chunk(trimmed, &mut thought, &mut answer, &mut last_err);
    }

    if answer.is_none() && thought.is_none() {
        for line in trimmed.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            merge_plan_chunk(line, &mut thought, &mut answer, &mut last_err);
        }
    }

    if let Some(a) = answer {
        return Ok(AgentStep::Answer(a));
    }
    if let Some(t) = thought {
        return Ok(AgentStep::Thought(t));
    }

    Err(last_err.unwrap_or(PlanStepParseError::InvalidJson(
        "no valid JSON step in response".into(),
    )))
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

    #[test]
    fn picks_answer_over_thought_multiline() {
        let raw = r#"{"step":"thought","content":"The user wants an apology email."}
{"step":"answer","content":"{
  \"summary\": \"Check compose form then draft apology\",
  \"skip_execution\": false,
  \"subtasks\": [
    {\"id\": 1, \"task\": \"get_compose_form\", \"params\": {}, \"goal\": \"read form\", \"done_when\": \"success\"}
  ]
}"}"#;
        let step = parse_plan_agent_step(raw).unwrap();
        match &step {
            AgentStep::Answer(body) => {
                assert!(body.contains("get_compose_form"));
                let plan = parse_plan(body).expect("plan artifact in answer");
                assert!(!plan.skip_execution);
                assert_eq!(plan.subtasks.len(), 1);
            }
            other => panic!("expected Answer, got {other:?}"),
        }
    }
}
