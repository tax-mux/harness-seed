//! サブタスク実行波（依存関係に基づく）。同一波内は並列実行可能。

use super::Subtask;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScheduleError {
    /// `depends_on` が存在しない id を指している。
    UnknownDependency { subtask_id: u32, missing: u32 },
    /// 依存が閉路になっている。
    Cycle,
}

impl std::fmt::Display for ScheduleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownDependency {
                subtask_id,
                missing,
            } => write!(
                f,
                "subtask {subtask_id} depends on unknown id {missing}"
            ),
            Self::Cycle => write!(f, "subtask dependency cycle"),
        }
    }
}

impl std::error::Error for ScheduleError {}

/// `depends_on` に従い、同時実行可能なサブタスクの波に分割する。
///
/// - `depends_on` が空 → 第 0 波（他に依存しないタスクと同時に実行可）
/// - ある波のタスクは、依存先がすべて**より前の波**で完了している
/// - 同一波内のキーは subtask id（配列ではなく波の `Vec` は実行単位）
pub fn execution_waves(subtasks: &[Subtask]) -> Result<Vec<Vec<Subtask>>, ScheduleError> {
    if subtasks.is_empty() {
        return Ok(vec![]);
    }

    let ids: std::collections::HashSet<u32> = subtasks.iter().map(|s| s.id).collect();
    for st in subtasks {
        for dep in &st.depends_on {
            if !ids.contains(dep) {
                return Err(ScheduleError::UnknownDependency {
                    subtask_id: st.id,
                    missing: *dep,
                });
            }
        }
    }

    let mut remaining: Vec<Subtask> = subtasks.to_vec();
    let mut done: std::collections::HashSet<u32> = std::collections::HashSet::new();
    let mut waves = Vec::new();

    while !remaining.is_empty() {
        let (ready, rest): (Vec<_>, Vec<_>) = remaining.into_iter().partition(|st| {
            st.depends_on.iter().all(|d| done.contains(d))
        });
        if ready.is_empty() {
            return Err(ScheduleError::Cycle);
        }
        for st in &ready {
            done.insert(st.id);
        }
        let mut ready = ready;
        ready.sort_by_key(|s| s.id);
        waves.push(ready);
        remaining = rest;
    }

    Ok(waves)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn st(id: u32, deps: &[u32]) -> Subtask {
        Subtask {
            id,
            task: None,
            params: json!({}),
            goal: format!("g{id}"),
            done_when: "d".into(),
            depends_on: deps.to_vec(),
        }
    }

    #[test]
    fn no_deps_single_wave() {
        let waves = execution_waves(&[st(1, &[]), st(2, &[])]).unwrap();
        assert_eq!(waves.len(), 1);
        assert_eq!(
            waves[0].iter().map(|s| s.id).collect::<Vec<_>>(),
            vec![1, 2]
        );
    }

    #[test]
    fn chain_makes_three_waves() {
        let waves = execution_waves(&[st(1, &[]), st(2, &[1]), st(3, &[2])]).unwrap();
        assert_eq!(waves.len(), 3);
        assert_eq!(waves[0][0].id, 1);
        assert_eq!(waves[1][0].id, 2);
        assert_eq!(waves[2][0].id, 3);
    }

    #[test]
    fn diamond_parallel_middle() {
        let waves =
            execution_waves(&[st(1, &[]), st(2, &[1]), st(3, &[1]), st(4, &[2, 3])]).unwrap();
        assert_eq!(waves.len(), 3);
        assert_eq!(waves[0][0].id, 1);
        assert_eq!(
            waves[1].iter().map(|s| s.id).collect::<Vec<_>>(),
            vec![2, 3]
        );
        assert_eq!(waves[2][0].id, 4);
    }

    #[test]
    fn unknown_dep_errors() {
        let err = execution_waves(&[st(1, &[9])]).unwrap_err();
        assert!(matches!(
            err,
            ScheduleError::UnknownDependency {
                subtask_id: 1,
                missing: 9
            }
        ));
    }

    #[test]
    fn cycle_errors() {
        let err = execution_waves(&[st(1, &[2]), st(2, &[1])]).unwrap_err();
        assert_eq!(err, ScheduleError::Cycle);
    }
}
