use std::fmt;
use std::io;
use std::path::PathBuf;

use crate::action::TurnTrace;
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
use crate::tool::ToolRuntime;

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
    /// 各ツールのコマンド・引数・実行結果を stderr に表示する（既定 ON）。
    pub show_tool_output: bool,
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
        )
    }

    pub fn with_blocks_and_tasks(
        exec_brain: E,
        plan_brain: PlanBrainMode,
        config: ReActConfig,
        blocks: PromptBlocks,
        task_registry: TaskRegistry,
        brave_search: Option<BraveSearchConfig>,
    ) -> Self {
        let session = SessionMemory::new(config.session_max_turns);
        let runtime = RuntimeEnvironment::detect();
        let mut blocks = blocks;
        blocks.runtime = runtime.clone();
        blocks.web_search_enabled = brave_search.is_some();
        Self {
            exec_brain,
            plan_brain,
            tools: ToolRuntime::with_environment_and_brave(runtime, brave_search),
            config,
            session,
            blocks,
            task_registry,
        }
    }

    pub fn with_defaults(exec_brain: E) -> Self {
        Self::new(exec_brain, PlanBrainMode::rule(), ReActConfig::default())
    }

    /// CLI の `-v` / `--verbose` を反映する。
    pub fn apply_cli_verbose(&mut self, verbose: bool) {
        self.config.verbose = verbose;
    }

    pub fn run_turn(&mut self, user_input: &str) -> Result<TurnResult, ReActError> {
        if self.config.two_phase {
            self.run_turn_two_phase(user_input)
        } else {
            self.run_turn_single(user_input, true, None, vec![])
        }
    }

    /// 計画層 ReAct → 実行層 ReAct（直列）。
    fn run_turn_two_phase(&mut self, user_input: &str) -> Result<TurnResult, ReActError> {
        if self.config.verbose {
            eprintln!("[plan] layer loop for: {user_input}");
        }
        let (mut plan, plan_steps) = run_plan_layer(
            &mut self.plan_brain,
            &mut self.tools,
            &self.blocks,
            &self.session,
            user_input,
            self.config.max_steps_plan,
            self.config.verbose,
            self.config.show_prompt,
            self.config.show_tool_output,
        )?;
        self.task_registry.resolve_plan(&mut plan);
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
            let result = self.run_turn_single(user_input, true, Some(plan.clone()), vec![])?;
            return Ok(result);
        }

        let mut progress = PlanProgress::default();
        let mut subtask_results = Vec::new();
        let mut total_steps = plan_steps;
        let mut final_answer = String::new();
        let mut combined_trace = TurnTrace::default();

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
            let (exec, used_driver) = self.run_subtask_exec(user_input, &plan, subtask, &progress)?;
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
            if self.config.verbose {
                if let Some(audit) = self.task_registry.audit_subtask(subtask, &exec.trace) {
                    eprintln!(
                        "[tasks] subtask {} audit: complete={} — {}",
                        subtask.id, audit.complete, audit.message
                    );
                }
            }
        }

        let result = TurnResult {
            answer: final_answer,
            context: TurnContextSummary::from_usages(&combined_trace.context_usages),
            trace: combined_trace,
            steps_used: total_steps,
            plan: Some(plan),
            subtask_results,
        };
        self.finish_turn(user_input, &result);
        Ok(result)
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
        let exec = self.run_turn_single(&mission, false, None, vec![])?;
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
        assert_eq!(result.steps_used, 1);
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
}
