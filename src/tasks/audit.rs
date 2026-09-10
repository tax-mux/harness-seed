//! タスク定義の必須実行順序と `TurnTrace` の照合。

use serde_json::Value;

use crate::action::TurnTrace;

use super::spec::{apply_template_value, TaskDefinition};

/// ステップ引数の照合モード。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ArgAuditMode {
    /// ツール名の順序のみ（従来）。
    #[default]
    Off,
    /// 引数不一致をメッセージに含めるが `complete` は順序のみで判定。
    Soft,
    /// 引数不一致も失敗とする。
    Hard,
}

impl ArgAuditMode {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "soft" => Self::Soft,
            "hard" => Self::Hard,
            _ => Self::Off,
        }
    }
}

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
pub fn audit_trace(def: &TaskDefinition, params: &Value, trace: &TurnTrace) -> TaskExecutionAudit {
    audit_trace_with_mode(def, params, trace, ArgAuditMode::Off)
}

/// [`ArgAuditMode`] 付き監査。
pub fn audit_trace_with_mode(
    def: &TaskDefinition,
    params: &Value,
    trace: &TurnTrace,
    arg_mode: ArgAuditMode,
) -> TaskExecutionAudit {
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

    let mut successful_actions = Vec::new();
    for action in &trace.actions {
        let ok = trace
            .observations
            .iter()
            .find(|o| o.invoke_id == action.invoke_id)
            .is_some_and(|o| o.ok);
        if ok {
            successful_actions.push(action);
        }
    }

    let expected = expected_args(def, params);
    let mut steps = Vec::new();
    let mut action_iter = successful_actions.into_iter().peekable();
    let mut all_ok = true;
    let mut arg_notes = Vec::new();

    for step in required {
        let expected_method = step.method.as_str();
        let expected_args_val = expected
            .iter()
            .find(|(o, m, _)| *o == step.order && m == &step.method)
            .map(|(_, _, v)| v.clone())
            .unwrap_or_else(|| step.args.clone());

        let mut satisfied = false;
        let mut arg_ok = true;
        while let Some(next) = action_iter.peek() {
            if next.tool == expected_method {
                let action = action_iter.next().expect("peeked");
                if arg_mode != ArgAuditMode::Off && action.args != expected_args_val {
                    arg_ok = false;
                    arg_notes.push(format!(
                        "{}:{} args mismatch (got {}, expected {})",
                        step.order, step.method, action.args, expected_args_val
                    ));
                }
                satisfied = true;
                break;
            }
            action_iter.next();
        }
        if !satisfied {
            all_ok = false;
        } else if arg_mode == ArgAuditMode::Hard && !arg_ok {
            all_ok = false;
            satisfied = false;
        }
        steps.push(StepAudit {
            order: step.order,
            method: step.method.clone(),
            satisfied,
        });
    }

    let mut message = if all_ok {
        "all required methods executed in order".into()
    } else {
        let missing: Vec<_> = steps
            .iter()
            .filter(|s| !s.satisfied)
            .map(|s| format!("{}:{}", s.order, s.method))
            .collect();
        format!("missing or out-of-order methods: {}", missing.join(", "))
    };
    if arg_mode == ArgAuditMode::Soft && !arg_notes.is_empty() {
        message = format!("{message}; arg warnings: {}", arg_notes.join("; "));
    } else if arg_mode == ArgAuditMode::Hard && !arg_notes.is_empty() && !all_ok {
        message = format!("{message}; {}", arg_notes.join("; "));
    }

    TaskExecutionAudit {
        complete: all_ok,
        steps,
        message,
    }
}

/// 監査用に期待する引数（展開済み）。
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
    fn hard_arg_audit_fails_on_mismatch() {
        let def = list_dir_task();
        let mut trace = TurnTrace::default();
        trace.push_action(Action::new(1, "list_dir", json!({"path": "/tmp"})));
        trace.push_observation(Observation::success(1, "ok"));
        let soft = audit_trace_with_mode(&def, &json!({}), &trace, ArgAuditMode::Soft);
        assert!(soft.complete);
        assert!(soft.message.contains("arg warnings"));
        let hard = audit_trace_with_mode(&def, &json!({}), &trace, ArgAuditMode::Hard);
        assert!(!hard.complete);
    }
}
