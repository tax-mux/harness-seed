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
///
/// 有効なエスケープは `\` と後続をまとめて消費する（`\"` の `"` を文字列終端と誤認しない）。
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
                Some(n @ ('"' | '\\' | '/' | 'b' | 'f' | 'n' | 'r' | 't')) => {
                    out.push('\\');
                    out.push(n);
                    chars.next();
                }
                Some('u') => {
                    // `\uXXXX` は 4 桁 hex が揃っているときだけ有効。足りなければ `\\` に直す。
                    let mut hex = [None; 4];
                    let mut iter = chars.clone();
                    iter.next(); // 'u'
                    let mut ok = true;
                    for slot in &mut hex {
                        match iter.next() {
                            Some(h) if h.is_ascii_hexdigit() => *slot = Some(h),
                            _ => {
                                ok = false;
                                break;
                            }
                        }
                    }
                    if ok {
                        out.push('\\');
                        out.push('u');
                        chars.next(); // 'u'
                        for _ in 0..4 {
                            out.push(chars.next().expect("hex digit"));
                        }
                    } else {
                        out.push_str("\\\\");
                    }
                }
                Some('\n') | Some('\r') | None => {
                    out.push_str("\\\\");
                }
                Some(_) => {
                    // `\C` など不正エスケープ → `\\C`
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

    #[test]
    fn repairs_escaped_quotes_without_leaving_string() {
        // `\"` のあとにも不正 `\` がある典型。旧実装は `\"` で文字列を抜けて修復に失敗した。
        let raw = r#"{"summary":"say \"hello\" path\C:\Users\x","skip_execution":true,"subtasks":[]}"#;
        let plan = parse_plan(raw).expect("repaired with escaped quotes");
        assert!(plan.skip_execution);
        assert!(plan.summary.contains('"'));
        assert!(plan.summary.contains(r"C:\Users\x") || plan.summary.contains("Users"));
    }

    #[test]
    fn repairs_incomplete_unicode_escape() {
        let raw = r#"{"summary":"bad\u12 end","skip_execution":true,"subtasks":[]}"#;
        let plan = parse_plan(raw).expect("repaired incomplete \\u");
        assert!(plan.summary.contains("u12"));
    }

    #[test]
    fn keeps_valid_unicode_escape() {
        let raw = r#"{"summary":"\u3042","skip_execution":true,"subtasks":[]}"#;
        let plan = parse_plan(raw).expect("valid \\u");
        assert_eq!(plan.summary, "あ");
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
