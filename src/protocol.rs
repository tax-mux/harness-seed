//! CUI とエージェントの間の JSON ワイヤプロトコル（ライブラリ埋め込み用）。

use std::fmt;
use std::io::{self, BufRead, Write};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::action::{Action, Observation, TurnTrace};
use crate::brain::AgentBrain;
use crate::context_metrics::{TokenSource, TurnContextSummary};
use crate::plan::{PlanArtifact, Subtask};
use crate::react::{ReActError, ReActLoop, SubtaskExecResult, TurnResult};
use crate::runtime::RuntimeEnvironment;

/// ワイヤプロトコルのスキーマバージョン。
pub const WIRE_VERSION: u32 = 1;

/// JSON 1 行のリクエスト（`type` で分岐）。
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WireRequest {
    /// 1 ターン実行。
    Turn {
        user_input: String,
        #[serde(default)]
        options: TurnWireOptions,
    },
    /// REPL 短期記憶をクリア。
    SessionClear,
    /// 接続確認・環境情報。
    Ping,
}

/// ターン要求のオプション。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct TurnWireOptions {
    /// `trace` を載せる（既定: true）。
    #[serde(default = "default_true")]
    pub include_trace: bool,
    /// `plan` / `subtask_results` を載せる（two_phase 時。既定: true）。
    #[serde(default = "default_true")]
    pub include_plan: bool,
    /// LLM コンテキスト計測サマリを載せる（既定: true）。
    #[serde(default = "default_true")]
    pub include_context: bool,
    /// observation の最大文字数（超過分は省略。未指定で切り詰めなし）。
    pub max_observation_chars: Option<usize>,
}

fn default_true() -> bool {
    true
}

/// JSON 1 行のレスポンス。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WireResponse {
    Turn {
        version: u32,
        ok: bool,
        answer: String,
        steps_used: usize,
        session_turns: usize,
        #[serde(skip_serializing_if = "Option::is_none")]
        trace: Option<TraceDto>,
        #[serde(skip_serializing_if = "Option::is_none")]
        plan: Option<PlanDto>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        subtask_results: Vec<SubtaskResultDto>,
        #[serde(skip_serializing_if = "Option::is_none")]
        context: Option<ContextSummaryDto>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<WireErrorBody>,
    },
    SessionClear {
        version: u32,
        ok: bool,
        session_turns: usize,
    },
    Ping {
        version: u32,
        runtime: RuntimeDto,
        harness_version: &'static str,
    },
    /// リクエスト JSON のパース失敗など。
    ProtocolError {
        version: u32,
        ok: bool,
        message: String,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct WireErrorBody {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeDto {
    pub os: String,
    pub arch: String,
    pub shell_label: String,
    pub shell_program: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TraceDto {
    pub thoughts: Vec<String>,
    pub actions: Vec<ActionDto>,
    pub observations: Vec<ObservationDto>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ActionDto {
    pub invoke_id: u64,
    pub tool: String,
    pub args: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct ObservationDto {
    pub invoke_id: u64,
    pub ok: bool,
    pub output: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanDto {
    pub summary: String,
    pub skip_execution: bool,
    pub subtasks: Vec<SubtaskDto>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SubtaskDto {
    pub id: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    pub params: Value,
    pub goal: String,
    pub done_when: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SubtaskResultDto {
    pub id: u32,
    pub answer: String,
    pub steps_used: usize,
    pub used_step_driver: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContextSummaryDto {
    pub llm_calls: usize,
    pub prompt_chars: usize,
    pub completion_chars: usize,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub token_source: String,
}

#[derive(Debug)]
pub enum ProtocolError {
    JsonParse(serde_json::Error),
    JsonSerialize(serde_json::Error),
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::JsonParse(e) => write!(f, "invalid request JSON: {e}"),
            Self::JsonSerialize(e) => write!(f, "failed to serialize response: {e}"),
        }
    }
}

impl std::error::Error for ProtocolError {}

impl<E: AgentBrain> ReActLoop<E> {
    /// 1 件のワイヤリクエストを処理する。
    pub fn handle_wire_request(&mut self, request: WireRequest) -> WireResponse {
        match request {
            WireRequest::Turn {
                user_input,
                options,
            } => self.handle_turn_wire(user_input, options),
            WireRequest::SessionClear => {
                self.session.clear();
                WireResponse::SessionClear {
                    version: WIRE_VERSION,
                    ok: true,
                    session_turns: self.session.len(),
                }
            }
            WireRequest::Ping => WireResponse::Ping {
                version: WIRE_VERSION,
                runtime: runtime_dto(&self.blocks.runtime),
                harness_version: crate::VERSION,
            },
        }
    }

    /// JSON 文字列 1 件を処理し、レスポンス JSON 文字列を返す。
    pub fn handle_wire_json(&mut self, json_line: &str) -> Result<String, ProtocolError> {
        let request: WireRequest = serde_json::from_str(json_line).map_err(ProtocolError::JsonParse)?;
        let response = self.handle_wire_request(request);
        serde_json::to_string(&response).map_err(ProtocolError::JsonSerialize)
    }

    fn handle_turn_wire(&mut self, user_input: String, options: TurnWireOptions) -> WireResponse {
        match self.run_turn(&user_input) {
            Ok(result) => turn_response_ok(self.session.len(), &result, &options),
            Err(err) => turn_response_err(self.session.len(), &err),
        }
    }
}

fn turn_response_ok(session_turns: usize, result: &TurnResult, options: &TurnWireOptions) -> WireResponse {
    let trace = options
        .include_trace
        .then(|| trace_dto(&result.trace, options.max_observation_chars));
    let plan = options
        .include_plan
        .then(|| result.plan.as_ref().map(plan_dto))
        .flatten();
    let subtask_results = if options.include_plan {
        result
            .subtask_results
            .iter()
            .map(subtask_result_dto)
            .collect()
    } else {
        vec![]
    };
    let context = options
        .include_context
        .then(|| context_summary_dto(&result.context))
        .flatten();

    WireResponse::Turn {
        version: WIRE_VERSION,
        ok: true,
        answer: result.answer.clone(),
        steps_used: result.steps_used,
        session_turns,
        trace,
        plan,
        subtask_results,
        context,
        error: None,
    }
}

fn turn_response_err(session_turns: usize, err: &ReActError) -> WireResponse {
    let (code, message) = match err {
        ReActError::MaxStepsExceeded { limit } => (
            "max_steps_exceeded",
            format!("ReAct loop exceeded max steps ({limit})"),
        ),
    };
    WireResponse::Turn {
        version: WIRE_VERSION,
        ok: false,
        answer: String::new(),
        steps_used: 0,
        session_turns,
        trace: None,
        plan: None,
        subtask_results: vec![],
        context: None,
        error: Some(WireErrorBody {
            code: code.into(),
            message,
        }),
    }
}

fn runtime_dto(env: &RuntimeEnvironment) -> RuntimeDto {
    RuntimeDto {
        os: env.os.clone(),
        arch: env.arch.clone(),
        shell_label: env.shell_label.clone(),
        shell_program: env.shell_program.clone(),
    }
}

fn trace_dto(trace: &TurnTrace, max_obs: Option<usize>) -> TraceDto {
    TraceDto {
        thoughts: trace.thoughts.clone(),
        actions: trace.actions.iter().map(action_dto).collect(),
        observations: trace
            .observations
            .iter()
            .map(|o| observation_dto(o, max_obs))
            .collect(),
    }
}

fn action_dto(a: &Action) -> ActionDto {
    ActionDto {
        invoke_id: a.invoke_id,
        tool: a.tool.clone(),
        args: a.args.clone(),
    }
}

fn observation_dto(o: &Observation, max_chars: Option<usize>) -> ObservationDto {
    let output = match max_chars {
        Some(max) if o.output.chars().count() > max => {
            let truncated: String = o.output.chars().take(max).collect();
            format!("{truncated}…")
        }
        _ => o.output.clone(),
    };
    ObservationDto {
        invoke_id: o.invoke_id,
        ok: o.ok,
        output,
    }
}

fn plan_dto(plan: &PlanArtifact) -> PlanDto {
    PlanDto {
        summary: plan.summary.clone(),
        skip_execution: plan.skip_execution,
        subtasks: plan.subtasks.iter().map(subtask_dto).collect(),
    }
}

fn subtask_dto(st: &Subtask) -> SubtaskDto {
    SubtaskDto {
        id: st.id,
        task: st.task.clone(),
        params: st.params.clone(),
        goal: st.goal.clone(),
        done_when: st.done_when.clone(),
    }
}

fn subtask_result_dto(r: &SubtaskExecResult) -> SubtaskResultDto {
    SubtaskResultDto {
        id: r.id,
        answer: r.answer.clone(),
        steps_used: r.steps_used,
        used_step_driver: r.used_step_driver,
    }
}

fn context_summary_dto(ctx: &TurnContextSummary) -> Option<ContextSummaryDto> {
    if ctx.is_empty() {
        return None;
    }
    let token_source = match ctx.token_source {
        TokenSource::Api => "api",
        TokenSource::Estimated => "estimated",
    };
    Some(ContextSummaryDto {
        llm_calls: ctx.llm_calls,
        prompt_chars: ctx.prompt.chars,
        completion_chars: ctx.completion.chars,
        prompt_tokens: ctx.prompt_tokens,
        completion_tokens: ctx.completion_tokens,
        token_source: token_source.into(),
    })
}

/// パース失敗時のレスポンス JSON 文字列。
pub fn protocol_error_response(message: impl Into<String>) -> String {
    let resp = WireResponse::ProtocolError {
        version: WIRE_VERSION,
        ok: false,
        message: message.into(),
    };
    serde_json::to_string(&resp).unwrap_or_else(|_| {
        r#"{"type":"protocol_error","version":1,"ok":false,"message":"serialize failed"}"#
            .into()
    })
}

/// JSON Lines REPL（stdin 1 行 = 1 リクエスト、stdout 1 行 = 1 レスポンス。ログは stderr）。
pub fn run_json_repl<E: AgentBrain>(
    loop_engine: &mut ReActLoop<E>,
    verbose: bool,
) -> io::Result<()> {
    loop_engine.apply_cli_verbose(verbose);

    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let reader = stdin.lock();

    eprintln!(
        "HarnessSeed JSON REPL — one JSON object per line (protocol v{WIRE_VERSION})"
    );
    eprintln!("runtime: {}", loop_engine.blocks.runtime.summary_line());
    eprintln!("request types: turn | session_clear | ping");

    for line in reader.lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let out = match loop_engine.handle_wire_json(trimmed) {
            Ok(json) => json,
            Err(ProtocolError::JsonParse(e)) => protocol_error_response(e.to_string()),
            Err(ProtocolError::JsonSerialize(e)) => protocol_error_response(e.to_string()),
        };

        writeln!(stdout, "{out}")?;
        stdout.flush()?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brain::SimpleRuleBrain;

    #[test]
    fn turn_roundtrip_help() {
        let mut react = ReActLoop::with_defaults(SimpleRuleBrain::new());
        let req = r#"{"type":"turn","user_input":"help","options":{"include_trace":true}}"#;
        let out = react.handle_wire_json(req).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["type"], "turn");
        assert_eq!(v["ok"], true);
        assert!(v["answer"].as_str().unwrap().contains("echo"));
    }

    #[test]
    fn session_clear_wire() {
        let mut react = ReActLoop::with_defaults(SimpleRuleBrain::new());
        react.run_turn("help").unwrap();
        assert_eq!(react.session.len(), 1);
        let out = react
            .handle_wire_json(r#"{"type":"session_clear"}"#)
            .unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["type"], "session_clear");
        assert_eq!(v["session_turns"], 0);
    }

    #[test]
    fn ping_returns_runtime() {
        let mut react = ReActLoop::with_defaults(SimpleRuleBrain::new());
        let out = react.handle_wire_json(r#"{"type":"ping"}"#).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["type"], "ping");
        assert!(v["runtime"]["os"].is_string());
    }

    #[test]
    fn invalid_json_returns_protocol_error() {
        let mut react = ReActLoop::with_defaults(SimpleRuleBrain::new());
        let out = react.handle_wire_json("not json").unwrap_err();
        assert!(matches!(out, ProtocolError::JsonParse(_)));
    }
}
