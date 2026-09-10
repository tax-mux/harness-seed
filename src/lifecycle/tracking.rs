//! タスクマネージメント向けホスト API。
//!
//! [`TurnLifecycle`] のタイミングはエンジンが決める。ホストが PM／チケット連携を載せるときは
//! 生 hook ではなく本モジュールの [`TaskTracking`] を実装し、
//! [`lifecycle_from_tracking`] で接続するのが推奨（開始／完了の業務語彙に寄せる）。

use std::sync::Arc;

use crate::plan::{PlanArtifact, Subtask};

use super::{HostView, SubtaskOutcome, TurnLifecycle, TurnOutcome};

/// サブタスク開始イベント（[`TaskTracking::on_work_started`]）。
#[derive(Debug)]
pub struct WorkStartedEvent<'a> {
    pub user_input: &'a str,
    pub plan: &'a PlanArtifact,
    pub subtask: &'a Subtask,
    /// 0 始まりの実行順。
    pub index: usize,
}

/// サブタスク終了イベント（[`TaskTracking::on_work_finished`]）。
#[derive(Debug)]
pub struct WorkFinishedEvent<'a> {
    pub user_input: &'a str,
    pub plan: &'a PlanArtifact,
    pub subtask: &'a Subtask,
    pub outcome: &'a SubtaskOutcome,
}

/// ターン終了イベント（[`TaskTracking::on_turn_finished`]）。
#[derive(Debug)]
pub struct TurnFinishedEvent<'a> {
    pub user_input: &'a str,
    pub plan: Option<&'a PlanArtifact>,
    pub outcome: &'a TurnOutcome,
}

/// 外部タスク追跡（起票・開始・完了）のホスト向け API。
///
/// エンジンはチケット製品を知らない。ホストがこの面に Redmine / Issue tracker 等を載せる。
/// すべてのメソッドに既定の空実装がある。
pub trait TaskTracking: Send + Sync {
    fn on_turn_started(&self, _user_input: &str, _host: HostView<'_>) {}

    /// 計画確定後（実行前。skip も含む）。親 work item の作成に使う。
    fn on_plan_ready(&self, _user_input: &str, _plan: &PlanArtifact, _host: HostView<'_>) {}

    /// サブタスク実行直前。子 work item の作成／開始に使う。
    fn on_work_started(&self, _event: WorkStartedEvent<'_>, _host: HostView<'_>) {}

    /// サブタスク終了（成功・失敗・取消いずれも含む）。
    fn on_work_finished(&self, _event: WorkFinishedEvent<'_>, _host: HostView<'_>) {}

    /// ターン終了（成功・失敗・取消いずれも含む）。
    fn on_turn_finished(&self, _event: TurnFinishedEvent<'_>, _host: HostView<'_>) {}
}

/// [`TaskTracking`] を [`TurnLifecycle`] にブリッジする。
pub struct TaskTrackingLifecycle {
    inner: Arc<dyn TaskTracking>,
}

impl TaskTrackingLifecycle {
    pub fn new(inner: Arc<dyn TaskTracking>) -> Self {
        Self { inner }
    }
}

/// `TaskTracking` 実装を lifecycle hook として登録できる形にする。
pub fn lifecycle_from_tracking(tracking: Arc<dyn TaskTracking>) -> Arc<dyn TurnLifecycle> {
    Arc::new(TaskTrackingLifecycle::new(tracking))
}

impl TurnLifecycle for TaskTrackingLifecycle {
    fn on_turn_started(&self, user_input: &str, host: HostView<'_>) {
        self.inner.on_turn_started(user_input, host);
    }

    fn on_plan_finished(&self, user_input: &str, plan: &PlanArtifact, host: HostView<'_>) {
        self.inner.on_plan_ready(user_input, plan, host);
    }

    fn on_subtask_started(
        &self,
        user_input: &str,
        plan: &PlanArtifact,
        subtask: &Subtask,
        index: usize,
        host: HostView<'_>,
    ) {
        self.inner.on_work_started(
            WorkStartedEvent {
                user_input,
                plan,
                subtask,
                index,
            },
            host,
        );
    }

    fn on_subtask_finished(
        &self,
        user_input: &str,
        plan: &PlanArtifact,
        subtask: &Subtask,
        outcome: &SubtaskOutcome,
        host: HostView<'_>,
    ) {
        self.inner.on_work_finished(
            WorkFinishedEvent {
                user_input,
                plan,
                subtask,
                outcome,
            },
            host,
        );
    }

    fn on_turn_finished(
        &self,
        user_input: &str,
        plan: Option<&PlanArtifact>,
        outcome: &TurnOutcome,
        host: HostView<'_>,
    ) {
        self.inner.on_turn_finished(
            TurnFinishedEvent {
                user_input,
                plan,
                outcome,
            },
            host,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lifecycle::{HostScratch, RunStatus, WriteScope};
    use serde_json::json;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Rec {
        events: Mutex<Vec<String>>,
    }

    impl TaskTracking for Rec {
        fn on_plan_ready(&self, _: &str, plan: &PlanArtifact, mut host: HostView<'_>) {
            host.insert("parent", 1);
            self.events
                .lock()
                .unwrap()
                .push(format!("plan:{}", plan.summary));
        }

        fn on_work_started(&self, event: WorkStartedEvent<'_>, mut host: HostView<'_>) {
            host.insert("child", event.subtask.id as i64);
            self.events
                .lock()
                .unwrap()
                .push(format!("start:{}", event.subtask.id));
        }

        fn on_work_finished(&self, event: WorkFinishedEvent<'_>, host: HostView<'_>) {
            let child = host.get_i64("child").unwrap_or(-1);
            self.events.lock().unwrap().push(format!(
                "finish:{}:{:?}:{child}",
                event.subtask.id, event.outcome.status
            ));
        }

        fn on_turn_finished(&self, event: TurnFinishedEvent<'_>, _: HostView<'_>) {
            self.events.lock().unwrap().push(format!(
                "turn:{:?}",
                event.outcome.status
            ));
        }
    }

    #[test]
    fn tracking_bridge_maps_lifecycle_events() {
        let rec = Arc::new(Rec::default());
        let life = TaskTrackingLifecycle::new(rec.clone());
        let mut scratch = HostScratch::new();
        let plan = PlanArtifact {
            summary: "s".into(),
            skip_execution: false,
            subtasks: vec![Subtask {
                id: 1,
                task: None,
                params: json!({}),
                goal: "g".into(),
                done_when: "d".into(),
                depends_on: vec![],
            }],
            knowledge_sufficient: None,
            user_reply: None,
        };
        life.on_plan_finished("u", &plan, HostView::new(&mut scratch, WriteScope::Turn));
        life.on_subtask_started(
            "u",
            &plan,
            &plan.subtasks[0],
            0,
            HostView::new(&mut scratch, WriteScope::Subtask(1)),
        );
        let outcome = SubtaskOutcome::completed("ok", 2);
        life.on_subtask_finished(
            "u",
            &plan,
            &plan.subtasks[0],
            &outcome,
            HostView::new(&mut scratch, WriteScope::Subtask(1)),
        );
        let turn = TurnOutcome::completed("final", 2);
        life.on_turn_finished(
            "u",
            Some(&plan),
            &turn,
            HostView::new(&mut scratch, WriteScope::Turn),
        );
        assert_eq!(
            rec.events.lock().unwrap().as_slice(),
            [
                "plan:s",
                "start:1",
                "finish:1:Completed:1",
                "turn:Completed",
            ]
        );
        assert!(RunStatus::Completed.is_ok());
    }
}
