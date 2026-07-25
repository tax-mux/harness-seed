//! ReAct ループの実行基盤。
mod advance_turn;
mod skip;
mod synthesis;
mod two_phase;

use std::fmt;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::action::TurnTrace;
use crate::advance::{restore_base_recalled, AdvanceConfig, AdvancePhaseSummary};
use crate::brain::AgentBrain;
use crate::context::PromptBlocks;
use crate::config::LogRotationConfig;
use crate::context_log::{default_log_path, ContextLogWriter};
use crate::context_map::{
    aggregate_prompt_sections, analyze_prompt_body, format_colormap_titled,
};
use crate::context_metrics::TurnContextSummary;
use crate::harness::{HarnessReference, HarnessState};
use crate::layer::{run_layer_loop, run_plan_layer, LayerLoopOptions};
use crate::lifecycle::{invoke_lifecycle, HostScratch, HostView, TurnLifecycle, WriteScope};
use crate::memory::{
    build_memory_rag, inject_memory_recalled, DiaryEntry, DiaryPhase, MemoryBridge, MemoryRag,
    MemoryRuntimeConfig, NoopBridge,
};
use crate::session::SessionPromptPolicy;
use crate::plan::{
    format_plan_for_display, format_planner_fixed_zone_html, PlanArtifact, PlanBrainMode, Subtask,
};
use crate::runtime::RuntimeEnvironment;
use crate::session::SessionMemory;
use crate::tasks::TaskRegistry;
use crate::brave_search::BraveSearchConfig;
use crate::tool::{ToolPack, ToolRuntime};
use crate::turn_observer::{emit_plan_artifact, TurnObserver};

/// サブタスク監査失敗時の再実行上限（契約ありタスクのみ）。
pub(super) const SUBTASK_AUDIT_MAX_ATTEMPTS: usize = 2;

/// ReAct ループの設定。
#[derive(Debug, Clone)]
pub struct ReActConfig {
    /// 1ターンあたりの最大ステップ（無限ループ防止）。
    pub max_steps: usize,
    /// Thought / Action / Observation を stderr に出す。
    pub verbose: bool,
    /// ターン終了時にコンテキスト計測を stderr に出す。
    pub show_context_metrics: bool,
    /// コンテキスト計測を追記する JSON Lines ログ（`None` のみファイル出力なし）。
    pub context_log_path: Option<PathBuf>,
    /// コンテキストログのサイズローテーション（`log.rotation`）。
    pub log_rotation: LogRotationConfig,
    /// REPL 短期記憶に保持する直近ターン数。
    pub session_max_turns: usize,
    /// 計画フェーズ → 実行フェーズの直列オーケストレーション。
    pub two_phase: bool,
    /// 計画層 ReAct ループの最大ステップ。
    pub max_steps_plan: usize,
    /// 実行層 ReAct ループあたりの `thought` 上限。
    pub max_thoughts: usize,
    /// `react_only: false` かつ steps 契約があるサブタスクを LLM なしで順次実行する。
    /// 組み込みタスクは `react_only: true` のため、このフラグが true でも ReAct 経路になる。
    pub use_step_driver: bool,
    /// 計画フェーズで summary による候補選定→コンテキスト登録を行う。
    pub plan_candidate_selection: bool,
    /// 候補 summary カタログの最大エントリ数。
    pub plan_catalog_max_entries: usize,
    /// 候補 summary カタログの最大文字数。
    pub plan_catalog_max_chars: usize,
    /// ステップ契約の引数監査（`off` / `soft` / `hard`）。
    pub arg_audit_mode: crate::tasks::ArgAuditMode,
    /// 各 ReAct ステップの LLM プロンプト全文を stderr に出す。
    pub show_prompt: bool,
    /// 計画層の `PlanArtifact` を stdout に表示する（`two_phase` 時）。
    pub show_plan: bool,
    /// 各サブタスクの契約ツール／実行結果ツールを stdout に表示する。
    pub show_task_execution: bool,
    /// 各ツールのコマンド・結果を stderr に表示する（既定 ON）。
    pub show_tool_output: bool,
    /// 外側推進ループ（有効時は `two_phase` より優先）。
    pub advance: AdvanceConfig,
    /// 同一依存波内のサブタスクを並列実行する（`two_phase` 時）。
    /// ステップドライバ契約があるタスクはスレッド並列、ReAct タスクは波内で直列。
    pub parallel_subtasks: bool,
    /// ターンごとに `monitor/context_monitor.html` を更新する。
    pub monitor_plan_html: bool,
    /// 外部メモリ注入（`memory` セクション）。
    pub memory: MemoryRuntimeConfig,
}

impl Default for ReActConfig {
    fn default() -> Self {
        Self {
            max_steps: 16,
            verbose: false,
            show_context_metrics: true,
            context_log_path: Some(default_log_path()),
            log_rotation: LogRotationConfig {
                max_bytes: LogRotationConfig::DEFAULT_MAX_BYTES,
                max_files: LogRotationConfig::DEFAULT_MAX_FILES,
            },
            session_max_turns: SessionMemory::DEFAULT_MAX_TURNS,
            two_phase: false,
            max_steps_plan: 8,
            max_thoughts: 1,
            use_step_driver: true,
            plan_candidate_selection: true,
            plan_catalog_max_entries: 40,
            plan_catalog_max_chars: 8_000,
            arg_audit_mode: crate::tasks::ArgAuditMode::Soft,
            show_prompt: false,
            show_plan: true,
            show_task_execution: true,
            show_tool_output: true,
            advance: AdvanceConfig::default(),
            parallel_subtasks: false,
            monitor_plan_html: false,
            memory: MemoryRuntimeConfig::default(),
        }
    }
}

/// サブタスクごとの実行結果（two_phase 時）。
#[derive(Debug, Clone)]
pub struct SubtaskExecResult {
    pub id: u32,
    pub answer: String,
    pub steps_used: usize,
    /// ステップドライバ（`tasks/*.json` の `steps[]`）で実行した。
    pub used_step_driver: bool,
}

/// 回答合成に渡す evidence の 1 件あたり上限（文字数）。
/// 計画層のみ実行したプレビュー結果（`--plan-zone` 用）。
#[derive(Debug)]
pub struct PlanPreviewResult {
    /// Planner が返した作業指示書（生テキスト）。
    pub planner_text: String,
    pub harness: HarnessState,
    pub trace: TurnTrace,
    pub steps_used: usize,
}

/// 1回のターン実行結果。
#[derive(Debug)]
pub struct TurnResult {
    pub answer: String,
    pub trace: TurnTrace,
    pub steps_used: usize,
    pub context: TurnContextSummary,
    /// 計画フェーズの成果（two_phase 時のみ）。
    pub plan: Option<PlanArtifact>,
    /// Harness 内部状態（計画パース後。`two_phase` / `advance` 時）。
    pub harness: Option<HarnessState>,
    /// サブタスク実行の列（two_phase・複数サブタスク時）。
    pub subtask_results: Vec<SubtaskExecResult>,
    /// 推進ループで実行したフェーズのサマリ（`advance.enabled` 時）。
    pub advance_phases: Vec<AdvancePhaseSummary>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ReActError {
    MaxStepsExceeded { limit: usize },
    Cancelled,
    PlanParseFailed { message: String },
    /// サブタスク依存関係が不正（未知 id・閉路）。
    ScheduleFailed { message: String },
}

impl fmt::Display for ReActError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MaxStepsExceeded { limit } => {
                write!(f, "ReAct loop exceeded max steps ({limit})")
            }
            Self::Cancelled => write!(f, "ReAct loop cancelled"),
            Self::PlanParseFailed { message } => {
                write!(f, "plan parse failed: {message}")
            }
            Self::ScheduleFailed { message } => {
                write!(f, "subtask schedule failed: {message}")
            }
        }
    }
}

impl std::error::Error for ReActError {}

/// 実行層 + 計画層（ReAct 派生ループ）のオーケストレータ。
pub struct ReActLoop<E: AgentBrain> {
    exec_brain: E,
    plan_brain: PlanBrainMode,
    tools: ToolRuntime,
    config: ReActConfig,
    /// REPL セッションの短期記憶（完了ターンの user/answer）。
    pub session: SessionMemory,
    /// 外部から差し替え可能なプロンプトブロック（rules / recalled）。
    pub blocks: PromptBlocks,
    /// 機能塊タスク定義（`tasks/*.json`）。
    pub task_registry: TaskRegistry,
    /// 各 LLM ステップ・ツール観測の通知（GUI 向け）。
    pub turn_observer: Option<TurnObserver>,
    /// ターン／計画／サブタスクの副作用専用 hook（本筋ループは変更しない）。
    pub lifecycle: Option<Arc<dyn TurnLifecycle>>,
    /// ターン専用ホスト袋（LLM コンテキストに出さない）。
    host_scratch: HostScratch,
    /// 次の `run_turn` 開始時に `host_scratch` へマージする seed。
    pending_host_seed: Option<HostScratch>,
    stop_requested: Option<Arc<AtomicBool>>,
    /// 次の `run_turn` / `run_plan_preview` で Harness に載せる参照情報。
    pending_reference_info: Vec<HarnessReference>,
    /// 外部メモリ（既定 noop）。
    memory: Box<dyn MemoryBridge>,
    /// アダプタ手前の記憶 RAG（分岐・検索語）。
    memory_rag: MemoryRag,
    /// 並列ワーカー用にツールランタイムを fork するときのパック。
    tool_packs: Vec<ToolPack>,
    brave_search: Option<BraveSearchConfig>,
}

impl<E: AgentBrain> ReActLoop<E> {
    pub fn new(exec_brain: E, plan_brain: PlanBrainMode, config: ReActConfig) -> Self {
        Self::with_blocks(exec_brain, plan_brain, config, PromptBlocks::default())
    }

    pub fn with_blocks(
        exec_brain: E,
        plan_brain: PlanBrainMode,
        config: ReActConfig,
        blocks: PromptBlocks,
    ) -> Self {
        Self::with_blocks_and_tasks(
            exec_brain,
            plan_brain,
            config,
            blocks,
            TaskRegistry::load_default(),
            None,
            &crate::tool::default_packs(false),
            Box::new(NoopBridge),
        )
    }

    pub fn with_blocks_and_tasks(
        exec_brain: E,
        plan_brain: PlanBrainMode,
        config: ReActConfig,
        blocks: PromptBlocks,
        task_registry: TaskRegistry,
        brave_search: Option<BraveSearchConfig>,
        tool_packs: &[ToolPack],
        memory: Box<dyn MemoryBridge>,
    ) -> Self {
        let session = SessionMemory::new(config.session_max_turns);
        let runtime = RuntimeEnvironment::detect();
        let mut blocks = blocks;
        blocks.runtime = runtime.clone();
        let tools = ToolRuntime::with_packs(runtime.clone(), brave_search.clone(), tool_packs);
        blocks.tool_catalog = tools.catalog();
        blocks.web_search_enabled = tools.has_tool("web_search");
        let memory_rag = build_memory_rag(&config.memory, None);
        Self {
            exec_brain,
            plan_brain,
            tools,
            config,
            session,
            blocks,
            task_registry,
            turn_observer: None,
            lifecycle: None,
            host_scratch: HostScratch::new(),
            pending_host_seed: None,
            stop_requested: None,
            pending_reference_info: Vec::new(),
            memory,
            memory_rag,
            tool_packs: tool_packs.to_vec(),
            brave_search,
        }
    }

    /// ライフサイクル hook を登録する（Redmine 連携など。本筋には影響しない）。
    pub fn set_lifecycle(&mut self, lifecycle: Option<Arc<dyn TurnLifecycle>>) {
        self.lifecycle = lifecycle;
    }

    /// 次ターン開始時にホスト袋へ載せる seed（UI で選んだ ticket id など）。
    ///
    /// `run_turn` 先頭で袋をクリアしたあとマージされ、その後 `on_turn_started` が走る。
    pub fn seed_host_scratch(&mut self, seed: HostScratch) {
        self.pending_host_seed = Some(seed);
    }

    /// 直近ターンのホスト袋（読み取り）。次の `run_turn` 開始でクリアされる。
    pub fn host_scratch(&self) -> &HostScratch {
        &self.host_scratch
    }

    fn begin_host_scratch_for_turn(&mut self) {
        self.host_scratch.clear();
        if let Some(seed) = self.pending_host_seed.take() {
            self.host_scratch.merge_turn_seed(seed);
        }
    }

    /// メモリブリッジを差し替える（テスト・ホスト用）。
    pub fn set_memory_bridge(&mut self, memory: Box<dyn MemoryBridge>) {
        self.memory = memory;
    }

    /// 記憶 RAG を差し替える（LLM ルータ組み立て後など）。
    pub fn set_memory_rag(&mut self, memory_rag: MemoryRag) {
        self.memory_rag = memory_rag;
    }

    /// ターン開始前に参照情報を登録する（計画層の固定ゾーンと Harness JSON に反映）。
    pub fn inject_reference_info(
        &mut self,
        refs: impl IntoIterator<Item = HarnessReference>,
    ) {
        self.pending_reference_info.extend(refs);
    }

    pub fn with_defaults(exec_brain: E) -> Self {
        Self::new(exec_brain, PlanBrainMode::rule(), ReActConfig::default())
    }

    /// CLI の `-v` / `--verbose` を反映する。
    pub fn apply_cli_verbose(&mut self, verbose: bool) {
        self.config.verbose = verbose;
    }

    /// ホストアプリから in-process ツールを追加し、プロンプト用カタログを更新する。
    pub fn register_plugin(&mut self, tool: Box<dyn crate::tool::Tool>) {
        self.tools.register_plugin(tool);
        self.refresh_tool_catalog();
    }

    pub fn refresh_tool_catalog(&mut self) {
        self.blocks.tool_catalog = self.tools.catalog();
        self.blocks.web_search_enabled = self.tools.has_tool("web_search");
        self.refresh_plan_task_catalog();
    }

    /// この ReAct ループに登録済みの実行ツール名（計画層の task id フィルタ用）。
    pub fn registered_tool_names(&self) -> Vec<String> {
        self.tools.registry().names()
    }

    /// 登録済み実行ツールとデータ契約に合わせて計画層タスクカタログを更新する。
    pub fn refresh_plan_task_catalog(&mut self) {
        use std::collections::HashSet;
        let available: HashSet<String> = self.registered_tool_names().into_iter().collect();
        let exclude: Vec<&str> = self
            .blocks
            .plan_data_contract
            .as_ref()
            .map(|c| c.excluded_task_ids.iter().map(String::as_str).collect())
            .unwrap_or_default();
        if self.config.verbose {
            for (task_id, missing) in self.task_registry.tasks_missing_tools(&available) {
                eprintln!(
                    "[tasks] task '{task_id}' requires unavailable tools: {} (excluded from planner catalog when filtering)",
                    missing.join(", ")
                );
            }
        }
        self.blocks.plan_task_catalog = Some(
            self.task_registry.catalog_for_planner_filtered(
                &available,
                self.blocks.web_search_enabled,
                &exclude,
                true,
            ),
        );
    }

    /// このターンの read / write 契約を設定し、計画カタログを更新する。
    pub fn set_plan_data_contract(
        &mut self,
        contract: Option<crate::plan::PlanDataContract>,
    ) {
        self.blocks.plan_data_contract = contract;
        self.refresh_plan_task_catalog();
    }

    /// Planner 固定ゾーン（system）のみ。LLM は呼ばない。
    pub fn format_plan_fixed_zone(&self) -> String {
        crate::plan::format_plan_fixed_zone_system(&self.blocks, &self.task_registry)
    }

    /// 計画層 1 ステップ目のプロンプト全文。LLM は呼ばない。
    pub fn format_plan_layer_prompt(&self, user_input: &str) -> String {
        crate::plan::format_plan_layer_prompt(
            &self.blocks,
            user_input,
            &self.session,
            &self.task_registry,
        )
    }

    /// 保留中の参照を Planner 用 `recalled` へ載せ、ターン用ベクタを返す。
    fn take_pending_reference_info_for_plan(&mut self) -> Vec<HarnessReference> {
        let refs = std::mem::take(&mut self.pending_reference_info);
        if !refs.is_empty() {
            let text = HarnessState::format_references_for_prompt_from_slice(&refs);
            if !text.is_empty() {
                self.blocks.push_recalled(text);
            }
        }
        refs
    }

    fn merge_turn_reference_info(harness: &mut HarnessState, turn_refs: Vec<HarnessReference>) {
        if !turn_refs.is_empty() {
            harness.add_references(turn_refs);
        }
    }

    /// 計画層のみ実行（固定ゾーン → Planner → Harness パース）。実行層には進まない。
    pub fn run_plan_preview(&mut self, user_input: &str) -> Result<PlanPreviewResult, ReActError> {
        let host_recalled = self.blocks.recalled.clone();
        self.inject_memory_for_turn(user_input);
        let result = self.run_plan_preview_inner(user_input);
        restore_base_recalled(&mut self.blocks, &host_recalled);
        result
    }

    fn run_plan_preview_inner(&mut self, user_input: &str) -> Result<PlanPreviewResult, ReActError> {
        let turn_refs = self.take_pending_reference_info_for_plan();
        let (mut harness, trace, steps_used) = run_plan_layer(
            &mut self.plan_brain,
            &mut self.tools,
            &mut self.blocks,
            &self.session,
            user_input,
            self.config.max_steps_plan,
            self.config.verbose,
            self.config.show_prompt,
            false,
            false,
            self.turn_observer.as_ref(),
            self.stop_requested.as_deref(),
            Some(self.memory.as_ref()),
            self.config.memory.recall_max_rounds,
            &self.task_registry,
            self.config.plan_candidate_selection,
            self.config.plan_catalog_max_entries,
            self.config.plan_catalog_max_chars,
        )?;
        Self::merge_turn_reference_info(&mut harness, turn_refs);
        Ok(PlanPreviewResult {
            planner_text: harness.work_instructions.clone(),
            harness,
            trace,
            steps_used,
        })
    }

    fn resolve_plan_for_turn(&self, plan: &mut PlanArtifact, user_input: &str) {
        use std::collections::HashSet;
        let available: HashSet<String> = self.registered_tool_names().into_iter().collect();
        self.task_registry.resolve_plan_with_tools(
            plan,
            user_input,
            self.blocks.plan_data_contract.as_ref(),
            Some(&available),
        );
    }

    pub fn set_stop_requested(&mut self, stop_requested: Option<Arc<AtomicBool>>) {
        self.stop_requested = stop_requested;
    }

    fn is_stop_requested(&self) -> bool {
        self.stop_requested
            .as_ref()
            .map(|t| t.load(Ordering::Relaxed))
            .unwrap_or(false)
    }

    fn notify_plan_artifact(&self, plan: &PlanArtifact) {
        let display = format_plan_for_display(plan, &self.task_registry);
        emit_plan_artifact(self.turn_observer.as_ref(), "plan", plan, &display);
    }

    fn emit_turn_started(&mut self, user_input: &str) {
        let Some(h) = self.lifecycle.clone() else {
            return;
        };
        invoke_lifecycle("on_turn_started", || {
            h.on_turn_started(
                user_input,
                HostView::new(&mut self.host_scratch, WriteScope::Turn),
            );
        });
    }

    fn emit_plan_finished(&mut self, user_input: &str, plan: &PlanArtifact) {
        let Some(h) = self.lifecycle.clone() else {
            return;
        };
        invoke_lifecycle("on_plan_finished", || {
            h.on_plan_finished(
                user_input,
                plan,
                HostView::new(&mut self.host_scratch, WriteScope::Turn),
            );
        });
    }

    fn emit_subtask_started(
        &mut self,
        user_input: &str,
        plan: &PlanArtifact,
        subtask: &Subtask,
        index: usize,
    ) {
        let Some(h) = self.lifecycle.clone() else {
            return;
        };
        let id = subtask.id;
        invoke_lifecycle("on_subtask_started", || {
            h.on_subtask_started(
                user_input,
                plan,
                subtask,
                index,
                HostView::new(&mut self.host_scratch, WriteScope::Subtask(id)),
            );
        });
    }

    fn emit_subtask_finished(
        &mut self,
        user_input: &str,
        plan: &PlanArtifact,
        subtask: &Subtask,
        answer: &str,
        steps_used: usize,
    ) {
        let Some(h) = self.lifecycle.clone() else {
            return;
        };
        let id = subtask.id;
        invoke_lifecycle("on_subtask_finished", || {
            h.on_subtask_finished(
                user_input,
                plan,
                subtask,
                answer,
                steps_used,
                HostView::new(&mut self.host_scratch, WriteScope::Subtask(id)),
            );
        });
    }

    fn emit_turn_finished(&mut self, user_input: &str, result: &TurnResult) {
        let Some(h) = self.lifecycle.clone() else {
            return;
        };
        invoke_lifecycle("on_turn_finished", || {
            h.on_turn_finished(
                user_input,
                &result.answer,
                result.plan.as_ref(),
                result.steps_used,
                HostView::new(&mut self.host_scratch, WriteScope::Turn),
            );
        });
    }

    /// 計画フェーズの Harness パース結果をプロンプト固定ゾーンへ反映する。
    fn apply_harness_from_plan(&mut self, harness: &mut HarnessState, user_input: &str) {
        self.resolve_plan_for_turn(&mut harness.plan, user_input);
        self.blocks.work_instructions_text =
            Some(harness.format_work_instructions_for_prompt());
        if harness.total_steps > 0 {
            harness.begin_execution();
        }
        self.sync_harness_step_to_blocks(harness);
        if self.config.verbose {
            eprintln!("[harness] state:\n{}", harness.to_json_pretty());
        }
    }

    fn sync_harness_step_to_blocks(&mut self, harness: &HarnessState) {
        self.blocks.current_step_text = Some(
            harness.format_current_step_for_prompt(&self.task_registry),
        );
    }

    fn prepare_harness_for_subtask(&mut self, harness: &mut HarnessState, subtask: &Subtask) {
        harness.current_step = subtask.id;
        let policy = self.task_registry.tool_policy_for_subtask(subtask);
        harness.set_tool_set_from_policy(policy.as_ref());
        self.sync_harness_step_to_blocks(harness);
    }

    fn clear_harness_prompt_blocks(&mut self) {
        self.blocks.work_instructions_text = None;
        self.blocks.current_step_text = None;
    }

    pub fn run_turn(&mut self, user_input: &str) -> Result<TurnResult, ReActError> {
        self.begin_host_scratch_for_turn();
        self.emit_turn_started(user_input);
        let host_recalled = self.blocks.recalled.clone();
        self.inject_memory_for_turn(user_input);
        let result = if self.config.advance.enabled {
            self.run_turn_advance(user_input)
        } else if self.config.two_phase {
            self.run_turn_two_phase(user_input)
        } else {
            let _ = self.take_pending_reference_info_for_plan();
            self.run_turn_single(user_input, true, None, vec![])
        };
        restore_base_recalled(&mut self.blocks, &host_recalled);
        result
    }

    fn inject_memory_for_turn(&mut self, user_input: &str) {
        let prior = self.session.prior_one_liner();
        let route = inject_memory_recalled(
            &mut self.blocks,
            self.memory.as_ref(),
            &self.config.memory,
            &self.memory_rag,
            user_input,
            prior.as_deref(),
        );
        self.session.set_prompt_policy(if route.work_log {
            SessionPromptPolicy::IncludePrior
        } else {
            SessionPromptPolicy::OmitPrior
        });
        if self.config.verbose {
            eprintln!(
                "[memory.rag] work_log={} knowledge={} queries={:?}",
                route.work_log, route.knowledge, route.queries
            );
        }
    }

    fn run_turn_single(
        &mut self,
        user_input: &str,
        record_session: bool,
        plan: Option<PlanArtifact>,
        subtask_results: Vec<SubtaskExecResult>,
    ) -> Result<TurnResult, ReActError> {
        let result = run_layer_loop(
            &mut self.exec_brain,
            &mut self.tools,
            &mut self.blocks,
            &self.session,
            user_input,
            LayerLoopOptions::exec(self.config.max_steps, self.config.max_thoughts),
            self.config.verbose,
            self.config.show_prompt,
            self.config.show_tool_output,
            plan,
            subtask_results,
            self.turn_observer.as_ref(),
            self.stop_requested.as_deref(),
            None,
            0,
        )?;
        if record_session {
            self.finish_turn(user_input, &result);
        }
        Ok(result)
    }

    fn finish_turn(&mut self, user_input: &str, result: &TurnResult) {
        self.session
            .push_turn(user_input.to_string(), result.answer.clone());
        self.record_diary(user_input, result);
        if self.config.show_context_metrics && !result.context.is_empty() {
            eprintln!("[context turn] {}", result.context);
            let turn_sections = aggregate_prompt_sections(
                result
                    .trace
                    .context_usages
                    .iter()
                    .map(|u| u.prompt_body.as_str()),
            );
            if !turn_sections.is_empty() {
                let title = format!("turn prompts ({} calls)", result.context.llm_calls);
                eprintln!(
                    "[context turn map]\n{}",
                    format_colormap_titled(&turn_sections, true, &title)
                );
            }
            if let Some(last) = result.trace.context_usages.last() {
                let sections = analyze_prompt_body(&last.prompt_body);
                eprintln!(
                    "[context map]\n{}",
                    format_colormap_titled(&sections, true, "last prompt sections")
                );
            }
        }
        self.write_context_log(user_input, result);
        self.write_monitor_html(user_input, result);
        self.emit_turn_finished(user_input, result);
    }

    fn record_diary(&mut self, user_input: &str, result: &TurnResult) {
        let summary = result
            .plan
            .as_ref()
            .map(|p| p.summary.clone())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| {
                result
                    .answer
                    .chars()
                    .take(200)
                    .collect::<String>()
            });
        let phases = if result.advance_phases.is_empty() {
            result
                .subtask_results
                .iter()
                .map(|s| DiaryPhase {
                    id: s.id,
                    goal: format!("subtask {}", s.id),
                    answer: s.answer.clone(),
                })
                .collect()
        } else {
            result
                .advance_phases
                .iter()
                .map(|p| DiaryPhase {
                    id: p.id,
                    goal: p.goal.clone(),
                    answer: p.answer.clone(),
                })
                .collect()
        };
        let entry = DiaryEntry {
            user_input: user_input.to_string(),
            summary,
            answer: result.answer.clone(),
            phases,
        };
        // 実行完了後の最終回答（TurnResult.answer）を MemoryBridge 経由で書く（mempalace 直叩きはしない）。
        match self.memory.diary(&entry) {
            Ok(()) => {
                let preview: String = user_input.chars().take(40).collect();
                eprintln!("[memory] diary written: {preview}");
            }
            Err(err) => eprintln!("[memory] diary: {err}"),
        }
    }

    fn write_context_log(&self, user_input: &str, result: &TurnResult) {
        if result.context.is_empty() {
            return;
        }
        let Some(path) = &self.config.context_log_path else {
            return;
        };
        let writer = ContextLogWriter::new(path).with_rotation(self.config.log_rotation);
        match writer.append_turn(user_input, result) {
            Ok(()) => eprintln!("context log: appended to {}", path.display()),
            Err(err) => eprintln!("context log: failed to write {}: {err}", path.display()),
        }
    }

    fn write_monitor_html(&self, user_input: &str, result: &TurnResult) {
        if !self.config.monitor_plan_html {
            return;
        }

        let monitor_dir = PathBuf::from("monitor");
        if let Err(err) = fs::create_dir_all(&monitor_dir) {
            eprintln!("monitor html: failed to create {}: {err}", monitor_dir.display());
            return;
        }

        let planner_output = result
            .harness
            .as_ref()
            .map(|h| h.work_instructions.as_str());
        let recent_turns = self.session.format_for_prompt();
        let subtask_modes: Vec<(u32, bool)> = result
            .subtask_results
            .iter()
            .map(|s| (s.id, s.used_step_driver))
            .collect();
        let html = format_planner_fixed_zone_html(
            &self.blocks,
            &self.task_registry,
            result.harness.as_ref(),
            planner_output,
            Some(user_input),
            Some(&result.context),
            Some(&result.trace),
            &self.blocks.recalled,
            if recent_turns.trim().is_empty() {
                None
            } else {
                Some(recent_turns.as_str())
            },
            &subtask_modes,
        );
        let path = monitor_dir.join("context_monitor.html");
        match fs::write(&path, html) {
            Ok(()) => {
                if self.config.verbose {
                    eprintln!("monitor html: wrote {}", path.display());
                }
            }
            Err(err) => eprintln!("monitor html: failed to write {}: {err}", path.display()),
        }
    }
}

pub(super) fn append_trace(acc: &mut TurnTrace, step: &TurnTrace) {
    acc.thoughts.extend(step.thoughts.iter().cloned());
    acc.actions.extend(step.actions.iter().cloned());
    acc.observations.extend(step.observations.iter().cloned());
    acc.context_usages.extend(step.context_usages.iter().cloned());
}

/// 対話 REPL（stdin → ReAct → stdout）。
pub fn run_repl<E: AgentBrain>(
    loop_engine: &mut ReActLoop<E>,
    verbose: bool,
) -> io::Result<()> {
    loop_engine.apply_cli_verbose(verbose);

    let stdin = io::stdin();
    let mut line = String::new();

    println!(
        "HarnessSeed ReAct REPL — 'help' でコマンド一覧、'clear' で短期記憶リセット、'quit' で終了"
    );

    loop {
        line.clear();
        print!("> ");
        io::Write::flush(&mut io::stdout())?;

        if stdin.read_line(&mut line)? == 0 {
            println!();
            break;
        }

        let input = line.trim();
        if input.is_empty() {
            continue;
        }
        if matches!(input, "quit" | "exit" | "q") {
            break;
        }
        if matches!(input, "clear" | "forget" | "reset") {
            loop_engine.session.clear();
            println!("session memory cleared");
            continue;
        }

        match loop_engine.run_turn(input) {
            Ok(result) => {
                if verbose {
                    eprintln!("--- trace ---\n{}", result.trace);
                }
                println!("{}", result.answer);
            }
            Err(err) => eprintln!("error: {err}"),
        }
    }

    Ok(())
}

/// ステップドライバの生 answer がそのままユーザー向けに十分そうなら true。
mod tests {
    use super::*;
    use crate::brain::SimpleRuleBrain;
    use crate::context::TurnPromptContext;

    #[test]
    fn help_turn_single_step() {
        let mut react = ReActLoop::with_defaults(SimpleRuleBrain::new());
        let result = react.run_turn("help").unwrap();
        assert_eq!(result.steps_used, 1);
        assert!(result.answer.contains("echo"));
    }

    #[test]
    fn generic_input_runs_thought_echo_answer() {
        let mut react = ReActLoop::with_defaults(SimpleRuleBrain::new());
        let result = react.run_turn("hello world").unwrap();
        assert_eq!(result.steps_used, 3);
        assert_eq!(result.trace.thoughts.len(), 1);
        assert_eq!(result.trace.actions.len(), 1);
        assert!(result.answer.contains("hello world"));
    }

    #[test]
    fn echo_command_skips_thought() {
        let mut react = ReActLoop::with_defaults(SimpleRuleBrain::new());
        let result = react.run_turn("echo ping").unwrap();
        assert_eq!(result.steps_used, 2);
        assert!(result.trace.thoughts.is_empty());
        assert!(result.answer.contains("ping"));
    }

    #[test]
    fn blocks_recalled_visible_in_llm_system_when_rendered() {
        let mut blocks = PromptBlocks::new();
        blocks.push_recalled("note from host");
        let trace = TurnTrace::default();
        let session = SessionMemory::default();
        let ctx = TurnPromptContext::new(&blocks, "hi", &trace, &session);
        let system = ctx
            .render()
            .into_iter()
            .find(|m| m.role == "system")
            .expect("system");
        assert!(system.content.as_text().contains("note from host"));
    }

    #[test]
    fn session_accumulates_completed_turns() {
        let mut react = ReActLoop::with_defaults(SimpleRuleBrain::new());
        react.run_turn("help").unwrap();
        react.run_turn("help").unwrap();
        assert_eq!(react.session.len(), 2);
        react.session.set_prompt_policy(SessionPromptPolicy::IncludePrior);
        assert!(react.session.format_for_prompt().contains("利用可能"));
    }

    #[test]
    fn two_phase_help_still_single_exec() {
        let mut config = ReActConfig::default();
        config.two_phase = true;
        let mut react = ReActLoop::new(SimpleRuleBrain::new(), PlanBrainMode::rule(), config);
        let result = react.run_turn("help").unwrap();
        // 計画層 1 + 実行層 1（ルール頭脳は context_usages なし）
        assert_eq!(result.steps_used, 2);
        assert!(result.plan.as_ref().unwrap().skip_execution);
        assert!(result.answer.contains("echo"));
    }

    #[test]
    fn two_phase_generic_runs_subtask_mission() {
        let mut config = ReActConfig::default();
        config.two_phase = true;
        let mut react = ReActLoop::new(SimpleRuleBrain::new(), PlanBrainMode::rule(), config);
        let result = react.run_turn("hello world").unwrap();
        assert_eq!(result.subtask_results.len(), 1);
        assert_eq!(result.subtask_results[0].id, 1);
        assert_eq!(result.steps_used, 5);
        assert!(!result.subtask_results[0].used_step_driver);
        assert!(result.answer.contains("hello world"));
    }

    #[test]
    fn lifecycle_panic_does_not_abort_turn() {
        use crate::lifecycle::{HostView, TurnLifecycle};

        struct Boom;
        impl TurnLifecycle for Boom {
            fn on_plan_finished(&self, _: &str, _: &PlanArtifact, _: HostView<'_>) {
                panic!("host hook exploded");
            }
        }

        let mut config = ReActConfig::default();
        config.two_phase = true;
        let mut react = ReActLoop::new(SimpleRuleBrain::new(), PlanBrainMode::rule(), config);
        react.set_lifecycle(Some(Arc::new(Boom)));
        let result = react.run_turn("hello world").unwrap();
        assert!(result.answer.contains("hello world"));
    }

    #[test]
    fn lifecycle_hooks_fire_without_changing_answer() {
        use crate::lifecycle::{HostScratch, HostView, TurnLifecycle};
        use std::sync::Mutex;

        #[derive(Default)]
        struct Rec {
            events: Mutex<Vec<String>>,
        }
        impl TurnLifecycle for Rec {
            fn on_turn_started(&self, _: &str, host: HostView<'_>) {
                let ticket = host.turn_get_i64("ticket_id").unwrap_or(-1);
                self.events
                    .lock()
                    .unwrap()
                    .push(format!("turn_started:{ticket}"));
            }
            fn on_plan_finished(&self, _: &str, _: &PlanArtifact, mut host: HostView<'_>) {
                host.insert("parent_ticket", 42);
                self.events.lock().unwrap().push("plan_finished".into());
            }
            fn on_subtask_started(
                &self,
                _: &str,
                _: &PlanArtifact,
                subtask: &Subtask,
                _: usize,
                mut host: HostView<'_>,
            ) {
                let parent = host.turn_get_i64("parent_ticket").unwrap_or(-1);
                host.insert("child_ticket", 7);
                self.events.lock().unwrap().push(format!(
                    "subtask_started:{}:{parent}",
                    subtask.id
                ));
            }
            fn on_subtask_finished(
                &self,
                _: &str,
                _: &PlanArtifact,
                subtask: &Subtask,
                _: &str,
                _: usize,
                host: HostView<'_>,
            ) {
                let child = host.get_i64("child_ticket").unwrap_or(-1);
                self.events.lock().unwrap().push(format!(
                    "subtask_finished:{}:{child}",
                    subtask.id
                ));
            }
            fn on_turn_finished(
                &self,
                _: &str,
                _: &str,
                _: Option<&PlanArtifact>,
                _: usize,
                host: HostView<'_>,
            ) {
                let parent = host.turn_get_i64("parent_ticket").unwrap_or(-1);
                let child = host.subtask_get_i64(1, "child_ticket").unwrap_or(-1);
                self.events
                    .lock()
                    .unwrap()
                    .push(format!("turn_finished:{parent}:{child}"));
            }
        }

        let rec = Arc::new(Rec::default());
        let mut config = ReActConfig::default();
        config.two_phase = true;
        let mut react = ReActLoop::new(SimpleRuleBrain::new(), PlanBrainMode::rule(), config);
        react.set_lifecycle(Some(rec.clone()));
        let mut seed = HostScratch::new();
        seed.turn_insert("ticket_id", 10);
        react.seed_host_scratch(seed);
        let result = react.run_turn("hello world").unwrap();
        assert!(result.answer.contains("hello world"));
        assert_eq!(
            rec.events.lock().unwrap().as_slice(),
            [
                "turn_started:10",
                "plan_finished",
                "subtask_started:1:42",
                "subtask_finished:1:7",
                "turn_finished:42:7",
            ]
        );
        assert_eq!(react.host_scratch().turn_get_i64("ticket_id"), Some(10));
        assert_eq!(react.host_scratch().turn_get_i64("parent_ticket"), Some(42));
        assert_eq!(
            react.host_scratch().subtask_get_i64(1, "child_ticket"),
            Some(7)
        );
        let json = react.host_scratch().to_value();
        assert_eq!(json["turn"]["parent_ticket"], 42);
        assert_eq!(json["subtasks"]["1"]["child_ticket"], 7);
    }

    #[test]
    fn advance_enabled_runs_single_phase_with_rule_brain() {
        let mut config = ReActConfig::default();
        config.advance.enabled = true;
        config.advance.show_phases = false;
        config.show_plan = false;
        config.show_task_execution = false;
        let mut react = ReActLoop::new(SimpleRuleBrain::new(), PlanBrainMode::rule(), config);
        let result = react.run_turn("hello world").unwrap();
        assert_eq!(result.advance_phases.len(), 1);
        assert_eq!(result.advance_phases[0].id, 1);
        assert!(result.answer.contains("hello world"));
    }

    #[test]
    fn plan_preview_runs_plan_layer_only() {
        let mut react = ReActLoop::new(SimpleRuleBrain::new(), PlanBrainMode::rule(), ReActConfig::default());
        let preview = react.run_plan_preview("hello world").unwrap();
        assert!(!preview.planner_text.is_empty());
        assert_eq!(preview.harness.plan.subtasks.len(), 1);
        assert!(preview.steps_used >= 1);
    }

    #[test]
    fn local_memory_survives_across_turns_without_panic() {
        use crate::memory::LocalDiaryBridge;
        let mut config = ReActConfig::default();
        config.advance.enabled = true;
        config.advance.show_phases = false;
        config.show_plan = false;
        config.show_task_execution = false;
        let mut react = ReActLoop::new(SimpleRuleBrain::new(), PlanBrainMode::rule(), config);
        react.set_memory_bridge(Box::new(LocalDiaryBridge::new()));
        react.run_turn("echo first-unique-token").unwrap();
        let second = react.run_turn("続きやって").unwrap();
        assert!(!second.answer.is_empty());
        // host recalled はターン終了後に復元される
        assert!(react.blocks.recalled.is_empty());
    }

    #[test]
    fn answer_looks_user_ready_accepts_plain_sentence() {
        assert!(synthesis::answer_looks_user_ready("実装可能です。"));
        assert!(synthesis::answer_looks_user_ready("  hello world  "));
    }

    #[test]
    fn answer_looks_user_ready_rejects_structured_or_multiline() {
        assert!(!synthesis::answer_looks_user_ready(""));
        assert!(!synthesis::answer_looks_user_ready("line1\nline2"));
        assert!(!synthesis::answer_looks_user_ready(r#"{"step":"answer"}"#));
        assert!(!synthesis::answer_looks_user_ready("a\tb"));
        assert!(!synthesis::answer_looks_user_ready("[a, b]"));
    }

    #[test]
    fn needs_user_answer_synthesis_skips_when_driver_answer_is_ready() {
        let results = vec![SubtaskExecResult {
            id: 1,
            answer: "一覧を取得しました。".into(),
            steps_used: 1,
            used_step_driver: true,
        }];
        assert!(!ReActLoop::<SimpleRuleBrain>::needs_user_answer_synthesis(&results));
    }

    #[test]
    fn needs_user_answer_synthesis_when_driver_output_is_raw() {
        let results = vec![SubtaskExecResult {
            id: 1,
            answer: "README.md\nCargo.toml\nsrc/".into(),
            steps_used: 1,
            used_step_driver: true,
        }];
        assert!(ReActLoop::<SimpleRuleBrain>::needs_user_answer_synthesis(&results));
    }

    #[test]
    fn build_synthesis_evidence_caps_total_chars() {
        let results = vec![
            SubtaskExecResult {
                id: 1,
                answer: "a".repeat(500),
                steps_used: 1,
                used_step_driver: true,
            },
            SubtaskExecResult {
                id: 2,
                answer: "b".repeat(500),
                steps_used: 1,
                used_step_driver: true,
            },
        ];
        let evidence = synthesis::build_synthesis_evidence(&results, &[], 600, 400);
        assert!(evidence.chars().count() <= 401);
    }
}
