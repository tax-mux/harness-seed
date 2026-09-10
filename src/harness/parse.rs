//! Planner 作業指示書（テキスト）→ [`HarnessState`]（JSON）。

use serde_json::json;

use super::state::HarnessState;
use crate::plan::{parse_plan, PlanArtifact, PlanParseError, Subtask};

#[derive(Debug, PartialEq, Eq)]
pub enum HarnessParseError {
    Empty,
    Plan(PlanParseError),
}

impl std::fmt::Display for HarnessParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "empty planner output"),
            Self::Plan(e) => write!(f, "{e}"),
        }
    }
}

/// Planner の `Answer` 本文（作業指示書）を Harness 内部 JSON に変換する。
///
/// 1. JSON 形式の計画（`PlanArtifact` / `input`+`steps`+`output`）を優先
/// 2. 失敗時は番号付きテキスト行からサブタスクを復元
pub fn parse_harness(
    raw: &str,
    fallback_input: &str,
) -> Result<HarnessState, HarnessParseError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(HarnessParseError::Empty);
    }

    let numbered_lines = count_numbered_instruction_lines(trimmed);
    let plan = if numbered_lines >= 2 {
        parse_work_instructions_text(trimmed, fallback_input)
    } else {
        match parse_plan(trimmed) {
            Ok(plan) => plan,
            Err(_) => parse_work_instructions_text(trimmed, fallback_input),
        }
    };

    Ok(HarnessState::new(trimmed, plan))
}

/// Planner の `Answer` 本文を Harness へ変換する。
///
/// 許可:
/// - JSON 計画（`PlanArtifact` / `input`+`steps`+`output`）
/// - 番号付き作業指示（2 行以上）
/// - それ以外の平文回答は汎用の passthrough 計画として扱う
pub fn parse_harness_strict(
    raw: &str,
    fallback_input: &str,
) -> Result<HarnessState, HarnessParseError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(HarnessParseError::Empty);
    }

    let plan = match parse_plan(trimmed) {
        Ok(plan) => plan,
        Err(plan_err) => {
            if let Some(numbered) = parse_numbered_steps(trimmed) {
                numbered
            } else if looks_like_plain_text(trimmed) {
                PlanArtifact::passthrough(fallback_input)
            } else {
                return Err(HarnessParseError::Plan(plan_err));
            }
        }
    };

    Ok(HarnessState::new(trimmed, plan))
}

fn count_numbered_instruction_lines(text: &str) -> usize {
    text.lines()
        .filter(|line| parse_numbered_line(line).is_some())
        .count()
}

fn looks_like_plain_text(text: &str) -> bool {
    let trimmed = text.trim_start();
    if trimmed.is_empty() {
        return false;
    }
    !matches!(trimmed.chars().next(), Some('{') | Some('[') | Some('`'))
}

/// テキスト作業指示書から単一または複数サブタスクへ（最終フォールバック）。
fn parse_work_instructions_text(text: &str, fallback_input: &str) -> PlanArtifact {
  if let Some(plan) = parse_numbered_steps(text) {
      return plan;
  }
  PlanArtifact::single_subtask(fallback_input)
}

/// `1.` / `1)` / `ステップ1` 形式の行からサブタスク列を組み立てる。
fn parse_numbered_steps(text: &str) -> Option<PlanArtifact> {
    let mut subtasks = Vec::new();

    for line in text.lines() {
        let Some((id, body)) = parse_numbered_line(line) else {
            continue;
        };
        subtasks.push(Subtask {
            id,
            task: None,
            params: json!({}),
            goal: body,
            done_when: "step completed".into(),
        });
    }

    if subtasks.is_empty() {
        return None;
    }

    let summary = text
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("planned from work instructions")
        .trim()
        .chars()
        .take(120)
        .collect::<String>();

    Some(PlanArtifact {
        summary,
        skip_execution: false,
        subtasks,
    })
}

/// `1. goal` / `2) goal` / `ステップ3: goal` 形式をパースする。
fn parse_numbered_line(line: &str) -> Option<(u32, String)> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    if let Some(rest) = line.strip_prefix("ステップ") {
        return parse_numbered_line(rest);
    }
    if let Some(rest) = line.strip_prefix("Step ") {
        return parse_numbered_line(rest);
    }
    if let Some(rest) = line.strip_prefix("step ") {
        return parse_numbered_line(rest);
    }
    let (head, tail) = line.split_once(|c| matches!(c, '.' | ')' | '、' | '：' | ':'))?;
    let id: u32 = head.trim().parse().ok()?;
    if id == 0 {
        return None;
    }
    let body = tail.trim();
    if body.is_empty() {
        return None;
    }
    Some((id, body.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_falls_back_to_passthrough_for_plain_text() {
        let raw = "はい、doc には多くの解説文書があります。";
        let state = parse_harness_strict(raw, "fallback").unwrap();
        assert!(state.plan.skip_execution);
        assert!(state.plan.subtasks.is_empty());
    }

    #[test]
    fn strict_accepts_numbered_instructions() {
        let raw = "1. doc ディレクトリを確認する\n2. 結果を要約する";
        let state = parse_harness_strict(raw, "fallback").unwrap();
        assert_eq!(state.plan.subtasks.len(), 2);
    }
}
