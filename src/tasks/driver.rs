use std::fmt;

use crate::action::{Action, Observation, TurnTrace};
use crate::plan::Subtask;
use crate::tool::{execute_action, ToolRuntime};
use crate::tool_display::eprintln_tool_execution;

use super::audit::{audit_trace, TaskExecutionAudit};
use super::registry::TaskRegistry;
use super::spec::{apply_template_value, TaskDefinition};

/// ステップドライバの実行結果（ReAct ループと同型の trace）。
#[derive(Debug)]
pub struct StepDriverResult {
    pub task_id: String,
    pub trace: TurnTrace,
    pub answer: String,
    pub steps_used: usize,
    pub audit: TaskExecutionAudit,
}

#[derive(Debug)]
pub enum StepDriverError {
    UnknownTask { id: String },
    NoContract { id: String },
    StepFailed {
        order: u32,
        method: String,
        output: String,
    },
}

impl fmt::Display for StepDriverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownTask { id } => write!(f, "unknown task: {id}"),
            Self::NoContract { id } => write!(f, "task '{id}' has no execution contract"),
            Self::StepFailed { order, method, output } => {
                write!(f, "step {order} ({method}) failed: {output}")
            }
        }
    }
}

impl std::error::Error for StepDriverError {}

impl TaskRegistry {
    /// サブタスクが `steps[]` 契約を持ち、ステップドライバで実行可能か。
    pub fn use_step_driver(&self, subtask: &Subtask) -> bool {
        subtask
            .task
            .as_ref()
            .and_then(|id| self.get(id))
            .is_some_and(|d| d.has_execution_contract())
    }

    /// 契約どおり `steps[]` を順に `execute_action` する（LLM なし）。
    pub fn run_subtask_driver(
        &self,
        subtask: &Subtask,
        tools: &mut ToolRuntime,
        verbose: bool,
        show_tool_output: bool,
    ) -> Result<StepDriverResult, StepDriverError> {
        let task_id = subtask
            .task
            .as_ref()
            .ok_or_else(|| StepDriverError::NoContract {
                id: "(no task id)".into(),
            })?
            .clone();
        let def = self
            .get(&task_id)
            .ok_or(StepDriverError::UnknownTask { id: task_id.clone() })?;
        if !def.has_execution_contract() {
            return Err(StepDriverError::NoContract { id: task_id });
        }
        let params = super::registry::merge_params(&def.default_params, &subtask.params);
        run_task_steps(def, &params, tools, verbose, show_tool_output)
    }
}

fn run_task_steps(
    def: &TaskDefinition,
    params: &serde_json::Value,
    tools: &mut ToolRuntime,
    verbose: bool,
    show_tool_output: bool,
) -> Result<StepDriverResult, StepDriverError> {
    let mut trace = TurnTrace::default();
    if verbose {
        eprintln!(
            "[driver] task '{}' — {} step(s)",
            def.id,
            def.ordered_steps().len()
        );
    }

    for step in def.ordered_steps() {
        let args = apply_template_value(&step.args, params);
        if verbose {
            eprintln!("[driver] step {}: {}({})", step.order, step.method, args);
        }
        let action = Action::new(0, step.method.clone(), args);
        let observation = execute_action(tools, &action);
        if show_tool_output {
            eprintln_tool_execution(&action, &observation);
        }
        let invoke_id = observation.invoke_id;
        trace.push_action(Action::new(invoke_id, step.method.clone(), action.args));
        if !observation.ok {
            let output = observation.output.clone();
            trace.push_observation(observation);
            return Err(StepDriverError::StepFailed {
                order: step.order,
                method: step.method.clone(),
                output,
            });
        }
        trace.push_observation(observation);
    }

    let audit = audit_trace(def, params, &trace);
    let answer = format_driver_answer(def, &trace, &audit);
    let steps_used = trace.actions.len();
    Ok(StepDriverResult {
        task_id: def.id.clone(),
        trace,
        answer,
        steps_used,
        audit,
    })
}

fn format_driver_answer(
    def: &TaskDefinition,
    trace: &TurnTrace,
    audit: &TaskExecutionAudit,
) -> String {
    let tail = trace
        .observations
        .last()
        .map(|o| summarize_observation(o))
        .unwrap_or_default();
    if audit.complete {
        format!(
            "[step-driver] task '{}' complete ({steps} step(s)).\n{tail}",
            def.id,
            steps = trace.actions.len()
        )
    } else {
        format!(
            "[step-driver] task '{}' finished but audit incomplete: {}\n{tail}",
            def.id, audit.message
        )
    }
}

fn summarize_observation(obs: &Observation) -> String {
    const MAX: usize = 800;
    if obs.output.len() <= MAX {
        obs.output.clone()
    } else {
        format!("{}…", &obs.output[..MAX])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::Subtask;
    use serde_json::json;

    #[test]
    fn driver_list_dir_runs_one_step() {
        let reg = TaskRegistry::builtin();
        let sub = Subtask {
            id: 1,
            task: Some("list_dir".into()),
            params: json!({ "path": "src" }),
            goal: String::new(),
            done_when: String::new(),
        };
        let mut tools = ToolRuntime::new();
        let r = reg.run_subtask_driver(&sub, &mut tools, false, false).unwrap();
        assert_eq!(r.steps_used, 1);
        assert_eq!(r.trace.actions[0].tool, "list_dir");
        assert!(r.audit.complete);
        assert!(r.answer.contains("complete"));
    }

    #[test]
    fn driver_write_file_verify_order() {
        let reg = TaskRegistry::builtin();
        let path = "tmp/driver_test.txt";
        let _ = std::fs::remove_file(path);
        let sub = Subtask {
            id: 1,
            task: Some("write_file_verify".into()),
            params: json!({
                "path": path,
                "content": "driver-test\n"
            }),
            goal: String::new(),
            done_when: String::new(),
        };
        let mut tools = ToolRuntime::new();
        let r = reg.run_subtask_driver(&sub, &mut tools, false, false).unwrap();
        assert_eq!(r.steps_used, 2);
        assert_eq!(
            r.trace.actions.iter().map(|a| a.tool.as_str()).collect::<Vec<_>>(),
            vec!["write_file", "read_file"]
        );
        assert!(r.audit.complete);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn generic_has_no_driver_contract() {
        let reg = TaskRegistry::builtin();
        let sub = Subtask {
            id: 1,
            task: Some("generic".into()),
            params: json!({}),
            goal: "free".into(),
            done_when: String::new(),
        };
        assert!(!reg.use_step_driver(&sub));
    }
}
