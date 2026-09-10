use std::fmt;
use std::io;
use std::path::PathBuf;

use crate::action::TurnTrace;
use crate::advance::{
    prepare_phase_recalled, restore_base_recalled, AdvanceConfig, AdvancePhaseSummary,
    AdvanceProgress,
};
use crate::scout::{
    apply_scout_recalled, is_trivial_scout_skip, run_scout_phase, ResearchArtifact, ScoutConfig,
};
use crate::brain::AgentBrain;
use crate::context::PromptBlocks;
use crate::context_log::{default_log_path, ContextLogWriter};
use crate::context_map::{analyze_prompt_body, format_colormap};
use crate::context_metrics::TurnContextSummary;
use crate::layer::{run_layer_loop, run_plan_layer, LayerLoopOptions};
use crate::plan::{
    format_mission, format_plan_for_display, PlanArtifact, PlanBrainMode, PlanProgress, Subtask,
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
    /// 計画前スカウト（有効時は plan / advance / two_phase の前に実行）。
    pub scout: ScoutConfig,
}

impl Default for ReActConfig {
    fn default() -> Self {
        Self {
            max_steps: 16,
            verbose: false,
            show_context_metrics: true,
            context_log_path: Some(default_log_path()),
            session_max_turns: SessionMemory::DEFAULT_MAX_TURNS,
            two_phase: false,
            max_steps_plan: 4,
            use_step_driver: true,
            show_prompt: false,
            show_plan: true,
            show_task_execution: true,
            show_tool_output: true,
            advance: AdvanceConfig::default(),
            scout: ScoutConfig::default(),
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

/// 1回のターン実行結果。
#[derive(Debug)]
pub struct TurnResult {
    pub answer: String,
    pub trace: TurnTrace,
    pub steps_used: usize,
    pub context: TurnContextSummary,
    /// 計画フェーズの成果（two_phase 時のみ）。
    pub plan: Option<PlanArtifact>,
    /// サブタスク実行の列（two_phase・複数サブタスク時）。
    pub subtask_results: Vec<SubtaskExecResult>,
    /// 推進ループで実行したフェーズのサマリ（`advance.enabled` 時）。
    pub advance_phases: Vec<AdvancePhaseSummary>,
    /// 計画前スカウトの成果（`scout.enabled` 時）。
    pub scout: Option<ResearchArtifact>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ReActError {
    MaxStepsExceeded { limit: usize },
}

impl fmt::Display for ReActError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MaxStepsExceeded { limit } => {
                write!(f, "ReAct loop exceeded max steps ({limit})")
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
        }
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
    }

    fn notify_plan_artifact(&self, plan: &PlanArtifact) {
        let display = format_plan_for_display(plan, &self.task_registry);
        emit_plan_artifact(self.turn_observer.as_ref(), "plan", plan, &display);
    }

    pub fn run_turn(&mut self, user_input: &str) -> Result<TurnResult, ReActError> {
        let (scout, scout_trace, scout_steps) = self.maybe_run_scout(user_input)?;
        let mut result = if self.config.advance.enabled {
            self.run_turn_advance(user_input)?
        } else if self.config.two_phase {
            self.run_turn_two_phase(user_input)?
        } else {
            self.run_turn_single(user_input, true, None, vec![])?
        };
        result.scout = scout;
        result.steps_used += scout_steps;
        Ok(self.finalize_turn_result(result, scout_trace))
    }

    /// 計画前スカウト。`recalled` に調査メモを載せる（ホスト注入分は保持）。
    fn maybe_run_scout(
        &mut self,
        user_input: &str,
    ) -> Result<(Option<ResearchArtifact>, Option<TurnTrace>, usize), ReActError> {
        if !self.config.scout.enabled {
            return Ok((None, None, 0));
        }
        if self.config.scout.skip_trivial && is_trivial_scout_skip(user_input) {
            if self.config.verbose {
                eprintln!("[scout] skipped (trivial input)");
            }
            return Ok((None, None, 0));
        }

        let base_recalled = self.blocks.recalled.clone();
        if self.config.verbose {
            eprintln!("[scout] gathering context before plan");
        }

        let (artifact, trace, steps) = run_scout_phase(
            &mut self.exec_brain,
            &mut self.tools,
            &mut self.blocks,
            &self.session,
            user_input,
            &self.config.scout,
            self.config.verbose,
            self.config.show_prompt,
            self.config.show_tool_output,
            self.turn_observer.as_ref(),
        )?;

        apply_scout_recalled(
            &mut self.blocks,
            &base_recalled,
            &artifact,
            self.config.scout.max_note_chars,
        );

        if self.config.scout.show_scout {
            println!("--- Scout ---");
            println!(
                "ready_to_plan: {}  gaps: {}",
                artifact.ready_to_plan,
                if artifact.gaps.is_empty() {
                    "(none)".into()
                } else {
                    artifact.gaps.join("; ")
                }
            );
            if !artifact.notes.is_empty() {
                println!("notes: {}", artifact.notes);
            }
            println!("--- end scout ---");
        }
        if self.config.verbose {
            eprintln!(
                "[scout] ready_to_plan={} gaps={} steps={steps}",
                artifact.ready_to_plan,
                artifact.gaps.len()
            );
        }

        Ok((Some(artifact), Some(trace), steps))
    }

    fn finalize_turn_result(&self, mut result: TurnResult, scout_trace: Option<TurnTrace>) -> TurnResult {
        if let Some(scout) = scout_trace {
            let mut merged = scout;
            append_trace(&mut merged, &result.trace);
            result.trace = merged;
        }
        result.context = TurnContextSummary::from_usages(&result.trace.context_usages);
        result
    }

    /// 計画層 → フェーズ逐次実行。各フェーズ前に `recalled` へ進捗を載せ、必要なら session をクリア。
    fn run_turn_advance(&mut self, user_input: &str) -> Result<TurnResult, ReActError> {
        let advance = self.config.advance.clone();
        let base_recalled = self.blocks.recalled.clone();

        if self.config.verbose {
            eprintln!("[advance] planning for: {user_input}");
        }
        let (mut plan, plan_trace, plan_steps) = run_plan_layer(
            &mut self.plan_brain,
            &mut self.tools,
            &self.blocks,
            &self.session,
            user_input,
            self.config.max_steps_plan,
            self.config.verbose,
            self.config.show_prompt,
            self.config.show_tool_output,
            self.turn_observer.as_ref(),
        )?;
        self.task_registry.resolve_plan(&mut plan, user_input);
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
            result.advance_phases.clear();
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

        for subtask in plan.subtasks.iter().take(phase_limit) {
            if advance.clear_session_each_phase {
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

            let (exec, used_driver) =
                self.run_subtask_exec_audited(user_input, &plan, subtask, &plan_progress)?;

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

        let result = TurnResult {
            answer: final_answer,
            context: TurnContextSummary::from_usages(&combined_trace.context_usages),
            trace: combined_trace,
            steps_used: total_steps,
            plan: Some(plan),
            subtask_results,
            advance_phases,
            scout: None,
        };
        self.finish_turn(user_input, &result);
        Ok(result)
    }

    /// 計画層 ReAct → 実行層 ReAct（直列）。
    fn run_turn_two_phase(&mut self, user_input: &str) -> Result<TurnResult, ReActError> {
        if self.config.verbose {
            eprintln!("[plan] layer loop for: {user_input}");
        }
        let (mut plan, plan_trace, plan_steps) = run_plan_layer(
            &mut self.plan_brain,
            &mut self.tools,
            &self.blocks,
            &self.session,
            user_input,
            self.config.max_steps_plan,
            self.config.verbose,
            self.config.show_prompt,
            self.config.show_tool_output,
            self.turn_observer.as_ref(),
        )?;
        self.task_registry.resolve_plan(&mut plan, user_input);
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
            result.advance_phases.clear();
            return Ok(result);
        }

        let mut progress = PlanProgress::default();
        let mut subtask_results = Vec::new();
        let mut total_steps = plan_steps;
        let mut final_answer = String::new();
        let mut combined_trace = plan_trace;

        for subtask in &plan.subtasks {
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
            let (exec, used_driver) =
                self.run_subtask_exec_audited(user_input, &plan, subtask, &progress)?;
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

        let result = TurnResult {
            answer: final_answer,
            context: TurnContextSummary::from_usages(&combined_trace.context_usages),
            trace: combined_trace,
            steps_used: total_steps,
            plan: Some(plan),
            subtask_results,
            advance_phases: vec![],
            scout: None,
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
                            subtask_results: vec![],
                            advance_phases: vec![],
                            scout: None,
                        },
                        true,
                    ));
                }
                Err(err) => {
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
    }

    fn write_context_log(&self, user_input: &str, result: &TurnResult) {
        if result.context.is_empty() {
            return;
        }
        let Some(path) = &self.config.context_log_path else {
            return;
        };
        let writer = ContextLogWriter::new(path);
        match writer.append_turn(user_input, result) {
            Ok(()) => eprintln!("context log: appended to {}", path.display()),
            Err(err) => eprintln!("context log: failed to write {}: {err}", path.display()),
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
    fn scout_skips_trivial_help() {
        let mut config = ReActConfig::default();
        config.scout.enabled = true;
        let mut react = ReActLoop::with_defaults(SimpleRuleBrain::new());
        react.config = config;
        let result = react.run_turn("help").unwrap();
        assert!(result.scout.is_none());
    }

    #[test]
    fn scout_populates_recalled_for_generic_input() {
        let mut config = ReActConfig::default();
        config.scout.enabled = true;
        config.scout.show_scout = false;
        config.two_phase = false;
        config.advance.enabled = false;
        let mut react =
            ReActLoop::new(SimpleRuleBrain::new(), PlanBrainMode::rule(), config);
        let result = react.run_turn("survey the project").unwrap();
        assert!(result.scout.is_some());
        assert!(
            react
                .blocks
                .recalled
                .iter()
                .any(|c| c.contains("Scout findings"))
        );
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
}
