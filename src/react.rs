use std::fmt;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::action::TurnTrace;
use crate::advance::{
    prepare_phase_recalled, restore_base_recalled, AdvanceConfig, AdvancePhaseSummary,
    AdvanceProgress,
};
use crate::brain::AgentBrain;
use crate::context::PromptBlocks;
use crate::config::LogRotationConfig;
use crate::context_log::{default_log_path, ContextLogWriter};
use crate::context_map::{analyze_prompt_body, format_colormap};
use crate::context_metrics::TurnContextSummary;
use crate::harness::{HarnessReference, HarnessState};
use crate::layer::{run_layer_loop, run_plan_layer, LayerLoopOptions};
use crate::plan::{
    format_mission, format_plan_for_display, format_planner_fixed_zone_html, PlanArtifact,
    PlanBrainMode, PlanProgress, Subtask,
};
use crate::runtime::RuntimeEnvironment;
use crate::session::SessionMemory;
use crate::tasks::TaskRegistry;
use crate::brave_search::BraveSearchConfig;
use crate::tool::{ToolPack, ToolRuntime};
use crate::turn_observer::{emit_plan_artifact, TurnObserver};

/// サブタスク監査失敗時の再実行上限（契約ありタスクのみ）。
const SUBTASK_AUDIT_MAX_ATTEMPTS: usize = 2;

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
    /// `tasks/*.json` の `steps[]` 契約があるサブタスクを LLM なしで順次実行する。
    pub use_step_driver: bool,
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
    /// ターンごとに `monitor/context_monitor.html` を更新する。
    pub monitor_plan_html: bool,
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
            max_steps_plan: 4,
            use_step_driver: true,
            show_prompt: false,
            show_plan: true,
            show_task_execution: true,
            show_tool_output: true,
            advance: AdvanceConfig::default(),
            monitor_plan_html: false,
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
    stop_requested: Option<Arc<AtomicBool>>,
    /// 次の `run_turn` / `run_plan_preview` で Harness に載せる参照情報。
    pending_reference_info: Vec<HarnessReference>,
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
    ) -> Self {
        let session = SessionMemory::new(config.session_max_turns);
        let runtime = RuntimeEnvironment::detect();
        let mut blocks = blocks;
        blocks.runtime = runtime.clone();
        let tools = ToolRuntime::with_packs(runtime.clone(), brave_search.clone(), tool_packs);
        blocks.tool_catalog = tools.catalog();
        blocks.web_search_enabled = tools.has_tool("web_search");
        Self {
            exec_brain,
            plan_brain,
            tools,
            config,
            session,
            blocks,
            task_registry,
            turn_observer: None,
            stop_requested: None,
            pending_reference_info: Vec::new(),
        }
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
            .map(|c| c.excluded_task_ids())
            .unwrap_or_default();
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
        let turn_refs = self.take_pending_reference_info_for_plan();
        let (mut harness, trace, steps_used) = run_plan_layer(
            &mut self.plan_brain,
            &mut self.tools,
            &self.blocks,
            &self.session,
            user_input,
            self.config.max_steps_plan,
            self.config.verbose,
            self.config.show_prompt,
            false,
            false,
            self.turn_observer.as_ref(),
            self.stop_requested.as_deref(),
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
        self.task_registry.resolve_plan(
            plan,
            user_input,
            self.blocks.plan_data_contract.as_ref(),
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
        if self.config.advance.enabled {
            self.run_turn_advance(user_input)
        } else if self.config.two_phase {
            self.run_turn_two_phase(user_input)
        } else {
            let _ = self.take_pending_reference_info_for_plan();
            self.run_turn_single(user_input, true, None, vec![])
        }
    }

    /// 計画層 → フェーズ逐次実行。各フェーズ前に `recalled` へ進捗を載せ、必要なら session をクリア。
    fn run_turn_advance(&mut self, user_input: &str) -> Result<TurnResult, ReActError> {
        let advance = self.config.advance.clone();
        let base_recalled = self.blocks.recalled.clone();

        if self.config.verbose {
            eprintln!("[advance] planning for: {user_input}");
        }
        let turn_refs = self.take_pending_reference_info_for_plan();
        let (mut harness, plan_trace, plan_steps) = run_plan_layer(
            &mut self.plan_brain,
            &mut self.tools,
            &self.blocks,
            &self.session,
            user_input,
            self.config.max_steps_plan,
            self.config.verbose,
            self.config.show_prompt,
            self.config.show_tool_output,
            true,
            self.turn_observer.as_ref(),
            self.stop_requested.as_deref(),
        )?;
        Self::merge_turn_reference_info(&mut harness, turn_refs);
        self.apply_harness_from_plan(&mut harness, user_input);
        let plan = harness.plan.clone();
        self.notify_plan_artifact(&plan);
        if self.config.show_plan {
            println!("{}", format_plan_for_display(&plan, &self.task_registry));
        }

        if !plan.needs_execution() {
            restore_base_recalled(&mut self.blocks, &base_recalled);
            let mut result =
                self.run_turn_single(user_input, true, Some(plan), vec![])?;
            append_trace(&mut result.trace, &plan_trace);
            result.context = TurnContextSummary::from_usages(&result.trace.context_usages);
            result.steps_used += plan_steps;
            result.harness = Some(harness);
            result.advance_phases.clear();
            self.clear_harness_prompt_blocks();
            return Ok(result);
        }

        let phase_limit = advance.max_phases.min(plan.subtasks.len());
        let mut advance_progress = AdvanceProgress::new(user_input, plan.summary.clone());
        let mut plan_progress = PlanProgress::default();
        let mut subtask_results = Vec::new();
        let mut advance_phases = Vec::new();
        let mut combined_trace = plan_trace;
        let mut total_steps = plan_steps;
        let mut final_answer = String::new();

        for (phase_index, subtask) in plan.subtasks.iter().take(phase_limit).enumerate() {
            if self.is_stop_requested() {
                return Err(ReActError::Cancelled);
            }
            // Keep previous-turn memory for the first phase of a turn.
            // When enabled, clear only between phases in the same turn.
            if advance.clear_session_each_phase && phase_index > 0 {
                self.session.clear();
            }
            prepare_phase_recalled(
                &mut self.blocks,
                &base_recalled,
                &advance_progress,
                &plan,
                subtask,
                &advance,
            );

            if advance.show_phases {
                println!("--- Advance phase {} / {phase_limit} ---", subtask.id);
                println!("  goal: {}", subtask.goal);
            }
            if self.config.verbose {
                eprintln!("[advance] phase {}: {}", subtask.id, subtask.goal);
            }
            if self.config.show_task_execution {
                println!("--- Exec subtask {} ---", subtask.id);
                println!(
                    "{}",
                    self.task_registry
                        .format_subtask_execution_for_display(subtask)
                );
            }

            self.prepare_harness_for_subtask(&mut harness, subtask);
            let (exec, used_driver) =
                self.run_subtask_exec_audited(user_input, &plan, subtask, &plan_progress)?;
            harness.advance_after_subtask(subtask.id);
            self.sync_harness_step_to_blocks(&harness);

            if self.config.show_task_execution {
                let mode = if used_driver { "step-driver" } else { "ReAct" };
                println!(
                    "  completed via {mode}: {}",
                    TaskRegistry::format_trace_tools_used(&exec.trace)
                );
            }

            advance_progress.push(subtask.id, subtask.goal.clone(), exec.answer.clone());
            plan_progress.push(subtask.id, exec.answer.clone());
            subtask_results.push(SubtaskExecResult {
                id: subtask.id,
                answer: exec.answer.clone(),
                steps_used: exec.steps_used,
                used_step_driver: used_driver,
            });
            advance_phases.push(AdvancePhaseSummary {
                id: subtask.id,
                goal: subtask.goal.clone(),
                answer: exec.answer.clone(),
                steps_used: exec.steps_used,
            });
            total_steps += exec.steps_used;
            final_answer = exec.answer;
            append_trace(&mut combined_trace, &exec.trace);
        }

        restore_base_recalled(&mut self.blocks, &base_recalled);
        self.clear_harness_prompt_blocks();

        let result = TurnResult {
            answer: final_answer,
            context: TurnContextSummary::from_usages(&combined_trace.context_usages),
            trace: combined_trace,
            steps_used: total_steps,
            plan: Some(plan),
            harness: Some(harness),
            subtask_results,
            advance_phases,
        };
        self.finish_turn(user_input, &result);
        Ok(result)
    }

    /// 計画層 ReAct → 実行層 ReAct（直列）。
    fn run_turn_two_phase(&mut self, user_input: &str) -> Result<TurnResult, ReActError> {
        if self.config.verbose {
            eprintln!("[plan] layer loop for: {user_input}");
        }
        let turn_refs = self.take_pending_reference_info_for_plan();
        let (mut harness, plan_trace, plan_steps) = run_plan_layer(
            &mut self.plan_brain,
            &mut self.tools,
            &self.blocks,
            &self.session,
            user_input,
            self.config.max_steps_plan,
            self.config.verbose,
            self.config.show_prompt,
            self.config.show_tool_output,
            true,
            self.turn_observer.as_ref(),
            self.stop_requested.as_deref(),
        )?;
        Self::merge_turn_reference_info(&mut harness, turn_refs);
        self.apply_harness_from_plan(&mut harness, user_input);
        let plan = harness.plan.clone();
        self.notify_plan_artifact(&plan);
        if self.config.show_plan {
            println!("{}", format_plan_for_display(&plan, &self.task_registry));
        }
        if self.config.verbose {
            eprintln!(
                "[plan] summary={} skip={} subtasks={}",
                plan.summary,
                plan.skip_execution,
                plan.subtasks.len()
            );
        }

        if !plan.needs_execution() {
            let mut result =
                self.run_turn_single(user_input, true, Some(plan.clone()), vec![])?;
            append_trace(&mut result.trace, &plan_trace);
            result.context = TurnContextSummary::from_usages(&result.trace.context_usages);
            result.steps_used += plan_steps;
            result.harness = Some(harness);
            result.advance_phases.clear();
            self.clear_harness_prompt_blocks();
            return Ok(result);
        }

        let mut progress = PlanProgress::default();
        let mut subtask_results = Vec::new();
        let mut total_steps = plan_steps;
        let mut final_answer = String::new();
        let mut combined_trace = plan_trace;

        for subtask in &plan.subtasks {
            if self.is_stop_requested() {
                return Err(ReActError::Cancelled);
            }
            if self.config.show_task_execution {
                println!("--- Exec subtask {} ---", subtask.id);
                println!(
                    "{}",
                    self.task_registry
                        .format_subtask_execution_for_display(subtask)
                );
            }
            if self.config.verbose {
                eprintln!("[exec] subtask {}: {}", subtask.id, subtask.goal);
            }
            self.prepare_harness_for_subtask(&mut harness, subtask);
            let (exec, used_driver) =
                self.run_subtask_exec_audited(user_input, &plan, subtask, &progress)?;
            harness.advance_after_subtask(subtask.id);
            self.sync_harness_step_to_blocks(&harness);
            if self.config.show_task_execution {
                let mode = if used_driver { "step-driver" } else { "ReAct" };
                println!(
                    "  completed via {mode}: {}",
                    TaskRegistry::format_trace_tools_used(&exec.trace)
                );
            }
            total_steps += exec.steps_used;
            progress.push(subtask.id, exec.answer.clone());
            subtask_results.push(SubtaskExecResult {
                id: subtask.id,
                answer: exec.answer.clone(),
                steps_used: exec.steps_used,
                used_step_driver: used_driver,
            });
            final_answer = exec.answer;
            append_trace(&mut combined_trace, &exec.trace);
        }

        self.clear_harness_prompt_blocks();

        let result = TurnResult {
            answer: final_answer,
            context: TurnContextSummary::from_usages(&combined_trace.context_usages),
            trace: combined_trace,
            steps_used: total_steps,
            plan: Some(plan),
            harness: Some(harness),
            subtask_results,
            advance_phases: vec![],
        };
        self.finish_turn(user_input, &result);
        Ok(result)
    }

    /// サブタスク 1 件を実行し、タスク契約の監査で完了を検証する（未達なら同一サブタスクを再実行）。
    fn run_subtask_exec_audited(
        &mut self,
        user_input: &str,
        plan: &PlanArtifact,
        subtask: &Subtask,
        progress: &PlanProgress,
    ) -> Result<(TurnResult, bool), ReActError> {
        let mut last: Option<(TurnResult, bool)> = None;
        let mut audit_msg = String::new();

        for attempt in 1..=SUBTASK_AUDIT_MAX_ATTEMPTS {
            if self.is_stop_requested() {
                return Err(ReActError::Cancelled);
            }
            let (exec, used_driver) = if audit_msg.is_empty() {
                self.run_subtask_exec(user_input, plan, subtask, progress)?
            } else {
                let base =
                    format_mission(&self.task_registry, user_input, plan, subtask, progress);
                let mission = format!(
                    "{base}\n\n## Subtask audit (retry {attempt})\n\
                     The previous run did NOT satisfy the task execution contract.\n\
                     {audit_msg}\n\
                     Call every required tool in order before emitting answer.\n"
                );
                let exec = self.run_turn_single(&mission, false, None, vec![])?;
                (exec, false)
            };

            let audit = self.task_registry.audit_subtask(subtask, &exec.trace);
            let complete = audit.as_ref().map(|a| a.complete).unwrap_or(true);
            if self.config.verbose {
                if let Some(a) = &audit {
                    eprintln!(
                        "[tasks] subtask {} audit (attempt {attempt}): complete={} — {}",
                        subtask.id, a.complete, a.message
                    );
                }
            }
            if complete {
                return Ok((exec, used_driver));
            }
            audit_msg = audit
                .map(|a| a.message)
                .unwrap_or_else(|| "contract not satisfied".into());
            last = Some((exec, used_driver));
        }

        Ok(last.expect("subtask exec attempts"))
    }

    /// サブタスク 1 件: 契約ありならステップドライバ、それ以外は実行層 ReAct。
    fn run_subtask_exec(
        &mut self,
        user_input: &str,
        plan: &PlanArtifact,
        subtask: &Subtask,
        progress: &PlanProgress,
    ) -> Result<(TurnResult, bool), ReActError> {
        if self.config.use_step_driver && self.task_registry.use_step_driver(subtask) {
            match self
                .task_registry
                .run_subtask_driver(
                    subtask,
                    &mut self.tools,
                    self.config.verbose,
                    self.config.show_tool_output,
                )
            {
                Ok(drv) => {
                    if self.config.verbose {
                        eprintln!(
                            "[driver] subtask {} task={} steps={} audit_ok={}",
                            subtask.id, drv.task_id, drv.steps_used, drv.audit.complete
                        );
                    }
                    return Ok((
                        TurnResult {
                            answer: drv.answer,
                            context: TurnContextSummary::default(),
                            trace: drv.trace,
                            steps_used: drv.steps_used,
                            plan: None,
                            harness: None,
                            subtask_results: vec![],
                            advance_phases: vec![],
                        },
                        true,
                    ));
                }
                Err(err) => {
                    if self.is_stop_requested() {
                        return Err(ReActError::Cancelled);
                    }
                    if self.config.verbose {
                        eprintln!(
                            "[driver] subtask {} failed ({err}); falling back to ReAct",
                            subtask.id
                        );
                    }
                }
            }
        }
        let mission = format_mission(&self.task_registry, user_input, plan, subtask, progress);
        let saved_catalog = self.blocks.tool_catalog.clone();
        let policy = self.task_registry.tool_policy_for_subtask(subtask);
        if let Some(ref p) = policy {
            self.blocks.tool_catalog = self.tools.format_catalog_filtered(Some(p));
            self.tools.set_exec_policy(Some(p.clone()));
        } else {
            self.tools.set_exec_policy(None);
        }
        let exec_result = self.run_turn_single(&mission, false, None, vec![]);
        self.blocks.tool_catalog = saved_catalog;
        self.tools.set_exec_policy(None);
        let exec = exec_result?;
        Ok((exec, false))
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
            &self.blocks,
            &self.session,
            user_input,
            LayerLoopOptions::exec(self.config.max_steps),
            self.config.verbose,
            self.config.show_prompt,
            self.config.show_tool_output,
            plan,
            subtask_results,
            self.turn_observer.as_ref(),
            self.stop_requested.as_deref(),
        )?;
        if record_session {
            self.finish_turn(user_input, &result);
        }
        Ok(result)
    }

    fn finish_turn(&mut self, user_input: &str, result: &TurnResult) {
        self.session
            .push_turn(user_input.to_string(), result.answer.clone());
        if self.config.show_context_metrics && !result.context.is_empty() {
            eprintln!("[context turn] {}", result.context);
            if let Some(last) = result.trace.context_usages.last() {
                let sections = analyze_prompt_body(&last.prompt_body);
                eprintln!("[context map]\n{}", format_colormap(&sections, true));
            }
        }
        self.write_context_log(user_input, result);
        self.write_monitor_html(user_input, result);
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

fn append_trace(acc: &mut TurnTrace, step: &TurnTrace) {
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

#[cfg(test)]
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
        assert!(system.content.contains("note from host"));
    }

    #[test]
    fn session_accumulates_completed_turns() {
        let mut react = ReActLoop::with_defaults(SimpleRuleBrain::new());
        react.run_turn("help").unwrap();
        react.run_turn("help").unwrap();
        assert_eq!(react.session.len(), 2);
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
}
