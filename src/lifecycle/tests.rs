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
        outcome: &SubtaskOutcome,
        host: HostView<'_>,
    ) {
        let child = host.get_i64("child_ticket").unwrap_or(-1);
        self.events.lock().unwrap().push(format!(
            "subtask_finished:{}:{}:child={child}",
            subtask.id, outcome.message
        ));
    }

    fn on_turn_finished(
        &self,
        _user_input: &str,
        _plan: Option<&PlanArtifact>,
        outcome: &TurnOutcome,
        host: HostView<'_>,
    ) {
        let parent = host.turn_get_i64("parent_ticket").unwrap_or(-1);
        let child1 = host.subtask_get_i64(1, "child_ticket").unwrap_or(-1);
        self.events.lock().unwrap().push(format!(
            "turn_finished:{}:parent={parent}:child1={child1}",
            outcome.answer
        ));
    }
}

#[test]
fn to_value_is_nested_turn_and_subtasks_map() {
    let mut s = HostScratch::new();
    s.turn_insert("ticket_id", 10);
    s.ensure_subtask_node(2)
        .insert("child_ticket".into(), json!(8));
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
            depends_on: vec![],
        }],
        knowledge_sufficient: None,
        user_reply: None,
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
        &SubtaskOutcome::completed("done", 1),
        HostView::new(&mut scratch, WriteScope::Subtask(1)),
    );
    composite.on_turn_finished(
        "hi",
        Some(&plan),
        &TurnOutcome::completed("final", 1),
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
