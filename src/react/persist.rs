//! ターン終了時の diary / コンテキストログ / 監視 HTML。
use std::fs;

use std::path::PathBuf;

use crate::brain::AgentBrain;
use crate::context_log::{
    format_step_console_lines, format_turn_console_summary, ContextLogWriter,
};
use crate::context_map::{aggregate_prompt_sections, analyze_prompt_body, format_colormap_titled};
use crate::lifecycle::TurnOutcome;
use crate::memory::{DiaryEntry, DiaryPhase};
use crate::monitor_view::render_monitor_html_from_log;

use super::{ReActLoop, TurnResult};

impl<E: AgentBrain> ReActLoop<E> {
    pub(super) fn finish_turn(&mut self, user_input: &str, result: &TurnResult) {
        self.session
            .push_turn(user_input.to_string(), result.answer.clone());
        self.record_diary(user_input, result);
        if self.config.show_context_metrics && !result.context.is_empty() {
            eprintln!("{}", format_turn_console_summary(user_input, result));
            for line in format_step_console_lines(result) {
                eprintln!("{line}");
            }
            if self.config.verbose {
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
        }
        self.write_context_log(user_input, result);
        self.write_monitor_html();
        let outcome = TurnOutcome::completed(&result.answer, result.steps_used);
        self.emit_turn_finished(user_input, result.plan.as_ref(), &outcome);
    }

    pub(super) fn record_diary(&mut self, user_input: &str, result: &TurnResult) {
        let summary = result
            .plan
            .as_ref()
            .map(|p| p.summary.clone())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| result.answer.chars().take(200).collect::<String>());
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

    pub(super) fn write_context_log(&self, user_input: &str, result: &TurnResult) {
        if result.context.is_empty() {
            return;
        }
        let Some(path) = &self.config.context_log_path else {
            return;
        };
        let writer = ContextLogWriter::new(path).with_rotation(self.config.log_rotation);
        match writer.append_turn(user_input, result) {
            Ok(()) => {
                if self.config.verbose {
                    eprintln!("context log: appended to {}", path.display());
                }
            }
            Err(err) => eprintln!("context log: failed to write {}: {err}", path.display()),
        }
    }

    pub(super) fn write_monitor_html(&self) {
        if !self.config.monitor_plan_html {
            return;
        }

        let monitor_dir = PathBuf::from("monitor");
        if let Err(err) = fs::create_dir_all(&monitor_dir) {
            eprintln!(
                "monitor html: failed to create {}: {err}",
                monitor_dir.display()
            );
            return;
        }

        // ログ追記後の events を埋め込む（パスが無い場合は空のビューア）。
        let html = render_monitor_html_from_log(self.config.context_log_path.as_deref());
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
