use crate::action::TurnTrace;
use crate::context::PromptBlocks;
use crate::context_metrics::TurnContextSummary;
use crate::harness::HarnessState;
use crate::tasks::TaskRegistry;
use serde_json::Value;

use super::escape_html;

pub(super) fn format_planner_output_for_html(planner_output: Option<&str>) -> String {
    let Some(raw) = planner_output else {
        return "（なし）".to_string();
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return "（なし）".to_string();
    }

    if let Some(pretty) = try_pretty_json(trimmed) {
        return pretty;
    }

    if let Some(unfenced) = strip_json_fence(trimmed) {
        if let Some(pretty) = try_pretty_json(unfenced) {
            return pretty;
        }
    }

    trimmed.to_string()
}

pub(super) fn format_compressed_zone_for_html(compressed_chunks: &[String]) -> String {
    if compressed_chunks.is_empty() {
        return "（なし）".to_string();
    }
    compressed_chunks
        .iter()
        .enumerate()
        .map(|(i, chunk)| format!("[recalled {}]\n{}", i + 1, chunk.trim()))
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub(super) fn format_recent_zone_for_html(recent_turns: Option<&str>) -> String {
    let Some(text) = recent_turns else {
        return "（なし）".to_string();
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        "（なし）".to_string()
    } else {
        trimmed.to_string()
    }
}

pub(super) fn format_turn_snapshot_for_html(
    latest_user_input: Option<&str>,
    turn_context: Option<&TurnContextSummary>,
    turn_trace: Option<&TurnTrace>,
    harness: Option<&HarnessState>,
) -> String {
    let mut out = String::new();

    out.push_str("最新ユーザープロンプト:\n");
    let user_input = latest_user_input
        .map(|text| text.trim())
        .filter(|text| !text.is_empty())
        .unwrap_or("（なし）");
    out.push_str(user_input);
    out.push_str("\n\n");

    out.push_str("内部状態サマリ（Context）:\n");
    out.push_str(
        &turn_context
            .map(|summary| summary.to_string())
            .unwrap_or_else(|| "（なし）".to_string()),
    );
    out.push_str("\n\n");

    out.push_str("今回の動き（Trace）:\n");
    if let Some(trace) = turn_trace {
        out.push_str(&format!(
            "thoughts={} / actions={} / observations={}\n",
            trace.thoughts.len(),
            trace.actions.len(),
            trace.observations.len()
        ));
        if let Some(last_action) = trace.actions.last() {
            out.push_str("last_action: ");
            out.push_str(&last_action.tool);
            out.push(' ');
            out.push_str(&summarize_text(&last_action.args.to_string(), 140));
            out.push('\n');
        }
        if let Some(last_obs) = trace.observations.last() {
            out.push_str("last_observation: ");
            out.push_str(if last_obs.ok { "ok " } else { "err " });
            out.push_str(&summarize_text(&last_obs.output, 180));
        }
    } else {
        out.push_str("（なし）");
    }
    out.push_str("\n\n");

    out.push_str("Harness内部状態:\n");
    if let Some(hs) = harness {
        out.push_str(&format!(
            "status={:?} / current_step={} / total_steps={} / tool_set={}",
            hs.status,
            hs.current_step,
            hs.total_steps,
            if hs.tool_set.is_empty() {
                "(none)".to_string()
            } else {
                hs.tool_set.join(", ")
            }
        ));
    } else {
        out.push_str("（なし）");
    }

    out
}

pub(super) fn summarize_text(text: &str, max_chars: usize) -> String {
    let compact = text.replace('\n', " ");
    let compact = compact.trim();
    if compact.chars().count() <= max_chars {
        return compact.to_string();
    }
    let mut out = compact.chars().take(max_chars).collect::<String>();
    out.push_str("...");
    out
}

pub(super) fn push_timeline_event(
    out: &mut String,
    index: usize,
    phase_tag: &str,
    phase_class: &str,
    title: &str,
    body: &str,
) {
    out.push_str("<li class=\"event\">\n<div class=\"event-head\">\n");
    out.push_str("<span class=\"event-no\">#");
    out.push_str(&index.to_string());
    out.push_str("</span>\n<span class=\"tag ");
    out.push_str(phase_class);
    out.push_str("\">");
    out.push_str(&escape_html(phase_tag));
    out.push_str("</span>\n<span class=\"event-title\">");
    out.push_str(&escape_html(title));
    out.push_str("</span>\n</div>\n<p class=\"event-body\">");
    out.push_str(&escape_html(body));
    out.push_str("</p>\n</li>\n");
}

pub(super) fn format_timeline_events_html(
    latest_user_input: Option<&str>,
    planner_output: Option<&str>,
    harness: Option<&HarnessState>,
    turn_trace: Option<&TurnTrace>,
    task_registry: &TaskRegistry,
    subtask_modes: &[(u32, bool)],
) -> String {
    let mut out = String::new();
    out.push_str("<ol class=\"timeline\">\n");

    let mut idx = 1usize;
    let goal = latest_user_input
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("（なし）");
    push_timeline_event(
        &mut out,
        idx,
        "Phase 1",
        "tag-p1",
        "ユーザー指示を受領",
        goal,
    );
    idx += 1;

    let planner_text = planner_output
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("（なし）");
    push_timeline_event(
        &mut out,
        idx,
        "Phase 1",
        "tag-p1",
        "Plannerが作業指示書を生成",
        &summarize_text(planner_text, 260),
    );
    idx += 1;

    if let Some(hs) = harness {
        let parse_body = format!(
            "Harnessがテキストを内部JSON化: skip_execution={} / subtasks={} / status={:?}",
            hs.plan.skip_execution,
            hs.plan.subtasks.len(),
            hs.status
        );
        push_timeline_event(
            &mut out,
            idx,
            "Phase 1",
            "tag-p1",
            "Harnessパース完了",
            &parse_body,
        );
        idx += 1;

        let current = hs
            .current_subtask()
            .map(|s| {
                let task = s.task.clone().unwrap_or_else(|| "(freeform)".to_string());
                format!(
                    "current_step={} / task={} / goal={}",
                    hs.current_step,
                    task,
                    summarize_text(&s.goal, 160)
                )
            })
            .unwrap_or_else(|| "current_step=0 (no active subtask)".to_string());
        push_timeline_event(
            &mut out,
            idx,
            "Phase 2",
            "tag-p2",
            "ミニPlanner入力（Harness内部状態）",
            &current,
        );
        idx += 1;

        if let Some(st) = hs.current_subtask() {
            let policy = task_registry.tool_policy_for_subtask(st);
            let allow = policy
                .as_ref()
                .map(|p| {
                    if p.allow.is_empty() {
                        "(none)".to_string()
                    } else {
                        p.allow.join(", ")
                    }
                })
                .unwrap_or_else(|| "(none)".to_string());
            push_timeline_event(
                &mut out,
                idx,
                "Phase 2",
                "tag-p2",
                "ミニPlanner出力（許可ツール）",
                &format!("allow tools: {allow}"),
            );
            idx += 1;
        }
    } else {
        push_timeline_event(
            &mut out,
            idx,
            "Phase 2",
            "tag-p2",
            "ミニPlanner",
            "Harness内部状態がないためスキップ",
        );
        idx += 1;
    }

    if let Some(hs) = harness {
        if hs.plan.subtasks.is_empty() {
            push_timeline_event(
                &mut out,
                idx,
                "Phase 3",
                "tag-p3",
                "実行フェーズ",
                "subtask がないため実行なし",
            );
            idx += 1;
        } else {
            for st in &hs.plan.subtasks {
                let mode = subtask_modes
                    .iter()
                    .find(|(id, _)| *id == st.id)
                    .map(|(_, is_driver)| if *is_driver { "step-driver" } else { "ReAct" })
                    .unwrap_or("未実行/不明");
                let title = format!("subtask {} を実行", st.id);
                let body = format!(
                    "mode={} / task={} / goal={}",
                    mode,
                    st.task.clone().unwrap_or_else(|| "(freeform)".to_string()),
                    summarize_text(&st.goal, 160)
                );
                push_timeline_event(&mut out, idx, "Phase 3", "tag-p3", &title, &body);
                idx += 1;
            }
        }
    }

    if let Some(trace) = turn_trace {
        for (i, thought) in trace.thoughts.iter().enumerate() {
            push_timeline_event(
                &mut out,
                idx,
                "Phase 3",
                "tag-p3",
                &format!("LLM thought {}", i + 1),
                &summarize_text(thought, 220),
            );
            idx += 1;
        }

        let mut obs_by_id: std::collections::HashMap<u64, &crate::action::Observation> =
            std::collections::HashMap::new();
        for obs in &trace.observations {
            obs_by_id.insert(obs.invoke_id, obs);
        }

        for action in &trace.actions {
            let action_body = format!(
                "tool={} args={}",
                action.tool,
                summarize_text(&action.args.to_string(), 180)
            );
            push_timeline_event(
                &mut out,
                idx,
                "Phase 3",
                "tag-p3",
                &format!("tool action #{}", action.invoke_id),
                &action_body,
            );
            idx += 1;

            if let Some(obs) = obs_by_id.get(&action.invoke_id) {
                let obs_body = format!(
                    "status={} output={}",
                    if obs.ok { "ok" } else { "err" },
                    summarize_text(&obs.output, 220)
                );
                push_timeline_event(
                    &mut out,
                    idx,
                    "Phase 3",
                    "tag-p3",
                    &format!("observation #{}", obs.invoke_id),
                    &obs_body,
                );
                idx += 1;
            }
        }
    } else {
        push_timeline_event(
            &mut out,
            idx,
            "Phase 3",
            "tag-p3",
            "ReAct trace",
            "trace が記録されていません",
        );
    }

    out.push_str("</ol>\n");
    out
}

pub(super) fn try_pretty_json(s: &str) -> Option<String> {
    let value: Value = serde_json::from_str(s).ok()?;
    serde_json::to_string_pretty(&value).ok()
}

pub(super) fn strip_json_fence(s: &str) -> Option<&str> {
    let content = s.strip_prefix("```json")?.strip_suffix("```")?;
    Some(content.trim())
}

pub(super) fn format_harness_state_for_html(harness: Option<&HarnessState>) -> String {
    let Some(harness) = harness else {
        return "（なし）".to_string();
    };
    harness.to_json_pretty()
}

pub(super) fn format_harness_state_after_mini_planner_for_html(
    harness: Option<&HarnessState>,
    task_registry: &TaskRegistry,
) -> String {
    let Some(harness) = harness else {
        return "（なし）".to_string();
    };

    let mut simulated = harness.clone();
    let subtask = simulated
        .current_subtask()
        .cloned()
        .or_else(|| simulated.plan.subtasks.first().cloned());

    let Some(subtask) = subtask else {
        return simulated.to_json_pretty();
    };

    if simulated.current_step == 0 {
        simulated.current_step = subtask.id;
    }
    let policy = task_registry.tool_policy_for_subtask(&subtask);
    simulated.set_tool_set_from_policy(policy.as_ref());
    simulated.to_json_pretty()
}

pub(super) fn format_phase3_tasks_for_html(
    blocks: &PromptBlocks,
    harness: Option<&HarnessState>,
    task_registry: &TaskRegistry,
    subtask_modes: &[(u32, bool)],
) -> String {
    let mode_by_id: std::collections::HashMap<u32, bool> = subtask_modes.iter().copied().collect();
    let Some(harness) = harness else {
        return "（なし）".to_string();
    };
    if harness.plan.subtasks.is_empty() {
        return "（なし）".to_string();
    }

    let mut out = String::new();
    let work_instructions = harness.format_work_instructions_for_prompt();
    for subtask in &harness.plan.subtasks {
        let mut simulated = harness.clone();
        simulated.current_step = subtask.id;
        let policy = task_registry.tool_policy_for_subtask(subtask);
        simulated.set_tool_set_from_policy(policy.as_ref());

        let catalog = if let Some(ref p) = policy {
            filter_catalog_for_policy(&blocks.tool_catalog, &p.allow)
        } else {
            blocks.tool_catalog.trim().to_string()
        };

        out.push_str("<details class=\"subtask-zone\" open>\n<summary>");
        out.push_str(&escape_html(&format!("subtask {}", subtask.id)));
        out.push_str("</summary>\n");

        let (mode_label, mode_class) = match mode_by_id.get(&subtask.id) {
            Some(true) => ("step-driver", "mode-driver"),
            Some(false) => ("ReAct", "mode-react"),
            None => ("未実行/不明", "mode-unknown"),
        };
        out.push_str("<p class=\"mode-line\">実行モード: <span class=\"mode-pill ");
        out.push_str(mode_class);
        out.push_str("\">");
        out.push_str(&escape_html(mode_label));
        out.push_str("</span></p>\n");

        out.push_str("<section>\n<h2>作業指示書</h2>\n<pre>");
        out.push_str(&escape_html(&work_instructions));
        out.push_str("</pre>\n</section>\n");

        out.push_str("<section>\n<h2>今のステップ（Harnessがテキスト変換）</h2>\n<pre>");
        out.push_str(&escape_html(
            &simulated.format_current_step_for_prompt(task_registry),
        ));
        out.push_str("</pre>\n</section>\n");

        out.push_str("<section>\n<h2>スキーマ・ツール定義（ステップ別）</h2>\n<pre>");
        if catalog.is_empty() {
            out.push_str("（なし）");
        } else {
            out.push_str(&escape_html(&catalog));
        }
        out.push_str("</pre>\n</section>\n");

        out.push_str("</details>\n");
    }
    out
}

pub(super) fn filter_catalog_for_policy(catalog: &str, allow: &[String]) -> String {
    if allow.is_empty() {
        return String::new();
    }
    let allow_set: std::collections::HashSet<&str> = allow.iter().map(String::as_str).collect();
    let mut out = Vec::new();
    for line in catalog.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("- ") {
            let name = rest.split(':').next().unwrap_or("").trim();
            if allow_set.contains(name) {
                out.push(line);
            }
        }
    }
    out.join("\n")
}
