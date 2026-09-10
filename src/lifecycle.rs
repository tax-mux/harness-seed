//! ターン／計画／サブタスクのライフサイクル hook（副作用専用）。
//!
//! ホストはここに外部連携（チケット起票・進捗記録など）を載せる。
//! **本筋の ReAct ループを駆動・変更してはならない**（`run_turn` の再入、
//! キューや trace の直接書き換え、Answer の差し替えは禁止）。
//!
//! [`HostScratch`] はターン専用の標識置き場で、**プロンプト／LLM コンテキストには載せない**。
//! エラーは hook 内で処理し、呼び出し元へ伝播させないこと。
//!
//! 詳細: `doc/lifecycle.md`。

use std::collections::HashMap;

use serde_json::Value;

use crate::plan::{PlanArtifact, Subtask};

/// ターン専用のホスト状態（LLM コンテキストに出さない）。
///
/// Redmine の project / ticket / 子 ticket id など、コールバック間で共有する標識を置く。
/// `run_turn` 開始時にクリアされ、任意の seed がマージされたあと `on_turn_started` が呼ばれる。
#[derive(Debug, Clone, Default)]
pub struct HostScratch {
    values: HashMap<String, Value>,
}

impl HostScratch {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.values.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn insert(&mut self, key: impl Into<String>, value: impl Into<Value>) {
        self.values.insert(key.into(), value.into());
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.values.get(key)
    }

    pub fn get_i64(&self, key: &str) -> Option<i64> {
        self.get(key).and_then(|v| {
            v.as_i64()
                .or_else(|| v.as_u64().and_then(|u| i64::try_from(u).ok()))
                .or_else(|| v.as_str()?.parse().ok())
        })
    }

    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Value::as_str)
    }

    pub fn remove(&mut self, key: &str) -> Option<Value> {
        self.values.remove(key)
    }

    pub fn contains(&self, key: &str) -> bool {
        self.values.contains_key(key)
    }

    /// 他袋のエントリを上書きマージする。
    pub fn merge(&mut self, other: HostScratch) {
        self.values.extend(other.values);
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &Value)> {
        self.values.iter()
    }
}

/// ホストが登録するライフサイクル観測点。
///
/// すべてのメソッドに既定の空実装がある。必要な点だけオーバーライドする。
/// `host` はターン袋。次の hook から読めるよう書き込んでよい（本筋コンテキストには出ない）。
pub trait TurnLifecycle: Send + Sync {
    /// `run_turn` 入口（記憶注入の前）。seed 適用済み。指示文から ID を袋へ書いてよい。
    fn on_turn_started(&self, _user_input: &str, _host: &mut HostScratch) {}

    /// 計画が確定し、`resolve_plan` 適用後（実行開始前。skip も含む）。
    fn on_plan_finished(
        &self,
        _user_input: &str,
        _plan: &PlanArtifact,
        _host: &mut HostScratch,
    ) {
    }

    /// サブタスク実行直前。`index` は 0 始まりの実行順。
    fn on_subtask_started(
        &self,
        _user_input: &str,
        _plan: &PlanArtifact,
        _subtask: &Subtask,
        _index: usize,
        _host: &mut HostScratch,
    ) {
    }

    /// サブタスク実行直後（そのサブタスクが完了したとき。ターン全体のエラー時は呼ばれない）。
    fn on_subtask_finished(
        &self,
        _user_input: &str,
        _plan: &PlanArtifact,
        _subtask: &Subtask,
        _answer: &str,
        _steps_used: usize,
        _host: &mut HostScratch,
    ) {
    }

    /// ターン完了（session / diary 記録と同じタイミング）。
    fn on_turn_finished(
        &self,
        _user_input: &str,
        _answer: &str,
        _plan: Option<&PlanArtifact>,
        _steps_used: usize,
        _host: &mut HostScratch,
    ) {
    }
}

/// 何もしない既定実装。
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopLifecycle;

impl TurnLifecycle for NoopLifecycle {}

/// 複数 hook を順に呼ぶ（いずれも本筋には影響しない）。同一 `host` を共有する。
pub struct CompositeLifecycle {
    hooks: Vec<std::sync::Arc<dyn TurnLifecycle>>,
}

impl CompositeLifecycle {
    pub fn new(hooks: Vec<std::sync::Arc<dyn TurnLifecycle>>) -> Self {
        Self { hooks }
    }

    pub fn push(&mut self, hook: std::sync::Arc<dyn TurnLifecycle>) {
        self.hooks.push(hook);
    }
}

impl TurnLifecycle for CompositeLifecycle {
    fn on_turn_started(&self, user_input: &str, host: &mut HostScratch) {
        for h in &self.hooks {
            h.on_turn_started(user_input, host);
        }
    }

    fn on_plan_finished(&self, user_input: &str, plan: &PlanArtifact, host: &mut HostScratch) {
        for h in &self.hooks {
            h.on_plan_finished(user_input, plan, host);
        }
    }

    fn on_subtask_started(
        &self,
        user_input: &str,
        plan: &PlanArtifact,
        subtask: &Subtask,
        index: usize,
        host: &mut HostScratch,
    ) {
        for h in &self.hooks {
            h.on_subtask_started(user_input, plan, subtask, index, host);
        }
    }

    fn on_subtask_finished(
        &self,
        user_input: &str,
        plan: &PlanArtifact,
        subtask: &Subtask,
        answer: &str,
        steps_used: usize,
        host: &mut HostScratch,
    ) {
        for h in &self.hooks {
            h.on_subtask_finished(user_input, plan, subtask, answer, steps_used, host);
        }
    }

    fn on_turn_finished(
        &self,
        user_input: &str,
        answer: &str,
        plan: Option<&PlanArtifact>,
        steps_used: usize,
        host: &mut HostScratch,
    ) {
        for h in &self.hooks {
            h.on_turn_finished(user_input, answer, plan, steps_used, host);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct Recorder {
        events: Mutex<Vec<String>>,
    }

    impl TurnLifecycle for Recorder {
        fn on_turn_started(&self, user_input: &str, host: &mut HostScratch) {
            if let Some(id) = host.get_i64("ticket_id") {
                self.events
                    .lock()
                    .unwrap()
                    .push(format!("turn_started:{user_input}:ticket={id}"));
            } else {
                self.events
                    .lock()
                    .unwrap()
                    .push(format!("turn_started:{user_input}"));
            }
        }

        fn on_plan_finished(
            &self,
            _user_input: &str,
            plan: &PlanArtifact,
            host: &mut HostScratch,
        ) {
            host.insert("parent_ticket", 99);
            self.events
                .lock()
                .unwrap()
                .push(format!("plan_finished:{}", plan.summary));
        }

        fn on_subtask_started(
            &self,
            _user_input: &str,
            _plan: &PlanArtifact,
            subtask: &Subtask,
            index: usize,
            host: &mut HostScratch,
        ) {
            let parent = host.get_i64("parent_ticket").unwrap_or(-1);
            host.insert(format!("child:{index}"), subtask.id as i64);
            self.events.lock().unwrap().push(format!(
                "subtask_started:{index}:{}:parent={parent}",
                subtask.id
            ));
        }

        fn on_subtask_finished(
            &self,
            _user_input: &str,
            _plan: &PlanArtifact,
            subtask: &Subtask,
            answer: &str,
            _steps_used: usize,
            _host: &mut HostScratch,
        ) {
            self.events
                .lock()
                .unwrap()
                .push(format!("subtask_finished:{}:{answer}", subtask.id));
        }

        fn on_turn_finished(
            &self,
            _user_input: &str,
            answer: &str,
            _plan: Option<&PlanArtifact>,
            _steps_used: usize,
            host: &mut HostScratch,
        ) {
            let parent = host.get_i64("parent_ticket").unwrap_or(-1);
            self.events
                .lock()
                .unwrap()
                .push(format!("turn_finished:{answer}:parent={parent}"));
        }
    }

    #[test]
    fn scratch_get_i64_accepts_number_and_string() {
        let mut s = HostScratch::new();
        s.insert("a", 10);
        s.insert("b", "20");
        assert_eq!(s.get_i64("a"), Some(10));
        assert_eq!(s.get_i64("b"), Some(20));
    }

    #[test]
    fn scratch_merge_overwrites() {
        let mut a = HostScratch::new();
        a.insert("x", 1);
        let mut b = HostScratch::new();
        b.insert("x", 2);
        b.insert("y", json!([1, 2]));
        a.merge(b);
        assert_eq!(a.get_i64("x"), Some(2));
        assert_eq!(a.get("y").and_then(|v| v.as_array()).map(|a| a.len()), Some(2));
    }

    #[test]
    fn composite_shares_host_scratch() {
        let a = Arc::new(Recorder::default());
        let composite = CompositeLifecycle::new(vec![a.clone()]);
        let mut host = HostScratch::new();
        host.insert("ticket_id", 10);
        let plan = PlanArtifact {
            summary: "s".into(),
            skip_execution: false,
            subtasks: vec![Subtask {
                id: 1,
                task: None,
                params: json!({}),
                goal: "g".into(),
                done_when: "d".into(),
            }],
            knowledge_sufficient: None,
        };
        composite.on_turn_started("hi", &mut host);
        composite.on_plan_finished("hi", &plan, &mut host);
        composite.on_subtask_started("hi", &plan, &plan.subtasks[0], 0, &mut host);
        assert_eq!(host.get_i64("parent_ticket"), Some(99));
        assert_eq!(host.get_i64("child:0"), Some(1));
        assert_eq!(
            a.events.lock().unwrap().as_slice(),
            [
                "turn_started:hi:ticket=10",
                "plan_finished:s",
                "subtask_started:0:1:parent=99",
            ]
        );
    }

    #[test]
    fn noop_does_not_panic() {
        let n = NoopLifecycle;
        let mut host = HostScratch::new();
        let plan = PlanArtifact {
            summary: "s".into(),
            skip_execution: true,
            subtasks: vec![],
            knowledge_sufficient: Some(true),
        };
        n.on_turn_started("x", &mut host);
        n.on_plan_finished("x", &plan, &mut host);
        n.on_turn_finished("x", "a", Some(&plan), 0, &mut host);
    }
}
