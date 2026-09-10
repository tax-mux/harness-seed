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

#[derive(Debug)]
struct SubtaskJson {
    id: u32,
    task: Option<String>,
    params: Value,
    goal: String,
    done_when: String,
    depends_on: Vec<u32>,
}

#[derive(Debug, Deserialize)]
struct PlanJsonLoose {
    #[serde(default)]
    summary: String,
    #[serde(default)]
    skip_execution: bool,
    #[serde(default)]
    knowledge_sufficient: Option<bool>,
    #[serde(default)]
    subtasks: Vec<Value>,
}

#[derive(Debug, Deserialize)]
struct PlanFlowJsonLoose {
    #[serde(default)]
    input: Vec<String>,
    #[serde(default)]
    steps: Vec<Value>,
    #[serde(default)]
    output: String,
    #[serde(default)]
    skip_execution: bool,
    #[serde(default)]
    knowledge_sufficient: Option<bool>,
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

    let (summary, skip_execution, knowledge_sufficient, step_values): (
        String,
        bool,
        Option<bool>,
        Vec<Value>,
    ) = if value.get("steps").is_some()
        || value.get("input").is_some()
        || value.get("output").is_some()
    {
        let flow: PlanFlowJsonLoose = serde_json::from_value(value)
            .map_err(|e| PlanParseError::InvalidJson(e.to_string()))?;
        let summary = if !flow.output.trim().is_empty() {
            flow.output
        } else if flow.input.is_empty() {
            "planned task".into()
        } else {
            format!("from input: {}", flow.input.join(" | "))
        };
        (
            summary,
            flow.skip_execution,
            flow.knowledge_sufficient,
            flow.steps,
        )
    } else {
        let plan: PlanJsonLoose = serde_json::from_value(value)
            .map_err(|e| PlanParseError::InvalidJson(e.to_string()))?;
        let summary = if plan.summary.is_empty() {
            "planned task".into()
        } else {
            plan.summary
        };
        (
            summary,
            plan.skip_execution,
            plan.knowledge_sufficient,
            plan.subtasks,
        )
    };

    let subtasks: Vec<Subtask> = step_values
        .into_iter()
        .enumerate()
        .filter_map(|(i, v)| parse_subtask_value(&v, (i as u32).saturating_add(1)))
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
            depends_on: s.depends_on,
        })
        .collect();

    let mut plan = PlanArtifact {
        summary,
        skip_execution,
        subtasks,
        knowledge_sufficient,
    };
    plan.enforce_knowledge_sufficiency();

    if !plan.skip_execution && plan.subtasks.is_empty() {
        return Err(PlanParseError::NoSubtasks);
    }

    Ok(plan)
}

/// LLM が `id` にタスク名（`"list_dir"`）を入れる・番号を文字列にする等を吸収する。
fn parse_subtask_value(value: &Value, fallback_id: u32) -> Option<SubtaskJson> {
    let obj = value.as_object()?;
    let mut task = obj
        .get("task")
        .and_then(|t| match t {
            Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        });

    let id = match obj.get("id") {
        Some(Value::Number(n)) => n.as_u64().unwrap_or(u64::from(fallback_id)) as u32,
        Some(Value::String(s)) => {
            let s = s.trim();
            if let Ok(n) = s.parse::<u32>() {
                n
            } else if !s.is_empty() {
                // `{"id":"list_dir",...}` — タスク名が id に入った典型ミス
                if task.is_none() {
                    task = Some(s.to_string());
                }
                fallback_id
            } else {
                fallback_id
            }
        }
        _ => fallback_id,
    };

    let params = obj.get("params").cloned().unwrap_or(Value::Object(Default::default()));
    let goal = obj
        .get("goal")
        .and_then(|g| g.as_str())
        .unwrap_or("")
        .to_string();
    let done_when = obj
        .get("done_when")
        .and_then(|g| g.as_str())
        .unwrap_or("")
        .to_string();
    let depends_on = obj
        .get("depends_on")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| {
                    x.as_u64()
                        .map(|n| n as u32)
                        .or_else(|| x.as_str()?.parse().ok())
                })
                .collect()
        })
        .unwrap_or_default();

    Some(SubtaskJson {
        id,
        task,
        params,
        goal,
        done_when,
        depends_on,
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
        let raw = r#"{"summary":"say \"hello\" path\C:\Users\x","skip_execution":true,"knowledge_sufficient":true,"subtasks":[]}"#;
        let plan = parse_plan(raw).expect("repaired with escaped quotes");
        assert!(plan.skip_execution);
        assert!(plan.summary.contains('"'));
        assert!(plan.summary.contains(r"C:\Users\x") || plan.summary.contains("Users"));
    }

    #[test]
    fn repairs_incomplete_unicode_escape() {
        let raw = r#"{"summary":"bad\u12 end","skip_execution":true,"knowledge_sufficient":true,"subtasks":[]}"#;
        let plan = parse_plan(raw).expect("repaired incomplete \\u");
        assert!(plan.summary.contains("u12"));
    }

    #[test]
    fn keeps_valid_unicode_escape() {
        let raw = r#"{"summary":"\u3042","skip_execution":true,"knowledge_sufficient":true,"subtasks":[]}"#;
        let plan = parse_plan(raw).expect("valid \\u");
        assert_eq!(plan.summary, "あ");
    }

    #[test]
    fn rejects_skip_without_knowledge_sufficient() {
        let raw = r#"{
  "summary":"thin diary only",
  "skip_execution":true,
  "subtasks":[],
  "output":"guessing from memory"
}"#;
        let plan = parse_plan(raw).expect("forced execute");
        assert!(!plan.skip_execution);
        assert_eq!(plan.knowledge_sufficient, Some(false));
        assert_eq!(plan.subtasks.len(), 1);
        assert!(plan.subtasks[0].task.is_none());
        assert!(plan.subtasks[0].goal.contains("insufficient"));
    }

    #[test]
    fn allows_skip_when_knowledge_sufficient() {
        let raw = r#"{"summary":"hi","skip_execution":true,"knowledge_sufficient":true,"subtasks":[],"output":"hello"}"#;
        let plan = parse_plan(raw).expect("skip ok");
        assert!(plan.skip_execution);
        assert!(plan.subtasks.is_empty());
    }

    #[test]
    fn fills_freeform_step_when_exec_with_empty_subtasks() {
        // 知識不足は認めたが steps を出し忘れた典型
        let raw = r#"{
  "summary":"need more evidence",
  "skip_execution":false,
  "knowledge_sufficient":false,
  "subtasks":[]
}"#;
        let plan = parse_plan(raw).expect("freeform filled");
        assert!(!plan.skip_execution);
        assert_eq!(plan.subtasks.len(), 1);
        assert!(plan.subtasks[0].task.is_none());
        assert!(plan.subtasks[0].goal.contains("need more evidence"));
    }

    #[test]
    fn accepts_task_name_in_id_field() {
        // LLM が id にタスク名を入れる典型ミス
        let raw = r#"{
  "summary": "list files",
  "skip_execution": false,
  "steps": [
    {"id": "list_dir", "params": {"path": "."}, "goal": "一覧", "done_when": "done"}
  ]
}"#;
        let plan = parse_plan(raw).expect("id as task name");
        assert_eq!(plan.subtasks.len(), 1);
        assert_eq!(plan.subtasks[0].id, 1);
        assert_eq!(plan.subtasks[0].task.as_deref(), Some("list_dir"));
    }

    #[test]
    fn accepts_string_numeric_id() {
        let raw = r#"{
  "summary": "x",
  "skip_execution": false,
  "subtasks": [
    {"id": "2", "task": "list_dir", "goal": "g", "done_when": "d"}
  ]
}"#;
        let plan = parse_plan(raw).expect("string id");
        assert_eq!(plan.subtasks[0].id, 2);
        assert_eq!(plan.subtasks[0].task.as_deref(), Some("list_dir"));
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
