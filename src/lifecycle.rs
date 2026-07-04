//! ターン／計画／サブタスクのライフサイクル hook（副作用専用）。
//!
//! ホストはここに外部連携（チケット起票・進捗記録など）を載せる。
//! **本筋の ReAct ループを駆動・変更してはならない**（`run_turn` の再入、
//! キューや trace の直接書き換え、Answer の差し替えは禁止）。
//!
//! [`HostScratch`] はターン専用の入れ子 JSON 袋で、**プロンプト／LLM コンテキストには載せない**。
//! - `turn`: ターン／計画レベルの標識（seed・親チケットなど）
//! - `subtasks.{id}`: 各サブタスク専用（子チケットなど）。キーは **subtask id**（配列ではない）
//!
//! hook には [`HostView`] を渡す。**参照は袋全体、書き込みは自ノードのみ**。
//! エラーは hook 内で処理し、呼び出し元へ伝播させないこと。
//!
//! 詳細: `doc/lifecycle.md`。

use std::collections::BTreeMap;

use serde_json::{json, Map, Value};

use crate::plan::{PlanArtifact, Subtask};

/// ターン専用のホスト状態（LLM コンテキストに出さない）。
///
/// ```json
/// {
///   "turn": { "project_id": 1, "ticket_id": 10, "parent_ticket_id": 42 },
///   "subtasks": {
///     "1": { "child_ticket_id": 7 },
///     "2": { "child_ticket_id": 8 }
///   }
/// }
/// ```
///
/// `run_turn` 開始時にクリアされ、任意の seed（`turn` 領域のみ）がマージされたあと
/// `on_turn_started` が呼ばれる。
#[derive(Debug, Clone, Default)]
pub struct HostScratch {
    turn: Map<String, Value>,
    /// subtask id → そのサブタスク専用ノード。
    subtasks: BTreeMap<u32, Map<String, Value>>,
}

impl HostScratch {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.turn.clear();
        self.subtasks.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.turn.is_empty() && self.subtasks.is_empty()
    }

    /// 袋全体を JSON オブジェクトにする（参照・永続化用）。
    pub fn to_value(&self) -> Value {
        let mut subtasks = Map::new();
        for (id, node) in &self.subtasks {
            subtasks.insert(id.to_string(), Value::Object(node.clone()));
        }
        json!({
            "turn": Value::Object(self.turn.clone()),
            "subtasks": Value::Object(subtasks),
        })
    }

    pub fn turn(&self) -> &Map<String, Value> {
        &self.turn
    }

    pub fn subtask(&self, id: u32) -> Option<&Map<String, Value>> {
        self.subtasks.get(&id)
    }

    /// seed 用: `turn` 領域へエントリを書く（UI で選んだ project / ticket など）。
    pub fn turn_insert(&mut self, key: impl Into<String>, value: impl Into<Value>) {
        self.turn.insert(key.into(), value.into());
    }

    pub fn turn_get(&self, key: &str) -> Option<&Value> {
        self.turn.get(key)
    }

    pub fn turn_get_i64(&self, key: &str) -> Option<i64> {
        value_as_i64(self.turn.get(key)?)
    }

    pub fn turn_get_str(&self, key: &str) -> Option<&str> {
        self.turn.get(key).and_then(Value::as_str)
    }

    pub fn subtask_get(&self, id: u32, key: &str) -> Option<&Value> {
        self.subtasks.get(&id)?.get(key)
    }

    pub fn subtask_get_i64(&self, id: u32, key: &str) -> Option<i64> {
        value_as_i64(self.subtask_get(id, key)?)
    }

    /// 他袋の `turn` を上書きマージする（seed）。`subtasks` はマージしない。
    pub fn merge_turn_seed(&mut self, other: HostScratch) {
        self.turn.extend(other.turn);
    }

    fn ensure_subtask_node(&mut self, id: u32) -> &mut Map<String, Value> {
        self.subtasks.entry(id).or_default()
    }
}

fn value_as_i64(v: &Value) -> Option<i64> {
    v.as_i64()
        .or_else(|| v.as_u64().and_then(|u| i64::try_from(u).ok()))
        .or_else(|| v.as_str()?.parse().ok())
}

/// hook から見える袋のビュー。参照は全体、書き込みは [`WriteScope`] の自ノードのみ。
pub struct HostView<'a> {
    scratch: &'a mut HostScratch,
    write: WriteScope,
}

/// 書き込み可能なノード。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteScope {
    /// `turn` オブジェクト（turn / plan / turn_finished hook）。
    Turn,
    /// `subtasks.{id}`（そのサブタスクの start / finished hook）。
    Subtask(u32),
}

impl<'a> HostView<'a> {
    pub fn new(scratch: &'a mut HostScratch, write: WriteScope) -> Self {
        Self { scratch, write }
    }

    /// 同じ write scope で再借用する（`CompositeLifecycle` 用）。
    pub fn reborrow(&mut self) -> HostView<'_> {
        HostView {
            scratch: &mut *self.scratch,
            write: self.write,
        }
    }

    pub fn write_scope(&self) -> WriteScope {
        self.write
    }

    /// 袋全体の JSON（読み取り専用のスナップショット）。
    pub fn to_value(&self) -> Value {
        self.scratch.to_value()
    }

    pub fn turn(&self) -> &Map<String, Value> {
        self.scratch.turn()
    }

    pub fn subtask(&self, id: u32) -> Option<&Map<String, Value>> {
        self.scratch.subtask(id)
    }

    pub fn turn_get(&self, key: &str) -> Option<&Value> {
        self.scratch.turn_get(key)
    }

    pub fn turn_get_i64(&self, key: &str) -> Option<i64> {
        self.scratch.turn_get_i64(key)
    }

    pub fn turn_get_str(&self, key: &str) -> Option<&str> {
        self.scratch.turn_get_str(key)
    }

    pub fn subtask_get(&self, id: u32, key: &str) -> Option<&Value> {
        self.scratch.subtask_get(id, key)
    }

    pub fn subtask_get_i64(&self, id: u32, key: &str) -> Option<i64> {
        self.scratch.subtask_get_i64(id, key)
    }

    /// 自ノードへ書く。
    pub fn insert(&mut self, key: impl Into<String>, value: impl Into<Value>) {
        let key = key.into();
        let value = value.into();
        match self.write {
            WriteScope::Turn => {
                self.scratch.turn.insert(key, value);
            }
            WriteScope::Subtask(id) => {
                self.scratch.ensure_subtask_node(id).insert(key, value);
            }
        }
    }

    /// 自ノードから削除する。
    pub fn remove(&mut self, key: &str) -> Option<Value> {
        match self.write {
            WriteScope::Turn => self.scratch.turn.remove(key),
            WriteScope::Subtask(id) => self.scratch.subtasks.get_mut(&id)?.remove(key),
        }
    }

    /// 自ノードにキーがあるか。
    pub fn contains(&self, key: &str) -> bool {
        match self.write {
            WriteScope::Turn => self.scratch.turn.contains_key(key),
            WriteScope::Subtask(id) => self
                .scratch
                .subtasks
                .get(&id)
                .is_some_and(|n| n.contains_key(key)),
        }
    }

    /// 自ノードから読む。
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self.write {
            WriteScope::Turn => self.scratch.turn.get(key),
            WriteScope::Subtask(id) => self.scratch.subtasks.get(&id)?.get(key),
        }
    }

    pub fn get_i64(&self, key: &str) -> Option<i64> {
        value_as_i64(self.get(key)?)
    }

    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Value::as_str)
    }
}

/// ホストが登録するライフサイクル観測点。
///
/// すべてのメソッドに既定の空実装がある。必要な点だけオーバーライドする。
/// `host` は [`HostView`]: 参照は袋全体、書き込みは自ノードのみ。
pub trait TurnLifecycle: Send + Sync {
    /// `run_turn` 入口（記憶注入の前）。seed 適用済み。指示文から ID を `turn` へ書いてよい。
    fn on_turn_started(&self, _user_input: &str, _host: HostView<'_>) {}

    /// 計画が確定し、`resolve_plan` 適用後（実行開始前。skip も含む）。
    fn on_plan_finished(&self, _user_input: &str, _plan: &PlanArtifact, _host: HostView<'_>) {}

    /// サブタスク実行直前。`index` は 0 始まりの実行順。書き込み先は `subtasks.{subtask.id}`。
    fn on_subtask_started(
        &self,
        _user_input: &str,
        _plan: &PlanArtifact,
        _subtask: &Subtask,
        _index: usize,
        _host: HostView<'_>,
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
        _host: HostView<'_>,
    ) {
    }

    /// ターン完了（session / diary 記録と同じタイミング）。書き込み先は `turn`。
    fn on_turn_finished(
        &self,
        _user_input: &str,
        _answer: &str,
        _plan: Option<&PlanArtifact>,
        _steps_used: usize,
        _host: HostView<'_>,
    ) {
    }
}

/// 何もしない既定実装。
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopLifecycle;

impl TurnLifecycle for NoopLifecycle {}

/// 複数 hook を順に呼ぶ（いずれも本筋には影響しない）。同一袋・同一 write scope を共有する。
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
    fn on_turn_started(&self, user_input: &str, mut host: HostView<'_>) {
        for h in &self.hooks {
            h.on_turn_started(user_input, host.reborrow());
        }
    }

    fn on_plan_finished(&self, user_input: &str, plan: &PlanArtifact, mut host: HostView<'_>) {
        for h in &self.hooks {
            h.on_plan_finished(user_input, plan, host.reborrow());
        }
    }

    fn on_subtask_started(
        &self,
        user_input: &str,
        plan: &PlanArtifact,
        subtask: &Subtask,
        index: usize,
        mut host: HostView<'_>,
    ) {
        for h in &self.hooks {
            h.on_subtask_started(user_input, plan, subtask, index, host.reborrow());
        }
    }

    fn on_subtask_finished(
        &self,
        user_input: &str,
        plan: &PlanArtifact,
        subtask: &Subtask,
        answer: &str,
        steps_used: usize,
        mut host: HostView<'_>,
    ) {
        for h in &self.hooks {
            h.on_subtask_finished(user_input, plan, subtask, answer, steps_used, host.reborrow());
        }
    }

    fn on_turn_finished(
        &self,
        user_input: &str,
        answer: &str,
        plan: Option<&PlanArtifact>,
        steps_used: usize,
        mut host: HostView<'_>,
    ) {
        for h in &self.hooks {
            h.on_turn_finished(user_input, answer, plan, steps_used, host.reborrow());
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
        fn on_turn_started(&self, user_input: &str, host: HostView<'_>) {
            let id = host.turn_get_i64("ticket_id").unwrap_or(-1);
            self.events
                .lock()
                .unwrap()
                .push(format!("turn_started:{user_input}:ticket={id}"));
        }

        fn on_plan_finished(&self, _user_input: &str, plan: &PlanArtifact, mut host: HostView<'_>) {
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
            mut host: HostView<'_>,
        ) {
            let parent = host.turn_get_i64("parent_ticket").unwrap_or(-1);
            host.insert("child_ticket", subtask.id as i64);
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
            host: HostView<'_>,
        ) {
            let child = host.get_i64("child_ticket").unwrap_or(-1);
            self.events.lock().unwrap().push(format!(
                "subtask_finished:{}:{answer}:child={child}",
                subtask.id
            ));
        }

        fn on_turn_finished(
            &self,
            _user_input: &str,
            answer: &str,
            _plan: Option<&PlanArtifact>,
            _steps_used: usize,
            host: HostView<'_>,
        ) {
            let parent = host.turn_get_i64("parent_ticket").unwrap_or(-1);
            let child1 = host.subtask_get_i64(1, "child_ticket").unwrap_or(-1);
            self.events.lock().unwrap().push(format!(
                "turn_finished:{answer}:parent={parent}:child1={child1}"
            ));
        }
    }

    #[test]
    fn to_value_is_nested_turn_and_subtasks_map() {
        let mut s = HostScratch::new();
        s.turn_insert("ticket_id", 10);
        s.ensure_subtask_node(2).insert("child_ticket".into(), json!(8));
        let v = s.to_value();
        assert_eq!(v["turn"]["ticket_id"], 10);
        assert_eq!(v["subtasks"]["2"]["child_ticket"], 8);
        assert!(v["subtasks"].as_object().unwrap().get("0").is_none());
    }

    #[test]
    fn write_scope_turn_does_not_create_subtask_nodes() {
        let mut s = HostScratch::new();
        {
            let mut view = HostView::new(&mut s, WriteScope::Turn);
            view.insert("parent_ticket", 42);
        }
        assert_eq!(s.turn_get_i64("parent_ticket"), Some(42));
        assert!(s.subtask(1).is_none());
    }

    #[test]
    fn write_scope_subtask_only_touches_own_node() {
        let mut s = HostScratch::new();
        s.turn_insert("parent_ticket", 99);
        {
            let mut view = HostView::new(&mut s, WriteScope::Subtask(1));
            assert_eq!(view.turn_get_i64("parent_ticket"), Some(99));
            view.insert("child_ticket", 7);
        }
        assert_eq!(s.subtask_get_i64(1, "child_ticket"), Some(7));
        assert!(s.subtask(2).is_none());
        assert!(!s.turn().contains_key("child_ticket"));
    }

    #[test]
    fn merge_turn_seed_ignores_subtasks() {
        let mut a = HostScratch::new();
        a.turn_insert("x", 1);
        let mut b = HostScratch::new();
        b.turn_insert("x", 2);
        b.ensure_subtask_node(1).insert("y".into(), json!(3));
        a.merge_turn_seed(b);
        assert_eq!(a.turn_get_i64("x"), Some(2));
        assert!(a.subtask(1).is_none());
    }

    #[test]
    fn composite_shares_nested_scratch() {
        let a = Arc::new(Recorder::default());
        let composite = CompositeLifecycle::new(vec![a.clone()]);
        let mut scratch = HostScratch::new();
        scratch.turn_insert("ticket_id", 10);
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
        composite.on_turn_started("hi", HostView::new(&mut scratch, WriteScope::Turn));
        composite.on_plan_finished("hi", &plan, HostView::new(&mut scratch, WriteScope::Turn));
        composite.on_subtask_started(
            "hi",
            &plan,
            &plan.subtasks[0],
            0,
            HostView::new(&mut scratch, WriteScope::Subtask(1)),
        );
        composite.on_subtask_finished(
            "hi",
            &plan,
            &plan.subtasks[0],
            "done",
            1,
            HostView::new(&mut scratch, WriteScope::Subtask(1)),
        );
        composite.on_turn_finished(
            "hi",
            "final",
            Some(&plan),
            1,
            HostView::new(&mut scratch, WriteScope::Turn),
        );
        assert_eq!(scratch.turn_get_i64("parent_ticket"), Some(99));
        assert_eq!(scratch.subtask_get_i64(1, "child_ticket"), Some(1));
        assert_eq!(
            a.events.lock().unwrap().as_slice(),
            [
                "turn_started:hi:ticket=10",
                "plan_finished:s",
                "subtask_started:0:1:parent=99",
                "subtask_finished:1:done:child=1",
                "turn_finished:final:parent=99:child1=1",
            ]
        );
    }
}
