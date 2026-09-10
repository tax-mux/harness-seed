//! HarnessSeed (`harness-seed`) — 組み込み用 ReAct ループの実行基盤。

pub mod action;
pub mod advance;
pub mod brain;
pub mod brave_search;
pub mod config;
pub mod context;
pub mod context_log;
pub mod context_map;
pub mod grep;
pub mod harness;
pub mod context_metrics;
pub mod layer;
pub mod llm;
pub mod plan;
pub mod protocol;
pub mod react;
pub mod runtime;
pub mod session;
pub mod tasks;
pub mod tool;
pub mod tool_display;
pub mod turn_observer;

pub use action::{Action, AgentStep, Observation, TurnTrace};
pub use advance::{
    format_recalled_progress, prepare_phase_recalled, restore_base_recalled, AdvanceConfig,
    AdvancePhaseNote, AdvancePhaseSummary, AdvanceProgress,
};
pub use brain::{AgentBrain, BrainMode, BrainPair, SimpleRuleBrain};
pub use brave_search::{BraveSearchConfig, BraveSearchError, WebSearchHit};
pub use config::{
    default_config_path, AppConfig, BraveSearchSection, ConfigError, LlmSection, LogRotationConfig,
    LogRotationSection, LogSection,
    PromptSection, ReactSection, ToolsSection,
};
pub use context::{
    format_plan_rule_prompt_preview, format_trace, ContextError, PromptBlocks, TurnPromptContext,
    REACT_SYSTEM_CORE, REACT_WEB_SEARCH_GUIDANCE,
};
pub use context_map::{
    analyze_messages, analyze_prompt_body, format_colormap, ContextSection, ContextSectionKind,
};
pub use context_log::{
    default_log_path, rotate_log_file, ContextLogEntry, ContextLogWriter, DEFAULT_CONTEXT_LOG_REL,
};
pub use context_metrics::{
    format_messages_body, ContextUsage, TextSize, TokenSource, TurnContextSummary,
};
pub use harness::{
    format_references_for_prompt, parse_harness, HarnessMailRefKind, HarnessParseError,
    HarnessReference, HarnessState, HarnessStatus,
};
pub use llm::{
    normalize_anthropic_base_url, normalize_gemini_base_url, normalize_lmstudio_base_url,
    normalize_ollama_base_url, parse_agent_step, AnthropicConnector, ChatMessage, ConnectorError,
    CompletionResult, GeminiConnector, LlmBrain, LlmConfig, LlmConnector, LlmConnectorKind,
    LlmProvider, LmStudioConnector, MockLlmConnector, OpenAiConnector, ParseError,
};
pub use layer::{run_layer_loop, run_plan_layer, LayerLoopOptions};
pub use plan::{
    artifact_from_plan_turn, format_mission, format_plan_for_display, harness_state_from_plan_turn,
    parse_plan, parse_plan_agent_step, plan_artifact_from_answer, PlanArtifact, PlanBrainMode,
    PlanDataContract, PlanLlmBrain,
    PlanParseError, PlanProgress, PlanPromptContext, PlanReadSource, PlanStepParseError,
    PlanWriteTarget, RulePlanBrain, Subtask, PLAN_REACT_SYSTEM_CORE, PLAN_SYSTEM_CORE,
    build_plan_layer_messages, format_plan_fixed_zone_system, format_plan_layer_prompt,
    format_plan_zone_after_preview, format_plan_zone_prompt_preview,
    format_planner_fixed_zone_html,
};
pub use protocol::{
    protocol_error_response, run_json_repl, ActionDto, ContextSummaryDto, ObservationDto,
    PlanDto, ProtocolError, RuntimeDto, SubtaskDto, SubtaskResultDto, TraceDto, TurnWireOptions,
    WireErrorBody, WireRequest, WireResponse, WIRE_VERSION,
};
pub use react::{
    run_repl, PlanPreviewResult, ReActConfig, ReActError, ReActLoop, SubtaskExecResult, TurnResult,
};
pub use turn_observer::{
    emit_llm_step, emit_observation_step, emit_phase_started, emit_plan_artifact, AgentStepDto,
    TurnObserver,
    TurnStepEvent,
};
pub use runtime::{OsFamily, RuntimeEnvironment, ShellKind};
pub use session::{PastTurn, SessionMemory};
pub use tasks::{
    apply_template, apply_template_value, audit_trace, ExecStep, MissionRenderContext,
    extract_reference_uid, StepAudit, SubtaskToolPolicy, TaskDefinition, TaskError,
    TaskExecutionAudit, TaskLoadError, TaskRegistry, ToolPolicySpec,
};
pub use tool::{
    apply_packs, default_packs, execute_action, format_tool_catalog, full_builtin_registry,
    packs_from_names, resolve_in_workspace, workspace_root, Tool, ToolContext, ToolPack,
    ToolRegistry, ToolRuntime, HELP_TEXT,
};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
