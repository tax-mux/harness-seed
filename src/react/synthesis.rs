//! 回答合成（ステップドライバ出力 → ユーザー向け最終文）。

use super::SubtaskExecResult;

/// 回答合成に渡す evidence の 1 件あたり上限（文字数）。
pub(super) const SYNTHESIS_EVIDENCE_ITEM_MAX_CHARS: usize = 600;
/// 回答合成に渡す evidence の総量上限（文字数）。
pub(super) const SYNTHESIS_EVIDENCE_TOTAL_MAX_CHARS: usize = 4000;

pub(super) fn answer_looks_user_ready(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    if trimmed.contains('\n') || trimmed.contains('\r') {
        return false;
    }
    const STRUCTURED_MARKERS: &[char] = &['{', '[', '\t'];
    !trimmed.chars().any(|c| STRUCTURED_MARKERS.contains(&c))
}

fn truncate_evidence_chars(text: &str, max_chars: usize) -> String {
    let max_chars = max_chars.max(1);
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_string();
    }
    format!("{}…", text.chars().take(max_chars).collect::<String>())
}

fn append_evidence_budget(out: &mut String, budget: &mut usize, piece: &str) -> bool {
    if *budget == 0 {
        return false;
    }
    let count = piece.chars().count();
    if count <= *budget {
        out.push_str(piece);
        *budget -= count;
        return true;
    }
    let truncated: String = piece.chars().take(*budget).collect();
    out.push_str(&truncated);
    out.push('…');
    *budget = 0;
    false
}

pub(super) fn build_synthesis_evidence(
    results: &[SubtaskExecResult],
    observations: &[crate::action::Observation],
    item_max_chars: usize,
    total_max_chars: usize,
) -> String {
    let mut out = String::new();
    let mut budget = total_max_chars.max(1);

    for r in results {
        let body = truncate_evidence_chars(&r.answer, item_max_chars.min(budget));
        let piece = format!("### subtask {}\n{body}\n\n", r.id);
        if !append_evidence_budget(&mut out, &mut budget, &piece) {
            break;
        }
    }

    for obs in observations {
        if budget == 0 {
            break;
        }
        if !obs.ok {
            continue;
        }
        let snippet = truncate_evidence_chars(&obs.output, item_max_chars.min(budget));
        let piece = format!("- observation: {snippet}\n");
        if !append_evidence_budget(&mut out, &mut budget, &piece) {
            break;
        }
    }

    out
}

/// 複数フェーズ完了後の最終回答を、フェーズ証拠だけで再合成すべきか。
pub(super) fn needs_advance_answer_synthesis(results: &[SubtaskExecResult]) -> bool {
    results.len() >= 2
}

pub(super) fn build_advance_phase_evidence(
    results: &[SubtaskExecResult],
    item_max_chars: usize,
    total_max_chars: usize,
) -> String {
    let mut out = String::new();
    let mut budget = total_max_chars.max(1);
    for r in results {
        let body = truncate_evidence_chars(&r.answer, item_max_chars.min(budget));
        let piece = format!("### Phase / subtask {}\n{body}\n\n", r.id);
        if !append_evidence_budget(&mut out, &mut budget, &piece) {
            break;
        }
    }
    out
}

#[cfg(test)]
mod ready_tests {
    use super::*;

    #[test]
    fn answer_looks_user_ready_accepts_plain_sentence() {
        assert!(answer_looks_user_ready("実装可能です。"));
        assert!(answer_looks_user_ready("  hello world  "));
    }

    #[test]
    fn answer_looks_user_ready_rejects_structured_or_multiline() {
        assert!(!answer_looks_user_ready(""));
        assert!(!answer_looks_user_ready("line1\nline2"));
        assert!(!answer_looks_user_ready(r#"{"step":"answer"}"#));
        assert!(!answer_looks_user_ready("a\tb"));
        assert!(!answer_looks_user_ready("[a, b]"));
    }

    #[test]
    fn needs_advance_synthesis_for_multi_phase() {
        assert!(!needs_advance_answer_synthesis(&[]));
        assert!(!needs_advance_answer_synthesis(&[SubtaskExecResult {
            id: 1,
            answer: "only".into(),
            steps_used: 1,
            used_step_driver: false,
        }]));
        assert!(needs_advance_answer_synthesis(&[
            SubtaskExecResult {
                id: 1,
                answer: "a".into(),
                steps_used: 1,
                used_step_driver: false,
            },
            SubtaskExecResult {
                id: 2,
                answer: "b".into(),
                steps_used: 1,
                used_step_driver: false,
            },
        ]));
    }

    #[test]
    fn build_advance_phase_evidence_includes_ids() {
        let results = vec![
            SubtaskExecResult {
                id: 1,
                answer: "found bug in resolve_plan".into(),
                steps_used: 2,
                used_step_driver: false,
            },
            SubtaskExecResult {
                id: 2,
                answer: "generic unwrap advice".into(),
                steps_used: 1,
                used_step_driver: false,
            },
        ];
        let text = build_advance_phase_evidence(&results, 600, 4000);
        assert!(text.contains("subtask 1"));
        assert!(text.contains("resolve_plan"));
        assert!(text.contains("subtask 2"));
    }
}
