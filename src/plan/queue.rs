//! 可変 subtask キュー（再計画による差し込み用）。

use std::collections::VecDeque;

use super::Subtask;

/// 実行待ち subtask の可変キュー。`total_budget` で replan 連鎖を制限する。
#[derive(Debug, Clone)]
pub struct PlanQueue {
    pending: VecDeque<Subtask>,
    consumed_count: usize,
    total_budget: usize,
    /// `(subtask_id, 生成元 subtask_id)`。初期計画は parent = None。
    lineage: Vec<(u32, Option<u32>)>,
    next_id: u32,
}

impl PlanQueue {
    pub fn from_plan(subtasks: &[Subtask], total_budget: usize) -> Self {
        let total_budget = total_budget.max(1);
        let pending: VecDeque<Subtask> = subtasks.iter().cloned().collect();
        let lineage = subtasks.iter().map(|s| (s.id, None)).collect();
        let next_id = subtasks.iter().map(|s| s.id).max().unwrap_or(0).saturating_add(1);
        Self {
            pending,
            consumed_count: 0,
            total_budget,
            lineage,
            next_id,
        }
    }

    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    pub fn consumed_count(&self) -> usize {
        self.consumed_count
    }

    pub fn total_budget(&self) -> usize {
        self.total_budget
    }

    pub fn lineage(&self) -> &[(u32, Option<u32>)] {
        &self.lineage
    }

    pub fn pop_next(&mut self) -> Option<Subtask> {
        let s = self.pending.pop_front()?;
        self.consumed_count += 1;
        Some(s)
    }

    /// replan 結果を先頭に差し込む。予算超過時は Err。
    pub fn splice_from_replan(
        &mut self,
        new_subtasks: Vec<Subtask>,
        parent_id: u32,
    ) -> Result<usize, PlanQueueError> {
        if new_subtasks.is_empty() {
            return Ok(0);
        }
        let total_after =
            self.consumed_count + self.pending.len() + new_subtasks.len();
        if total_after > self.total_budget {
            return Err(PlanQueueError::BudgetExceeded {
                budget: self.total_budget,
                would_be: total_after,
            });
        }
        let mut renumbered = Vec::with_capacity(new_subtasks.len());
        for mut s in new_subtasks {
            s.id = self.next_id;
            self.next_id = self.next_id.saturating_add(1);
            self.lineage.push((s.id, Some(parent_id)));
            renumbered.push(s);
        }
        let n = renumbered.len();
        for s in renumbered.into_iter().rev() {
            self.pending.push_front(s);
        }
        Ok(n)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanQueueError {
    BudgetExceeded { budget: usize, would_be: usize },
}

impl std::fmt::Display for PlanQueueError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BudgetExceeded { budget, would_be } => {
                write!(
                    f,
                    "plan queue budget exceeded (budget={budget}, would_be={would_be})"
                )
            }
        }
    }
}

impl std::error::Error for PlanQueueError {}

/// 予約タスク id: 実行層ではなく計画層を再起動する。
pub const REPLAN_TASK_ID: &str = "replan";

pub fn is_replan_subtask(subtask: &Subtask) -> bool {
    subtask
        .task
        .as_deref()
        .is_some_and(|t| t.eq_ignore_ascii_case(REPLAN_TASK_ID))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::PlanArtifact;

    #[test]
    fn pops_in_order() {
        let plan = PlanArtifact {
            summary: "s".into(),
            skip_execution: false,
            subtasks: vec![
                Subtask {
                    id: 1,
                    task: None,
                    params: serde_json::json!({}),
                    goal: "a".into(),
                    done_when: "d".into(),
                                    depends_on: vec![],
},
                Subtask {
                    id: 2,
                    task: None,
                    params: serde_json::json!({}),
                    goal: "b".into(),
                    done_when: "d".into(),
                                    depends_on: vec![],
},
            ],
            knowledge_sufficient: None,
        };
        let mut q = PlanQueue::from_plan(&plan.subtasks, 8);
        assert_eq!(q.pop_next().unwrap().goal, "a");
        assert_eq!(q.pop_next().unwrap().goal, "b");
        assert!(q.pop_next().is_none());
        assert_eq!(q.consumed_count(), 2);
    }

    #[test]
    fn splice_inserts_at_front_with_new_ids() {
        let plan = PlanArtifact::single_subtask("first");
        let mut q = PlanQueue::from_plan(&plan.subtasks, 8);
        let parent = q.pop_next().unwrap();
        let added = q
            .splice_from_replan(
                vec![
                    Subtask {
                        id: 99,
                        task: None,
                        params: serde_json::json!({}),
                        goal: "new-a".into(),
                        done_when: "d".into(),
                                            depends_on: vec![],
},
                    Subtask {
                        id: 100,
                        task: None,
                        params: serde_json::json!({}),
                        goal: "new-b".into(),
                        done_when: "d".into(),
                                            depends_on: vec![],
},
                ],
                parent.id,
            )
            .unwrap();
        assert_eq!(added, 2);
        assert_eq!(q.pop_next().unwrap().goal, "new-a");
        assert_eq!(q.pop_next().unwrap().goal, "new-b");
        assert!(q.lineage().iter().any(|(_, p)| *p == Some(parent.id)));
    }

    #[test]
    fn splice_respects_budget() {
        let plan = PlanArtifact::single_subtask("only");
        let mut q = PlanQueue::from_plan(&plan.subtasks, 2);
        let parent = q.pop_next().unwrap();
        let err = q
            .splice_from_replan(
                vec![
                    Subtask {
                        id: 1,
                        task: None,
                        params: serde_json::json!({}),
                        goal: "x".into(),
                        done_when: "d".into(),
                                            depends_on: vec![],
},
                    Subtask {
                        id: 2,
                        task: None,
                        params: serde_json::json!({}),
                        goal: "y".into(),
                        done_when: "d".into(),
                                            depends_on: vec![],
},
                ],
                parent.id,
            )
            .unwrap_err();
        assert!(matches!(err, PlanQueueError::BudgetExceeded { .. }));
    }

    #[test]
    fn detects_replan_task() {
        let s = Subtask {
            id: 1,
            task: Some("replan".into()),
            params: serde_json::json!({}),
            goal: "rethink".into(),
            done_when: "new plan".into(),
                    depends_on: vec![],
};
        assert!(is_replan_subtask(&s));
    }
}
