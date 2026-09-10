//! 計画成果の形だけを見る推進ループ昇格判定。

use crate::plan::{execution_waves, is_replan_subtask, PlanArtifact};

/// この件数以上のサブタスクなら、計画後に推進ループへ上げる。
pub const ESCALATE_MIN_SUBTASKS: usize = 3;

/// `PlanArtifact` の形だけを見て、実行を推進ループへ上げるか。
///
/// 依頼文は見ない。実行中の薄い証拠ゲートは昇格後の advance 内に残す。
pub fn should_escalate_from_plan(plan: &PlanArtifact) -> bool {
    if !plan.needs_execution() {
        return false;
    }
    if plan.subtasks.iter().any(is_replan_subtask) {
        return true;
    }
    if plan.subtasks.len() >= ESCALATE_MIN_SUBTASKS {
        return true;
    }
    match execution_waves(&plan.subtasks) {
        Ok(waves) => waves.len() >= 2,
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::Subtask;
    use serde_json::json;

    fn st(id: u32, task: Option<&str>, deps: &[u32]) -> Subtask {
        Subtask {
            id,
            task: task.map(str::to_string),
            params: json!({}),
            goal: format!("g{id}"),
            done_when: "done".into(),
            depends_on: deps.to_vec(),
        }
    }

    fn plan(skip: bool, subtasks: Vec<Subtask>) -> PlanArtifact {
        PlanArtifact {
            summary: "s".into(),
            skip_execution: skip,
            subtasks,
            knowledge_sufficient: if skip { Some(true) } else { Some(false) },
            user_reply: None,
        }
    }

    #[test]
    fn skip_execution_does_not_escalate() {
        assert!(!should_escalate_from_plan(&plan(true, vec![])));
        assert!(!should_escalate_from_plan(&plan(
            true,
            vec![st(1, None, &[]), st(2, None, &[1]), st(3, None, &[2])]
        )));
    }

    #[test]
    fn single_freeform_does_not_escalate() {
        assert!(!should_escalate_from_plan(&plan(false, vec![st(1, None, &[])])));
    }

    #[test]
    fn two_independent_do_not_escalate() {
        assert!(!should_escalate_from_plan(&plan(
            false,
            vec![st(1, None, &[]), st(2, None, &[])]
        )));
    }

    #[test]
    fn replan_escalates() {
        assert!(should_escalate_from_plan(&plan(
            false,
            vec![st(1, None, &[]), st(2, Some("replan"), &[])]
        )));
    }

    #[test]
    fn two_waves_escalate() {
        assert!(should_escalate_from_plan(&plan(
            false,
            vec![st(1, None, &[]), st(2, None, &[1])]
        )));
    }

    #[test]
    fn three_subtasks_escalate() {
        assert!(should_escalate_from_plan(&plan(
            false,
            vec![st(1, None, &[]), st(2, None, &[]), st(3, None, &[])]
        )));
    }
}
