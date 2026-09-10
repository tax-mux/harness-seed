//! タスク定義の必須実行順序と `TurnTrace` の照合（スケルトン）。

use serde_json::Value;

use crate::action::TurnTrace;

use super::spec::{apply_template_value, TaskDefinition};

/// 1 必須ステップの照合結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepAudit {
    pub order: u32,
    pub method: String,
    pub satisfied: bool,
}

/// ターン trace とタスク契約の照合結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskExecutionAudit {
    pub complete: bool,
    pub steps: Vec<StepAudit>,
    pub message: String,
}

impl TaskExecutionAudit {
    pub fn ok() -> Self {
        Self {
            complete: true,
            steps: vec![],
            message: "no execution contract".into(),
        }
    }
}

/// 成功した Observation に対応するツール呼び出し列を、定義順と照合する。
///
/// スケルトン: **ツール名の順序のみ**一致すればよい（引数の完全一致は未実装）。
pub fn audit_trace(def: &TaskDefinition, _params: &Value, trace: &TurnTrace) -> TaskExecutionAudit {
    let policy = def.resolved_tool_policy();

    let mut forbidden = Vec::new();
    for action in &trace.actions {
        let ok = trace
            .observations
            .iter()
            .find(|o| o.invoke_id == action.invoke_id)
            .is_some_and(|o| o.ok);
        if ok && !policy.is_allowed(&action.tool) {
            forbidden.push(action.tool.clone());
        }
    }
    if !forbidden.is_empty() {
        return TaskExecutionAudit {
            complete: false,
            steps: vec![],
            message: format!("forbidden tools called: {}", forbidden.join(", ")),
        };
    }

    let required = def.ordered_required_steps();
    if required.is_empty() {
        return TaskExecutionAudit::ok();
    }

    let mut successful_tools = Vec::new();
    for action in &trace.actions {
        let ok = trace
            .observations
            .iter()
            .find(|o| o.invoke_id == action.invoke_id)
            .is_some_and(|o| o.ok);
        if ok {
            successful_tools.push(action.tool.as_str());
        }
    }

    let mut steps = Vec::new();
    let mut tool_iter = successful_tools.into_iter().peekable();
    let mut all_ok = true;

    for step in required {
        let expected = step.method.as_str();
        let mut satisfied = false;
        while let Some(next) = tool_iter.peek() {
            if *next == expected {
                tool_iter.next();
                satisfied = true;
                break;
            }
            tool_iter.next();
        }
        if !satisfied {
            all_ok = false;
        }
        steps.push(StepAudit {
            order: step.order,
            method: step.method.clone(),
            satisfied,
        });
    }

    let message = if all_ok {
        "all required methods executed in order".into()
    } else {
        let missing: Vec<_> = steps
            .iter()
            .filter(|s| !s.satisfied)
            .map(|s| format!("{}:{}", s.order, s.method))
            .collect();
        format!("missing or out-of-order methods: {}", missing.join(", "))
    };

    TaskExecutionAudit {
        complete: all_ok,
        steps,
        message,
    }
}

/// 監査用に期待する引数（展開済み）。将来の厳密照合向け。
pub fn expected_args(def: &TaskDefinition, params: &Value) -> Vec<(u32, String, Value)> {
    def.ordered_required_steps()
        .into_iter()
        .map(|s| (s.order, s.method.clone(), apply_template_value(&s.args, params)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::{Action, Observation};
    use serde_json::json;

    fn list_dir_task() -> TaskDefinition {
        serde_json::from_str(
            r#"{
            "id": "list_dir",
            "summary": "list",
            "steps": [{"order": 1, "method": "list_dir", "args": {"path": "."}}]
        }"#,
        )
        .unwrap()
    }

    #[test]
    fn audit_passes_when_tools_in_order() {
        let def = list_dir_task();
        let mut trace = TurnTrace::default();
        trace.push_action(Action::new(1, "list_dir", json!({"path": "."})));
        trace.push_observation(Observation::success(1, "ok"));
        let audit = audit_trace(&def, &json!({}), &trace);
        assert!(audit.complete);
    }

    #[test]
    fn audit_fails_on_forbidden_tool() {
        let def: TaskDefinition = serde_json::from_str(
            r#"{
            "id": "ctx",
            "summary": "ctx",
            "steps": [{"order": 1, "method": "get_compose_form", "args": {}}],
            "tool_policy": { "deny": ["set_compose_form"] }
        }"#,
        )
        .unwrap();
        let mut trace = TurnTrace::default();
        trace.push_action(Action::new(1, "get_compose_form", json!({})));
        trace.push_observation(Observation::success(1, "ok"));
        trace.push_action(Action::new(2, "set_compose_form", json!({"body": "x"})));
        trace.push_observation(Observation::success(2, "ok"));
        let audit = audit_trace(&def, &json!({}), &trace);
        assert!(!audit.complete);
        assert!(audit.message.contains("forbidden"));
    }

    #[test]
    fn audit_passes_when_extra_tools_precede_required() {
        let def: TaskDefinition = serde_json::from_str(
            r#"{
            "id": "compose_write",
            "summary": "write",
            "steps": [{"order": 1, "method": "set_compose_form", "args": {}}],
            "tool_policy": { "allow": ["get_compose_form", "set_compose_form"] }
        }"#,
        )
        .unwrap();
        let mut trace = TurnTrace::default();
        trace.push_action(Action::new(1, "get_compose_form", json!({})));
        trace.push_observation(Observation::success(1, "ok"));
        trace.push_action(Action::new(2, "set_compose_form", json!({"body": "x"})));
        trace.push_observation(Observation::success(2, "ok"));
        let audit = audit_trace(&def, &json!({}), &trace);
        assert!(audit.complete, "{}", audit.message);
    }

    #[test]
    fn audit_fails_when_wrong_order() {
        let def: TaskDefinition = serde_json::from_str(
            r#"{
            "id": "w",
            "summary": "w",
            "steps": [
                {"order": 1, "method": "write_file", "args": {}},
                {"order": 2, "method": "read_file", "args": {}}
            ]
        }"#,
        )
        .unwrap();
        let mut trace = TurnTrace::default();
        trace.push_action(Action::new(1, "read_file", json!({})));
        trace.push_observation(Observation::success(1, "ok"));
        trace.push_action(Action::new(2, "write_file", json!({})));
        trace.push_observation(Observation::success(2, "ok"));
        let audit = audit_trace(&def, &json!({}), &trace);
        assert!(!audit.complete);
    }
}
