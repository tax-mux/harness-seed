use serde::Deserialize;
use serde_json::Value;

use crate::action::{Action, AgentStep};

#[derive(Debug, Deserialize)]
#[serde(tag = "step", rename_all = "snake_case")]
enum StepJson {
    Thought { content: String },
    Action { tool: String, #[serde(default)] args: Value },
    Answer { content: String },
}

#[derive(Debug, PartialEq, Eq)]
pub enum ParseError {
    Empty,
    InvalidJson(String),
    UnknownStep(String),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "empty LLM output"),
            Self::InvalidJson(e) => write!(f, "invalid JSON: {e}"),
            Self::UnknownStep(s) => write!(f, "unknown step payload: {s}"),
        }
    }
}

/// LLM の生テキストから `AgentStep` を復元する。
///
/// 1 行に 1 JSON が理想だが、複数行出力時は **answer > action > thought** の優先度で 1 件選ぶ。
pub fn parse_agent_step(raw: &str, invoke_id: u64) -> Result<AgentStep, ParseError> {
    let trimmed = strip_code_fence(raw.trim());
    if trimmed.is_empty() {
        return Err(ParseError::Empty);
    }

    if let Ok(step) = parse_one_json(trimmed, invoke_id) {
        return Ok(step);
    }

    let mut thought = None;
    let mut action = None;
    let mut answer = None;
    let mut last_err = None;

    for line in trimmed.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match parse_one_json(line, invoke_id) {
            Ok(AgentStep::Answer(a)) => answer = Some(a),
            Ok(AgentStep::Action(a)) => action = Some(a),
            Ok(AgentStep::Thought(t)) => {
                if thought.is_none() {
                    thought = Some(t);
                }
            }
            Err(e) => last_err = Some(e),
        }
    }

    if let Some(a) = answer {
        return Ok(AgentStep::Answer(a));
    }
    if let Some(a) = action {
        return Ok(AgentStep::Action(a));
    }
    if let Some(t) = thought {
        return Ok(AgentStep::Thought(t));
    }

    Err(last_err.unwrap_or(ParseError::InvalidJson(
        "no valid JSON step in response".into(),
    )))
}

fn parse_one_json(text: &str, invoke_id: u64) -> Result<AgentStep, ParseError> {
    let step: StepJson = serde_json::from_str(text)
        .map_err(|e| ParseError::InvalidJson(e.to_string()))?;

    Ok(match step {
        StepJson::Thought { content } => AgentStep::Thought(content),
        StepJson::Action { tool, args } => AgentStep::Action(Action::new(invoke_id, tool, args)),
        StepJson::Answer { content } => AgentStep::Answer(content),
    })
}

fn strip_code_fence(s: &str) -> &str {
    let s = s.strip_prefix("```json").or_else(|| s.strip_prefix("```")).unwrap_or(s);
    let s = s.strip_suffix("```").unwrap_or(s);
    s.trim()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_thought_json() {
        let step = parse_agent_step(r#"{"step":"thought","content":"考え中"}"#, 1).unwrap();
        assert!(matches!(step, AgentStep::Thought(_)));
    }

    #[test]
    fn parses_action_json() {
        let raw = r#"{"step":"action","tool":"echo","args":{"message":"hi"}}"#;
        let step = parse_agent_step(raw, 7).unwrap();
        assert!(matches!(step, AgentStep::Action(a) if a.invoke_id == 7 && a.tool == "echo"));
    }

    #[test]
    fn picks_action_from_multiline_response() {
        let raw = r#"{"step":"thought","content":"plan"}
{"step":"action","tool":"write_file","args":{"path":"a.rs","content":"x"}}"#;
        let step = parse_agent_step(raw, 1).unwrap();
        assert!(matches!(step, AgentStep::Action(a) if a.tool == "write_file"));
    }

    #[test]
    fn strips_markdown_fence() {
        let raw = "```json\n{\"step\":\"answer\",\"content\":\"ok\"}\n```";
        let step = parse_agent_step(raw, 1).unwrap();
        assert!(matches!(step, AgentStep::Answer(a) if a == "ok"));
    }
}
