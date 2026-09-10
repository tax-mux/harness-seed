//! ReAct ループの実行基盤。
mod advance_inject;
mod advance_turn;
mod hooks;
mod persist;
mod plan_turn;
mod repl;
mod skip;
mod step_driver;
mod synthesis;
mod two_phase;

use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::action::TurnTrace;
use crate::advance::{restore_base_recalled, AdvanceConfig, AdvanceMode, AdvancePhaseSummary};
use crate::brain::AgentBrain;
use crate::brave_search::BraveSearchConfig;
use crate::config::LogRotationConfig;
use crate::context::PromptBlocks;
use crate::context_log::default_log_path;
use crate::context_metrics::TurnContextSummary;
use crate::harness::{HarnessReference, HarnessState};
use crate::layer::{run_layer_loop, run_plan_layer, LayerLoopOptions};
use crate::lifecycle::{HostScratch, TurnLifecycle};
use crate::memory::{
    build_memory_rag, inject_memory_recalled, MemoryBridge, MemoryRag, MemoryRuntimeConfig,
    NoopBridge,
};
use crate::plan::{format_plan_for_display, PlanArtifact, PlanBrainMode, Subtask};
use crate::runtime::RuntimeEnvironment;
use crate::session::SessionMemory;
use crate::session::SessionPromptPolicy;
use crate::tasks::TaskRegistry;
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
    /// ライブラリ既定は `false`。CLI の JSON 省略時は `AppConfig::react_config` が `true`。
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
    /// 各ツールのコマンド・結果を stderr に表示する（既定 ON。ログがある場合は OFF 推奨）。
    pub show_tool_output: bool,
    /// Thought とツール一行要約を stderr に出す（既定 ON）。
    pub show_thinking: bool,
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
            show_thinking: false,
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
    MaxStepsExceeded {
        limit: usize,
    },
    Cancelled,
    PlanParseFailed {
        message: String,
    },
    /// サブタスク依存関係が不正（未知 id・閉路）。
    ScheduleFailed {
        message: String,
    },
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
    /// ライフサイクル用: 直近の計画（中断時の finished 通知に使う）。
    lifecycle_plan: Option<PlanArtifact>,
    /// `on_subtask_started` 済みで未 `finished` のサブタスク。
    lifecycle_open_subtasks: Vec<(Subtask, usize)>,
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
        blocks.web_search_enabled = tools.has_web_search();
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
            lifecycle_plan: None,
            lifecycle_open_subtasks: Vec::new(),
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
        self.lifecycle_plan = None;
        self.lifecycle_open_subtasks.clear();
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
    pub fn inject_reference_info(&mut self, refs: impl IntoIterator<Item = HarnessReference>) {
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
        self.blocks.web_search_enabled = self.tools.has_web_search();
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
        self.blocks.plan_task_catalog = Some(self.task_registry.catalog_for_planner_filtered(
            &available,
            self.blocks.web_search_enabled,
            &exclude,
            true,
        ));
    }

    /// このターンの read / write 契約を設定し、計画カタログを更新する。
    pub fn set_plan_data_contract(&mut self, contract: Option<crate::plan::PlanDataContract>) {
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

    fn run_plan_preview_inner(
        &mut self,
        user_input: &str,
    ) -> Result<PlanPreviewResult, ReActError> {
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
            self.config.show_thinking,
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

    pub fn run_turn(&mut self, user_input: &str) -> Result<TurnResult, ReActError> {
        self.begin_host_scratch_for_turn();
        self.emit_turn_started(user_input);
        let host_recalled = self.blocks.recalled.clone();
        self.inject_memory_for_turn(user_input);
        let result = if self.config.advance.mode.is_always() {
            self.run_turn_advance(user_input)
        } else if self.config.advance.mode == AdvanceMode::FromPlan || self.config.two_phase {
            self.run_turn_two_phase(user_input)
        } else {
            let _ = self.take_pending_reference_info_for_plan();
            self.run_turn_single(user_input, true, None, vec![])
        };
        restore_base_recalled(&mut self.blocks, &host_recalled);
        if let Err(ref err) = result {
            self.finalize_lifecycle_on_error(user_input, err);
        }
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
            self.config.show_thinking,
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
}

pub(super) fn append_trace(acc: &mut TurnTrace, step: &TurnTrace) {
    acc.thoughts.extend(step.thoughts.iter().cloned());
    acc.actions.extend(step.actions.iter().cloned());
    acc.observations.extend(step.observations.iter().cloned());
    acc.context_usages
        .extend(step.context_usages.iter().cloned());
}

pub use repl::run_repl;

#[cfg(test)]
mod tests;
