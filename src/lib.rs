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
pub mod context_metrics;
pub mod layer;
pub mod llm;
pub mod plan;
pub mod protocol;
pub mod react;
pub mod scout;
pub mod runtime;
pub mod session;
pub mod tasks;
pub mod tool;
pub mod tool_display;

pub use action::{Action, AgentStep, Observation, TurnTrace};
pub use advance::{
    format_recalled_progress, prepare_phase_recalled, restore_base_recalled, AdvanceConfig,
    AdvancePhaseNote, AdvancePhaseSummary, AdvanceProgress,
};
pub use brain::{AgentBrain, BrainMode, BrainPair, SimpleRuleBrain};
pub use brave_search::{BraveSearchConfig, BraveSearchError, WebSearchHit};
pub use config::{
    default_config_path, AppConfig, BraveSearchSection, ConfigError, LlmSection, LogSection,
    PromptSection, ReactSection, ToolsSection,
};
pub use context::{
    format_trace, ContextError, PromptBlocks, TurnPromptContext, REACT_SYSTEM_CORE,
    REACT_WEB_SEARCH_GUIDANCE,
};
pub use context_map::{
    analyze_messages, analyze_prompt_body, format_colormap, ContextSection, ContextSectionKind,
};
pub use context_log::{
    default_log_path, ContextLogEntry, ContextLogWriter, DEFAULT_CONTEXT_LOG_REL,
};
pub use context_metrics::{
    format_messages_body, ContextUsage, TextSize, TokenSource, TurnContextSummary,
};
pub use llm::{
    normalize_anthropic_base_url, normalize_gemini_base_url, normalize_lmstudio_base_url,
    normalize_ollama_base_url, parse_agent_step, AnthropicConnector, ChatMessage, ConnectorError,
    CompletionResult, GeminiConnector, LlmBrain, LlmConfig, LlmConnector, LlmConnectorKind,
    LlmProvider, LmStudioConnector, MockLlmConnector, OpenAiConnector, ParseError,
};
pub use layer::{run_layer_loop, run_plan_layer, LayerLoopOptions};
pub use plan::{
    artifact_from_plan_turn, format_mission, format_plan_for_display, parse_plan,
    parse_plan_agent_step,
    plan_artifact_from_answer, PlanArtifact, PlanBrainMode, PlanLlmBrain, PlanParseError,
    PlanProgress, PlanPromptContext, PlanStepParseError, RulePlanBrain, Subtask,
    PLAN_REACT_SYSTEM_CORE, PLAN_SYSTEM_CORE,
};
pub use protocol::{
    protocol_error_response, run_json_repl, ActionDto, ContextSummaryDto, ObservationDto,
    PlanDto, ProtocolError, RuntimeDto, SubtaskDto, SubtaskResultDto, TraceDto, TurnWireOptions,
    WireErrorBody, WireRequest, WireResponse, WIRE_VERSION,
};
pub use react::{
    run_repl, ReActConfig, ReActError, ReActLoop, SubtaskExecResult, TurnResult,
};
pub use scout::{
    apply_scout_recalled, artifact_from_scout_answer, format_scout_recalled,
    format_scout_user_input, is_trivial_scout_skip, ResearchArtifact, ScoutConfig,
    SCOUT_SYSTEM_APPEND,
};
pub use runtime::{OsFamily, RuntimeEnvironment, ShellKind};
pub use session::{PastTurn, SessionMemory};
pub use tasks::{
    apply_template, apply_template_value, audit_trace, ExecStep, MissionRenderContext,
    StepAudit, TaskDefinition, TaskError, TaskExecutionAudit, TaskLoadError, TaskRegistry,
};
pub use tool::{
    apply_packs, default_packs, execute_action, format_tool_catalog, full_builtin_registry,
    packs_from_names, resolve_in_workspace, workspace_root, Tool, ToolContext, ToolPack,
    ToolRegistry, ToolRuntime, HELP_TEXT,
};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
