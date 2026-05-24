//! ReAct ループ中の LLM / ツール実行をホストへ通知するオブザーバ。

use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;

use crate::action::{AgentStep, Observation};
use crate::context_metrics::ContextUsage;
use crate::plan::PlanArtifact;

pub type TurnObserver = Arc<dyn Fn(TurnStepEvent) + Send + Sync>;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AgentStepDto {
    #[serde(rename = "thought")]
    Thought { text: String },
    #[serde(rename = "action")]
    Action {
        invoke_id: u64,
        tool: String,
        args: Value,
    },
    #[serde(rename = "answer")]
    Answer { text: String },
}

impl From<&AgentStep> for AgentStepDto {
    fn from(step: &AgentStep) -> Self {
        match step {
            AgentStep::Thought(text) => Self::Thought { text: text.clone() },
            AgentStep::Action(action) => Self::Action {
                invoke_id: action.invoke_id,
                tool: action.tool.clone(),
                args: action.args.clone(),
            },
            AgentStep::Answer(text) => Self::Answer { text: text.clone() },
        }
    }
}

/// 1 回の LLM 呼び出しまたはツール観測。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum TurnStepEvent {
    Llm {
        layer: String,
        step_index: usize,
        metrics: String,
        output: String,
        step: AgentStepDto,
    },
    Observation {
        layer: String,
        step_index: usize,
        tool: String,
        ok: bool,
        output: String,
    },
    /// 計画層完了時の [`PlanArtifact`] サマリー。
    Plan {
        layer: String,
        summary: String,
        display: String,
        skip_execution: bool,
        subtask_count: usize,
    },
    /// LLM 呼び出し直前（GPU 待ちの間も UI にフェーズを出す）。
    PhaseStarted {
        layer: String,
        label: String,
    },
}

pub fn emit_phase_started(observer: Option<&TurnObserver>, layer: &str, label: &str) {
    let Some(obs) = observer else {
        return;
    };
    obs(TurnStepEvent::PhaseStarted {
        layer: layer.to_string(),
        label: label.to_string(),
    });
}

pub fn emit_llm_step(
    observer: Option<&TurnObserver>,
    layer: &str,
    step_index: usize,
    usage: &ContextUsage,
    step: &AgentStep,
) {
    let Some(obs) = observer else {
        return;
    };
    obs(TurnStepEvent::Llm {
        layer: layer.to_string(),
        step_index,
        metrics: format!("[context step] {usage}"),
        output: usage.completion_body.clone(),
        step: AgentStepDto::from(step),
    });
}

pub fn emit_plan_artifact(
    observer: Option<&TurnObserver>,
    layer: &str,
    plan: &PlanArtifact,
    display: &str,
) {
    let Some(obs) = observer else {
        return;
    };
    obs(TurnStepEvent::Plan {
        layer: layer.to_string(),
        summary: plan.summary.clone(),
        display: display.to_string(),
        skip_execution: plan.skip_execution,
        subtask_count: plan.subtasks.len(),
    });
}

pub fn emit_observation_step(
    observer: Option<&TurnObserver>,
    layer: &str,
    step_index: usize,
    tool: &str,
    observation: &Observation,
) {
    let Some(obs) = observer else {
        return;
    };
    obs(TurnStepEvent::Observation {
        layer: layer.to_string(),
        step_index,
        tool: tool.to_string(),
        ok: observation.ok,
        output: observation.output.clone(),
    });
}
