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

    for chunk in extract_json_objects(trimmed) {
        match parse_one_json(&chunk, invoke_id) {
            Ok(AgentStep::Answer(a)) => answer = Some(a),
            Ok(AgentStep::Action(a)) => action = Some(a),
            Ok(AgentStep::Thought(t)) => {
                if thought.is_none() {
                    thought = Some(t);
                }
            }
            Err(e) => {
                if answer.is_none() {
                    if let Some(a) = salvage_answer_step_content(&chunk) {
                        answer = Some(a);
                    } else {
                        last_err = Some(e);
                    }
                } else {
                    last_err = Some(e);
                }
            }
        }
    }

    if answer.is_none() && action.is_none() && thought.is_none() {
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

fn normalize_escaped_json_body(body: &str) -> String {
    if body.contains("\\\"") {
        body.replace("\\\"", "\"")
    } else {
        body.to_string()
    }
}

/// `answer` ステップの `content` が改行入りで JSON パースに失敗したときの救済。
pub fn salvage_answer_step_content(chunk: &str) -> Option<String> {
    if !chunk.contains("\"step\":\"answer\"") && !chunk.contains("\"step\": \"answer\"") {
        return None;
    }
    let markers = ["\"content\":\"", "\"content\": \""];
    let start = markers
        .iter()
        .find_map(|m| chunk.find(m).map(|i| i + m.len()))?;
    let rest = chunk[start..].trim_start();
    if rest.starts_with('"') {
        let end = rest[1..].find('"')? + 1;
        return Some(normalize_escaped_json_body(rest[1..end].trim()));
    }
    if !rest.starts_with('{') {
        return None;
    }
    let end = rest.rfind("\"}")?;
    let body = rest[..end].trim();
    if body.is_empty() {
        return None;
    }
    Some(normalize_escaped_json_body(body))
}

/// テキスト中のトップレベル JSON オブジェクトを出現順に抽出する（複数行・複数オブジェクト対応）。
pub fn extract_json_objects(text: &str) -> Vec<String> {
    let mut objects = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'{' {
            i += 1;
            continue;
        }
        let start = i;
        let mut depth = 0i32;
        let mut in_string = false;
        let mut escape = false;
        while i < bytes.len() {
            let c = bytes[i];
            if in_string {
                if escape {
                    escape = false;
                } else if c == b'\\' {
                    escape = true;
                } else if c == b'"' {
                    in_string = false;
                }
                i += 1;
                continue;
            }
            match c {
                b'"' => {
                    in_string = true;
                    i += 1;
                }
                b'{' => {
                    depth += 1;
                    i += 1;
                }
                b'}' => {
                    depth -= 1;
                    i += 1;
                    if depth == 0 {
                        if let Ok(s) = std::str::from_utf8(&bytes[start..i]) {
                            objects.push(s.to_string());
                        }
                        break;
                    }
                }
                _ => i += 1,
            }
        }
        if depth != 0 {
            break;
        }
    }
    objects
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

    #[test]
    fn picks_answer_from_multiline_objects_with_embedded_newlines() {
        let raw = r#"{"step":"thought","content":"planning"}
{"step":"answer","content":"{
  \"summary\": \"do work\",
  \"skip_execution\": false,
  \"subtasks\": [{\"id\": 1, \"goal\": \"g\", \"done_when\": \"d\"}]
}"}"#;
        let step = parse_agent_step(raw, 1).unwrap();
        assert!(matches!(step, AgentStep::Answer(a) if a.contains("summary")));
    }
}
