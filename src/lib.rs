//! HarnessSeed (`harness-seed`) — 組み込み用 ReAct ループの実行基盤。

pub mod action;
pub mod advance;
pub mod agent_assets;
pub mod brain;
pub mod brave_search;
pub mod cli_agent;
pub mod config;
pub mod context;
pub mod context_log;
pub mod context_manifest;
pub mod context_map;
pub mod context_metrics;
pub mod grep;
pub mod harness;
pub mod io_utf8;
pub mod layer;
pub mod lifecycle;
pub mod line_io;
pub mod llm;
pub mod memory;
pub mod mcp;
pub mod monitor_view;
pub mod plan;
pub mod protocol;
pub mod react;
pub mod runtime;
pub mod seed;
pub mod session;
pub mod tasks;
pub mod text_match;
pub mod tool;
pub mod tool_display;
pub mod turn_observer;

pub use action::{Action, AgentStep, Observation, TurnTrace};
pub use advance::{
    apply_absence_gate, apply_citation_gate, build_phase_note, claim_audit_rules,
    claim_falsification_retry_subtask, claim_falsification_subtask, classify_absence_claims,
    count_ok_tool_observations, count_substantive_ok_observations, evidence_deepening_subtask,
    evidence_grounding_rules, evidence_paths_from_notes, evidence_paths_from_texts,
    extract_absence_claims, format_recalled_progress, is_substantive_evidence_tool,
    looks_like_absence_claim, prepare_phase_recalled, prior_evidence_is_thin,
    prior_has_auditable_claims, restore_base_recalled, unverified_cited_paths, AbsenceClaimVerdict,
    AdvanceConfig, AdvanceMode, AdvancePhaseNote, AdvancePhaseSummary, AdvanceProgress,
    MIN_OK_TOOL_OBSERVATIONS_BEFORE_JUDGMENT, MIN_SUBSTANTIVE_OK_OBSERVATIONS_BEFORE_JUDGMENT,
    SUBSTANTIVE_EVIDENCE_TOOLS,
};
pub use agent_assets::{
    apply_agent_project, load_agent_project_file, resolve_cli_agent_config, AgentConfigError,
    AgentLoadError, AgentLoadReport, AgentProjectConfig, CliAgentSource, ScriptTool,
    ScriptToolDefinition, DEFAULT_FILENAME,
};
pub use brain::{AgentBrain, BrainMode, BrainPair, SimpleRuleBrain};
pub use brave_search::{BraveSearchConfig, BraveSearchError, WebSearchHit};
pub use config::{
    default_config_path, resolve_user_config_rel, user_config_dir, user_config_path, AppConfig,
    BraveSearchSection, ConfigError, LlmSection, LogRotationConfig, LogRotationSection, LogSection,
    MemoryRecentWorkSection, MemorySearchSection, MemorySection, MempalaceSection, McpToolsSection,
    PromptSection, ReactSection, ToolsSection,
};
pub use context::{
    format_plan_rule_prompt_preview, format_trace, ContextError, PromptBlocks, TurnPromptContext,
    REACT_SYSTEM_CORE, REACT_WEB_SEARCH_GUIDANCE,
};
pub use context_log::{
    default_log_path, format_step_console_lines, format_turn_console_summary, infer_phase,
    infer_step_meta, iso8601_timestamp_now, preview_text, rotate_log_file, ContextLogEntry,
    ContextLogWriter, CONTEXT_LOG_SCHEMA_VERSION, DEFAULT_CONTEXT_LOG_REL, PREVIEW_CHARS,
};
pub use context_manifest::{
    apply_scoped_entry, format_apply_error_hint, note_manifest_available, ContextManifestError,
    VisionAttachment, SCOPED_RECALL_PREFIX,
};
pub use context_map::{
    aggregate_prompt_sections, analyze_messages, analyze_prompt_body, format_colormap,
    format_colormap_titled, ContextSection, ContextSectionKind,
};
pub use context_metrics::{
    format_messages_body, ContextUsage, TextSize, TokenSource, TurnContextSummary,
};
pub use io_utf8::{apply_utf8_child_env, ensure_utf8_stdio, Utf8StreamDecoder};
pub use harness::{
    format_references_for_prompt, parse_harness, HarnessMailRefKind, HarnessParseError,
    HarnessReference, HarnessState, HarnessStatus,
};
pub use layer::{run_layer_loop, run_plan_layer, LayerLoopOptions};
pub use lifecycle::{
    invoke_lifecycle, lifecycle_from_tracking, CompositeLifecycle, HostScratch, HostView,
    NoopLifecycle, RunStatus, SubtaskOutcome, TaskTracking, TaskTrackingLifecycle,
    TurnFinishedEvent, TurnLifecycle, TurnOutcome, WorkFinishedEvent, WorkStartedEvent, WriteScope,
};
pub use llm::{
    coerce_tool_named_step_json, format_error_chain, normalize_anthropic_base_url,
    normalize_gemini_base_url, normalize_lmstudio_base_url, normalize_ollama_base_url,
    parse_agent_step, require_absolute_http_base, AnthropicConnector, ChatMessage,
    CompletionResult, ConnectorError, GeminiConnector, LlmBrain, LlmConfig, LlmConnector,
    LlmConnectorKind, LlmProvider, LmStudioConnector, MockLlmConnector, OpenAiConnector,
    ParseError,
};
#[cfg(feature = "mempalace")]
pub use memory::MempalaceBridge;
pub use memory::{
    build_memory_bridge, build_memory_rag, diary_from_plan, format_recalled_block,
    inject_memory_recalled, provider_options, recall_knowledge, resolve_memory_layers, DiaryEntry,
    DiaryPhase, LayeredMemoryBridge, LocalDiaryBridge, MemoryBridge, MemoryError, MemoryLayerPlan,
    MemoryRag, MemoryRoute, MemoryRouter, MemoryRuntimeConfig, NoopBridge, RecalledItem,
    RecalledSource, RuleRouter, PROVIDER_LOCAL, PROVIDER_MEMPALACE, PROVIDER_NOOP,
};
pub use mcp::load_mcp_tools;
pub use monitor_view::{
    format_events_monitor_html, load_recent_event_lines, render_monitor_html_from_log,
};
pub use plan::{
    artifact_from_plan_turn, build_plan_layer_messages, execution_waves, format_mission,
    format_plan_fixed_zone_system, format_plan_for_display, format_plan_layer_prompt,
    format_plan_zone_after_preview, format_plan_zone_prompt_preview,
    format_planner_fixed_zone_html, harness_state_from_plan_turn, is_replan_subtask,
    is_reserved_control_task, is_weak_done_when, normalize_candidates, parse_candidate_selection,
    parse_plan, parse_plan_agent_step, plan_artifact_from_answer,
    select_and_register_plan_candidates, select_and_register_plan_candidates_with_budget,
    strengthen_weak_done_when, PlanArtifact, PlanBrainMode, PlanDataContract, PlanEnforceFn,
    PlanLlmBrain, PlanParseError, PlanProgress, PlanPromptContext, PlanQueue, PlanQueueError,
    PlanStepParseError, RulePlanBrain, ScheduleError, Subtask, CANDIDATE_SELECTION_SYSTEM,
    EVIDENCE_ORIENTED_DONE_WHEN, PLAN_CATALOG_SUMMARY_MAX_CHARS, PLAN_CATALOG_SUMMARY_MAX_ENTRIES,
    PLAN_REACT_SYSTEM_CORE, PLAN_SYSTEM_CORE, REPLAN_TASK_ID,
};
pub use protocol::{
    protocol_error_response, run_json_repl, ActionDto, ContextSummaryDto, ObservationDto, PlanDto,
    ProtocolError, RuntimeDto, SubtaskDto, SubtaskResultDto, TraceDto, TurnWireOptions,
    WireErrorBody, WireRequest, WireResponse, WIRE_VERSION,
};
pub use react::{
    run_repl, PlanPreviewResult, ReActConfig, ReActError, ReActLoop, SubtaskExecResult, TurnResult,
};
pub use runtime::{OsFamily, RuntimeEnvironment, ShellKind};
pub use seed::SeedBuilder;
pub use session::{PastTurn, SessionMemory, SessionPromptPolicy};
pub use tasks::{
    apply_template, apply_template_value, args_satisfy_contract, audit_trace,
    audit_trace_with_mode, expected_args, ArgAuditMode, ContextManifestSpec, ExecStep,
    MissionRenderContext, StepAudit, SubtaskToolPolicy, TaskDefinition, TaskError,
    TaskExecutionAudit, TaskLoadError, TaskRegistry, ToolPolicySpec,
};
pub use tool::{
    apply_packs, default_packs, execute_action, format_tool_catalog, full_builtin_registry,
    packs_from_names, resolve_in_workspace, workspace_root, Tool, ToolContext, ToolPack,
    ToolRegistry, ToolRuntime, HELP_TEXT,
};
pub use turn_observer::{
    emit_candidates, emit_llm_step, emit_observation_step, emit_phase_started, emit_plan_artifact,
    AgentStepDto, TurnObserver, TurnStepEvent,
};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
