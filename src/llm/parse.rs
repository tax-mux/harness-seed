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
    if let Some(step) = salvage_step_object(trimmed) {
        return Ok(step);
    }
    if let Some(content) = salvage_answer_step_content(trimmed) {
        return Ok(AgentStep::Answer(content));
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

fn is_answer_step_json(chunk: &str) -> bool {
    chunk.contains(r#""step":"answer""#)
        || chunk.contains(r#""step": "answer""#)
        || chunk.contains(r#""step" : "answer""#)
}

/// 複数行応答のうち、最後の `{"step":"answer",...}` オブジェクトを切り出す。
fn extract_last_answer_object<'a>(chunk: &'a str) -> Option<&'a str> {
    if !is_answer_step_json(chunk) {
        return None;
    }
    let markers = [r#""step":"answer""#, r#""step": "answer""#, r#""step" : "answer""#];
    let answer_idx = markers.iter().filter_map(|m| chunk.rfind(m)).max()?;
    let start = chunk[..answer_idx].rfind('{')?;
    let bytes = chunk.as_bytes();
    let mut i = start;
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
                    return std::str::from_utf8(&bytes[start..i]).ok();
                }
            }
            _ => i += 1,
        }
    }
    None
}

/// 先頭の `"` から JSON 文字列値を走査で取り出す（改行未エスケープでも可）。
fn extract_json_string_value(s: &str) -> Option<String> {
    let s = s.trim_start();
    if !s.starts_with('"') {
        return None;
    }
    let mut out = String::new();
    let mut escape = false;
    for c in s[1..].chars() {
        if escape {
            match c {
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                other => {
                    out.push('\\');
                    out.push(other);
                }
            }
            escape = false;
            continue;
        }
        if c == '\\' {
            escape = true;
            continue;
        }
        if c == '"' {
            return Some(out);
        }
        out.push(c);
    }
    None
}

/// 先頭の `"` から JSON 文字列値を走査で取り出す（未エスケープの引用符もある程度許容する）。
fn extract_json_string_value_lenient(s: &str) -> Option<String> {
    let s = s.trim_start();
    if !s.starts_with('"') {
        return None;
    }

    let mut out = String::new();
    let mut chars = s[1..].chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(next) = chars.next() {
                match next {
                    'n' => out.push('\n'),
                    'r' => out.push('\r'),
                    't' => out.push('\t'),
                    '"' => out.push('"'),
                    '\\' => out.push('\\'),
                    other => {
                        out.push('\\');
                        out.push(other);
                    }
                }
            } else {
                out.push('\\');
            }
            continue;
        }

        if c == '"' {
            let mut lookahead = chars.clone();
            while let Some(next) = lookahead.peek() {
                if next.is_whitespace() {
                    lookahead.next();
                } else {
                    break;
                }
            }
            match lookahead.peek().copied() {
                Some(',') | Some('}') | Some(']') | None => return Some(out),
                _ => out.push('"'),
            }
            continue;
        }

        out.push(c);
    }

    None
}

fn extract_json_string_value_sloppy(s: &str) -> Option<String> {
    let s = s.trim_start();
    if !s.starts_with('"') {
        return None;
    }
    let body = &s[1..];
    let end = body.rfind('"')?;
    let value = &body[..end];
    Some(value.replace("\\\"", "\"").replace("\\n", "\n").replace("\\r", "\r").replace("\\t", "\t"))
}

fn extract_step_kind(chunk: &str) -> Option<&'static str> {
    if chunk.contains(r#""step":"thought""#) || chunk.contains(r#""step": "thought""#) {
        Some("thought")
    } else if chunk.contains(r#""step":"answer""#) || chunk.contains(r#""step": "answer""#) {
        Some("answer")
    } else if chunk.contains(r#""step":"action""#) || chunk.contains(r#""step": "action""#) {
        Some("action")
    } else {
        None
    }
}

fn salvage_step_object(chunk: &str) -> Option<AgentStep> {
    let kind = extract_step_kind(chunk)?;
    let content = salvage_content_field(chunk)?;
    Some(match kind {
        "thought" => AgentStep::Thought(content),
        "answer" => AgentStep::Answer(content),
        "action" => AgentStep::Thought(format!(
            "LLM returned malformed action JSON; ignoring tools and keeping text: {content}"
        )),
        _ => return None,
    })
}

fn salvage_content_field(chunk: &str) -> Option<String> {
    const CONTENT_KEY: &str = "\"content\"";
    let mut search_from = 0usize;
    while let Some(rel) = chunk[search_from..].find(CONTENT_KEY) {
        let key_start = search_from + rel;
        let after_key = chunk[key_start + CONTENT_KEY.len()..].trim_start();
        if !after_key.starts_with(':') {
            search_from = key_start + 1;
            continue;
        }
        let after_colon = after_key[1..].trim_start();
        if let Some(value) = extract_json_string_value_sloppy(after_colon)
            .or_else(|| extract_json_string_value_lenient(after_colon))
            .or_else(|| extract_json_string_value(after_colon))
        {
            let trimmed = normalize_escaped_json_body(value.trim());
            if !trimmed.is_empty() {
                return Some(trimmed);
            }
        }
        search_from = key_start + 1;
    }
    None
}

/// `answer` ステップの `content` が改行入りで JSON パースに失敗したときの救済。
pub fn salvage_answer_step_content(chunk: &str) -> Option<String> {
    let chunk = extract_last_answer_object(chunk).unwrap_or(chunk);
    if !is_answer_step_json(chunk) {
        return None;
    }
    salvage_content_field(chunk)
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
    fn salvages_answer_with_unescaped_newlines_in_content() {
        let raw = r#"{"step":"answer","content":"参照メール（UID: 302699）の要点です。

【概要】
マネックス証券の高配当米国ETF案内です。

手続き案内: 配信解除の案内があります。"}"#;
        let step = parse_agent_step(raw, 1).unwrap();
        assert!(matches!(
            step,
            AgentStep::Answer(a) if a.contains("手続き案内") && a.contains("302699")
        ));
    }

    #[test]
    fn salvage_extracts_long_markdown_answer() {
        let raw = r#"{"step":"answer","content":"**【概要】**\n本メールは証券会社からの案内です。\n\n**注意**\n投資は自己責任です。"}"#;
        let body = salvage_answer_step_content(raw).expect("salvaged");
        assert!(body.contains("証券会社"));
        assert!(body.contains("自己責任"));
    }

    #[test]
    fn salvages_answer_with_unescaped_quotes_in_content() {
        let raw = r#"{"step":"thought","content":"The user asked for a self-introduction in Japanese ("自己紹介して"). Since I am an AI agent, I should provide a polite introduction."}"#;
        let step = parse_agent_step(raw, 1).unwrap();
        assert!(matches!(step, AgentStep::Thought(a) if a.contains("自己紹介して") && a.contains("AI agent")));
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
