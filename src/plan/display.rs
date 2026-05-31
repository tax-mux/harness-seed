//! 計画フェーズ（Phase 1）の `--plan-zone` 表示 — 図 [`doc/full_agent_architecture_v2.svg`] の用語で枠囲む。

use crate::action::TurnTrace;
use crate::harness::HarnessState;
use crate::context::PromptBlocks;
use crate::context_metrics::TurnContextSummary;
use crate::tasks::TaskRegistry;
use serde_json::Value;

use super::brain::PLAN_REACT_SYSTEM_CORE;
use super::prompt::{
    catalog_has_skill_entries, catalog_has_tool_entries, plan_task_catalog_for_blocks,
};

/// Phase 1 計画フェーズの stdout 表示（Planner 実行後）。
pub fn format_plan_zone_after_preview(
    blocks: &PromptBlocks,
    task_registry: &TaskRegistry,
    goal: &str,
    work_instructions: &str,
    harness: &HarnessState,
) -> String {
    let mut out = String::new();
    push_phase1_open(&mut out);
    push_goal(&mut out, goal);
    push_planner_fixed_zone(&mut out, blocks, task_registry, Some(harness));
    push_work_instructions(&mut out, work_instructions);
    push_harness_internal_state(&mut out, harness);
    push_phase1_close(&mut out);
    out
}

/// Phase 1 プロンプト全文プレビュー（`--plan-zone-full`）。
pub fn format_plan_zone_prompt_preview(
    blocks: &PromptBlocks,
    task_registry: &TaskRegistry,
    goal: &str,
    prompt_body: &str,
) -> String {
    let mut out = String::new();
    push_phase1_open(&mut out);
    push_goal(&mut out, goal);
    push_planner_fixed_zone(&mut out, blocks, task_registry, None);
    push_section(&mut out, "Planner入力（LLMプロンプト全文）", prompt_body);
    push_phase1_close(&mut out);
    out
}

/// Planner固定ゾーンのみをHTML描画する（観察用）。
pub fn format_planner_fixed_zone_html(
    blocks: &PromptBlocks,
    task_registry: &TaskRegistry,
    harness: Option<&HarnessState>,
    planner_output: Option<&str>,
    latest_user_input: Option<&str>,
    turn_context: Option<&TurnContextSummary>,
    turn_trace: Option<&TurnTrace>,
    compressed_chunks: &[String],
    recent_turns: Option<&str>,
    subtask_modes: &[(u32, bool)],
) -> String {
    let sections = planner_fixed_zone_sections(blocks, task_registry, harness);
    let mut out = String::new();
    out.push_str("<!doctype html>\n");
    out.push_str("<html lang=\"ja\">\n<head>\n<meta charset=\"utf-8\">\n");
    out.push_str(
        "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n",
    );
    out.push_str("<title>Planner監視ビュー</title>\n");
    out.push_str("<style>\n");
    out.push_str(":root{color-scheme:light dark;}\n");
    out.push_str(
        "body{font-family:\"Segoe UI\",\"Hiragino Kaku Gothic ProN\",Meiryo,sans-serif;margin:24px;line-height:1.5;}\n",
    );
    out.push_str("main{max-width:1100px;margin:0 auto;}\n");
    out.push_str("h1{font-size:1.4rem;margin:0 0 8px;}\n");
    out.push_str(".subtitle{margin:0 0 16px;color:#777;font-size:0.92rem;}\n");
    out.push_str("details{margin:0 0 14px;border:1px solid #9996;border-radius:10px;padding:10px 12px;}\n");
    out.push_str("summary{cursor:pointer;font-weight:600;font-size:1rem;}\n");
    out.push_str("details[open]{background:rgba(120,120,120,0.06);}\n");
    out.push_str("details.raw{background:transparent;}\n");
    out.push_str("details.subtask-zone{margin:10px 0 12px;padding:8px 10px;}\n");
    out.push_str("details.subtask-zone > summary{font-size:0.96rem;}\n");
    out.push_str(".mode-line{margin:0 0 10px;font-size:0.92rem;}\n");
    out.push_str(".mode-pill{display:inline-block;padding:2px 8px;border-radius:999px;border:1px solid transparent;font-weight:700;}\n");
    out.push_str(".mode-driver{color:#1f6feb;background:#1f6feb22;border-color:#1f6feb66;}\n");
    out.push_str(".mode-react{color:#2ea043;background:#2ea04322;border-color:#2ea04366;}\n");
    out.push_str(".mode-unknown{color:#9e6a03;background:#9e6a0322;border-color:#9e6a0366;}\n");
    out.push_str("section{margin:0 0 20px;}\n");
    out.push_str(
        "h2{font-size:1rem;margin:0 0 8px;padding-bottom:6px;border-bottom:1px solid #9996;}\n",
    );
    out.push_str("h3{font-size:0.96rem;margin:0 0 8px;}\n");
    out.push_str("pre{margin:0;padding:12px;border:1px solid #9996;border-radius:8px;overflow:auto;white-space:pre-wrap;word-break:break-word;}\n");
    out.push_str(".timeline{margin:0;padding:0;list-style:none;display:grid;gap:10px;}\n");
    out.push_str(".event{border:1px solid #9996;border-radius:8px;padding:10px;background:rgba(120,120,120,0.04);}\n");
    out.push_str(".event-head{display:flex;gap:8px;align-items:center;margin:0 0 6px;}\n");
    out.push_str(".event-no{font-weight:700;color:#888;min-width:32px;}\n");
    out.push_str(".tag{display:inline-block;border:1px solid #9996;border-radius:999px;padding:1px 8px;font-size:0.78rem;}\n");
    out.push_str(".tag-p1{background:#ef9f271f;color:#9e6a03;}\n");
    out.push_str(".tag-p2{background:#f0997b1f;color:#9c4221;}\n");
    out.push_str(".tag-p3{background:#5dcaa51f;color:#0d664f;}\n");
    out.push_str(".event-title{font-weight:600;}\n");
    out.push_str(".event-body{margin:0;font-size:0.92rem;white-space:pre-wrap;word-break:break-word;}\n");
    out.push_str("</style>\n</head>\n<body>\n<main>\n");
    out.push_str("<h1>Planner監視ビュー</h1>\n");
    out.push_str("<p class=\"subtitle\">Phase 1→2→3 を時系列で追えるように並べ替えた表示です。上から読むだけで流れを追跡できます。</p>\n");

    push_html_section(
        &mut out,
        "今回の入力と内部状態",
        &format_turn_snapshot_for_html(
            latest_user_input,
            turn_context,
            turn_trace,
            harness,
        ),
    );

    out.push_str("<details open>\n<summary>時系列トレース（Phase 1 → 2 → 3）</summary>\n<div>\n");
    push_html_section_raw(
        &mut out,
        "トレースイベント",
        &format_timeline_events_html(
            latest_user_input,
            planner_output,
            harness,
            turn_trace,
            task_registry,
            subtask_modes,
        ),
    );
    out.push_str("</div>\n</details>\n");

    out.push_str("<details class=\"raw\">\n<summary>Phase 1 詳細ログ</summary>\n<div>\n");
    push_html_section(
        &mut out,
        "Planner指令（システム）",
        &sections.planner_instructions,
    );
    push_html_section(&mut out, "ツール定義", &sections.tool_definitions);
    push_html_section(&mut out, "スキル一覧", &sections.skills);
    push_html_section(&mut out, "参照情報", &sections.reference_info);
    push_html_section(
        &mut out,
        "圧縮ゾーン",
        &format_compressed_zone_for_html(compressed_chunks),
    );
    push_html_section(
        &mut out,
        "直近ゾーン",
        &format_recent_zone_for_html(recent_turns),
    );
    push_html_section(
        &mut out,
        "Planner出力（作業指示書）",
        &format_planner_output_for_html(planner_output),
    );
    out.push_str("</div>\n</details>\n");

    out.push_str("<details class=\"raw\">\n<summary>Phase 2 詳細ログ</summary>\n<div>\n");
    push_html_section(
        &mut out,
        "Harness内部状態（JSON）",
        &format_harness_state_for_html(harness),
    );
    push_html_section(
        &mut out,
        "Harness内部状態（ミニPlanner適用後）",
        &format_harness_state_after_mini_planner_for_html(harness, task_registry),
    );
    out.push_str("</div>\n</details>\n");

    out.push_str("<details class=\"raw\">\n<summary>Phase 3 詳細ログ</summary>\n<div>\n");
    push_html_section_raw(
        &mut out,
        "タスク実行プラン",
        &format_phase3_tasks_for_html(blocks, harness, task_registry, subtask_modes),
    );
    out.push_str("</div>\n</details>\n");

    out.push_str("</main>\n</body>\n</html>\n");
    out
}

fn format_planner_output_for_html(planner_output: Option<&str>) -> String {
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

fn format_compressed_zone_for_html(compressed_chunks: &[String]) -> String {
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

fn format_recent_zone_for_html(recent_turns: Option<&str>) -> String {
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

fn format_turn_snapshot_for_html(
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

fn summarize_text(text: &str, max_chars: usize) -> String {
    let compact = text.replace('\n', " ");
    let compact = compact.trim();
    if compact.chars().count() <= max_chars {
        return compact.to_string();
    }
    let mut out = compact.chars().take(max_chars).collect::<String>();
    out.push_str("...");
    out
}

fn push_timeline_event(
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

fn format_timeline_events_html(
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
    push_timeline_event(&mut out, idx, "Phase 1", "tag-p1", "ユーザー指示を受領", goal);
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

        let current = hs.current_subtask().map(|s| {
            let task = s.task.clone().unwrap_or_else(|| "(freeform)".to_string());
            format!(
                "current_step={} / task={} / goal={}",
                hs.current_step,
                task,
                summarize_text(&s.goal, 160)
            )
        }).unwrap_or_else(|| "current_step=0 (no active subtask)".to_string());
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

fn try_pretty_json(s: &str) -> Option<String> {
    let value: Value = serde_json::from_str(s).ok()?;
    serde_json::to_string_pretty(&value).ok()
}

fn strip_json_fence(s: &str) -> Option<&str> {
    let content = s.strip_prefix("```json")?.strip_suffix("```")?;
    Some(content.trim())
}

fn format_harness_state_for_html(harness: Option<&HarnessState>) -> String {
    let Some(harness) = harness else {
        return "（なし）".to_string();
    };
    harness.to_json_pretty()
}

fn format_harness_state_after_mini_planner_for_html(
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

fn format_phase3_tasks_for_html(
    blocks: &PromptBlocks,
    harness: Option<&HarnessState>,
    task_registry: &TaskRegistry,
    subtask_modes: &[(u32, bool)],
) -> String {
    let mode_by_id: std::collections::HashMap<u32, bool> =
        subtask_modes.iter().copied().collect();
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
        out.push_str(&escape_html(&simulated.format_current_step_for_prompt(task_registry)));
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

fn filter_catalog_for_policy(catalog: &str, allow: &[String]) -> String {
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

fn push_phase1_open(out: &mut String) {
    out.push_str("### Phase 1　計画フェーズ ###\n\n");
}

fn push_phase1_close(out: &mut String) {
    out.push_str("### END Phase 1　計画フェーズ ###\n");
}

fn push_goal(out: &mut String, goal: &str) {
    push_section(out, "ゴール", goal);
}

fn push_work_instructions(out: &mut String, text: &str) {
    push_section(out, "作業指示書", text);
}

fn push_harness_internal_state(out: &mut String, harness: &HarnessState) {
    push_section(out, "Harness内部状態（JSON）", &harness.to_json_pretty());
}

fn push_planner_fixed_zone(
    out: &mut String,
    blocks: &PromptBlocks,
    task_registry: &TaskRegistry,
    harness: Option<&HarnessState>,
) {
    let sections = planner_fixed_zone_sections(blocks, task_registry, harness);
    out.push_str("### Planner固定ゾーン ###\n\n");
    push_section(out, "Planner指令（システム）", &sections.planner_instructions);
    push_section(out, "ツール定義", &sections.tool_definitions);
    push_section(out, "スキル一覧", &sections.skills);
    push_section(out, "参照情報", &sections.reference_info);
    out.push_str("### END Planner固定ゾーン ###\n\n");
}

fn push_section(out: &mut String, title: &str, body: &str) {
    out.push_str(&format!("### {title} ###\n"));
    let trimmed = body.trim();
    if trimmed.is_empty() {
        out.push_str("（なし）\n");
    } else {
        out.push_str(trimmed);
        if !trimmed.ends_with('\n') {
            out.push('\n');
        }
    }
    out.push_str(&format!("### END {title} ###\n\n"));
}

fn push_html_section(out: &mut String, title: &str, body: &str) {
    out.push_str("<section>\n<h2>");
    out.push_str(&escape_html(title));
    out.push_str("</h2>\n<pre>");
    let trimmed = body.trim();
    if trimmed.is_empty() {
        out.push_str("（なし）");
    } else {
        out.push_str(&escape_html(trimmed));
    }
    out.push_str("</pre>\n</section>\n");
}

fn push_html_section_raw(out: &mut String, title: &str, raw_html_body: &str) {
    out.push_str("<section>\n<h2>");
    out.push_str(&escape_html(title));
    out.push_str("</h2>\n");
    if raw_html_body.trim().is_empty() {
        out.push_str("<pre>（なし）</pre>\n");
    } else {
        out.push_str(raw_html_body);
        if !raw_html_body.ends_with('\n') {
            out.push('\n');
        }
    }
    out.push_str("</section>\n");
}

fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

struct PlannerFixedZoneSections {
    planner_instructions: String,
    tool_definitions: String,
    skills: String,
    reference_info: String,
}

fn planner_fixed_zone_sections(
    blocks: &PromptBlocks,
    task_registry: &TaskRegistry,
    harness: Option<&HarnessState>,
) -> PlannerFixedZoneSections {
    let task_catalog = plan_task_catalog_for_blocks(blocks, task_registry);
    PlannerFixedZoneSections {
        planner_instructions: format_planner_instructions(blocks),
        tool_definitions: format_tool_definitions_section(&blocks.tool_catalog),
        skills: format_skills_section(&task_catalog),
        reference_info: format_reference_info_section(&blocks.recalled, harness),
    }
}

fn format_planner_instructions(blocks: &PromptBlocks) -> String {
    let mut out = String::from(PLAN_REACT_SYSTEM_CORE);
    if blocks.web_search_enabled {
        out.push_str(
            "\n- Web search が有効: 外部・時事向けは task `web_research` を検討。\n",
        );
    }
    if !blocks.rules.is_empty() {
        out.push_str("\n\n追加ルール:\n");
        for (i, rule) in blocks.rules.iter().enumerate() {
            out.push_str(&format!("\n[rule {}]\n{rule}\n", i + 1));
        }
    }
    if let Some(contract) = &blocks.plan_data_contract {
        out.push_str("\n\n");
        out.push_str(&contract.format_for_planner());
    }
    out.push_str("\n\n実行環境:\n");
    out.push_str(&blocks.runtime.prompt_hint());
    out
}

fn format_tool_definitions_section(catalog: &str) -> String {
    if catalog_has_tool_entries(catalog) {
        catalog.trim().to_string()
    } else {
        "（なし）".into()
    }
}

fn format_skills_section(catalog: &str) -> String {
    if catalog_has_skill_entries(catalog) {
        catalog.trim().to_string()
    } else {
        "（なし）".into()
    }
}

fn format_reference_info_section(recalled: &[String], harness: Option<&HarnessState>) -> String {
    let from_harness = harness
        .map(HarnessState::format_references_for_prompt)
        .unwrap_or_default();
    if !from_harness.is_empty() {
        return from_harness;
    }
    if recalled.is_empty() {
        return "（なし）".into();
    }
    recalled
        .iter()
        .enumerate()
        .map(|(i, chunk)| format!("[recalled {}]\n{chunk}", i + 1))
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::HarnessState;
    use crate::plan::PlanArtifact;

    #[test]
    fn plan_zone_uses_diagram_japanese_section_titles() {
        let blocks = PromptBlocks::default();
        let reg = TaskRegistry::builtin();
        let hs = HarnessState::new("1. step", PlanArtifact::single_subtask("do"));
        let text = format_plan_zone_after_preview(&blocks, &reg, "フォルダ一覧", "{}", &hs);
        assert!(text.contains("### Phase 1　計画フェーズ ###"));
        assert!(text.contains("### ゴール ###"));
        assert!(text.contains("### Planner固定ゾーン ###"));
        assert!(text.contains("### ツール定義 ###"));
        assert!(text.contains("### スキル一覧 ###"));
        assert!(text.contains("### 参照情報 ###"));
        assert!(text.contains("### 作業指示書 ###"));
        assert!(text.contains("### Harness内部状態（JSON） ###"));
        assert!(!text.contains("Plan request"));
        assert!(!text.contains("Planner fixed zone (system)"));
    }

    #[test]
    fn empty_tool_skill_mail_show_nashi() {
        let mut blocks = PromptBlocks::default();
        blocks.tool_catalog.clear();
        blocks.plan_task_catalog = Some(String::new());
        blocks.recalled.clear();
        let sections = planner_fixed_zone_sections(&blocks, &TaskRegistry::builtin(), None);
        assert_eq!(sections.tool_definitions, "（なし）");
        assert_eq!(sections.skills, "（なし）");
        assert_eq!(sections.reference_info, "（なし）");
    }

    #[test]
    fn planner_fixed_zone_html_escapes_body() {
        let mut blocks = PromptBlocks::default();
        blocks
            .rules
            .push("allow <tag> & \"quote\" 'single'".to_string());
        let html = format_planner_fixed_zone_html(
            &blocks,
            &TaskRegistry::builtin(),
            None,
            Some("allow <tag> & \"quote\" 'single'"),
            None,
            None,
            None,
            &[],
            None,
            &[],
        );
        assert!(html.contains("&lt;tag&gt; &amp; &quot;quote&quot;"));
        assert!(html.contains("&#39;single&#39;"));
        assert!(!html.contains("<tag>"));
    }

    #[test]
    fn planner_output_json_is_pretty_formatted() {
        let blocks = PromptBlocks::default();
        let html = format_planner_fixed_zone_html(
            &blocks,
            &TaskRegistry::builtin(),
            None,
            Some("{\"a\":1,\"nested\":{\"b\":2}}"),
            None,
            None,
            None,
            &[],
            None,
            &[],
        );
        assert!(html.contains("\n  &quot;a&quot;: 1,"));
        assert!(html.contains("\n  &quot;nested&quot;: {"));
    }

    #[test]
    fn harness_state_is_embedded_in_html() {
        let blocks = PromptBlocks::default();
        let harness = HarnessState::new("1. step", PlanArtifact::single_subtask("do"));
        let html = format_planner_fixed_zone_html(
            &blocks,
            &TaskRegistry::builtin(),
            Some(&harness),
            Some("{}"),
            None,
            None,
            None,
            &[],
            None,
            &[],
        );
        assert!(html.contains("Harness内部状態（JSON）"));
        assert!(html.contains("&quot;current_step&quot;"));
    }

    #[test]
    fn harness_state_after_mini_planner_is_embedded_in_html() {
        let blocks = PromptBlocks::default();
        let harness = HarnessState::new(
            "{}",
            PlanArtifact {
                summary: "single task".into(),
                skip_execution: false,
                subtasks: vec![crate::plan::Subtask {
                    id: 1,
                    task: Some("list_dir".into()),
                    params: serde_json::json!({}),
                    goal: "dir".into(),
                    done_when: "done".into(),
                }],
            },
        );
        let html = format_planner_fixed_zone_html(
            &blocks,
            &TaskRegistry::builtin(),
            Some(&harness),
            Some("{}"),
            None,
            None,
            None,
            &[],
            None,
            &[],
        );
        assert!(html.contains("Harness内部状態（ミニPlanner適用後）"));
        assert!(html.contains("&quot;tool_set&quot;: ["));
        assert!(html.contains("&quot;list_dir&quot;"));
    }

    #[test]
    fn html_wraps_sections_with_phase_accordions() {
        let blocks = PromptBlocks::default();
        let html = format_planner_fixed_zone_html(
            &blocks,
            &TaskRegistry::builtin(),
            None,
            Some("{}"),
            None,
            None,
            None,
            &[],
            None,
            &[],
        );
        assert!(html.contains("<summary>時系列トレース（Phase 1 → 2 → 3）</summary>"));
        assert!(html.contains("<summary>Phase 1 詳細ログ</summary>"));
        assert!(html.contains("<summary>Phase 2 詳細ログ</summary>"));
        assert!(html.contains("<summary>Phase 3 詳細ログ</summary>"));
    }

    #[test]
    fn phase3_tasks_are_embedded_in_html() {
        let blocks = PromptBlocks::default();
        let harness = HarnessState::new(
            "{}",
            PlanArtifact {
                summary: "single task".into(),
                skip_execution: false,
                subtasks: vec![crate::plan::Subtask {
                    id: 1,
                    task: Some("list_dir".into()),
                    params: serde_json::json!({}),
                    goal: "dir".into(),
                    done_when: "done".into(),
                }],
            },
        );
        let html = format_planner_fixed_zone_html(
            &blocks,
            &TaskRegistry::builtin(),
            Some(&harness),
            Some("{}"),
            None,
            None,
            None,
            &[],
            None,
            &[],
        );
        assert!(html.contains("タスク実行プラン"));
        assert!(html.contains("<details class=\"subtask-zone\" open>"));
        assert!(html.contains("今のステップ（Harnessがテキスト変換）"));
        assert!(html.contains("スキーマ・ツール定義（ステップ別）"));
        assert!(!html.contains("&lt;details class=&quot;subtask-zone&quot;"));
    }

    #[test]
    fn compressed_and_recent_zones_are_embedded_in_html() {
        let blocks = PromptBlocks::default();
        let compressed = vec!["phase1 summary".to_string(), "phase2 summary".to_string()];
        let recent = "Previous turns:\n[turn 1]\nUser: hi\nAssistant: hello";
        let html = format_planner_fixed_zone_html(
            &blocks,
            &TaskRegistry::builtin(),
            None,
            Some("{}"),
            None,
            None,
            None,
            &compressed,
            Some(recent),
            &[],
        );
        assert!(html.contains("圧縮ゾーン"));
        assert!(html.contains("[recalled 1]"));
        assert!(html.contains("phase2 summary"));
        assert!(html.contains("直近ゾーン"));
        assert!(html.contains("Previous turns:"));
        assert!(html.contains("User: hi"));
    }

    #[test]
    fn phase3_subtask_mode_badges_are_embedded_in_html() {
        let blocks = PromptBlocks::default();
        let harness = HarnessState::new(
            "{}",
            PlanArtifact {
                summary: "single task".into(),
                skip_execution: false,
                subtasks: vec![crate::plan::Subtask {
                    id: 1,
                    task: Some("list_dir".into()),
                    params: serde_json::json!({}),
                    goal: "dir".into(),
                    done_when: "done".into(),
                }],
            },
        );
        let html = format_planner_fixed_zone_html(
            &blocks,
            &TaskRegistry::builtin(),
            Some(&harness),
            Some("{}"),
            None,
            None,
            None,
            &[],
            None,
            &[(1, true)],
        );
        assert!(html.contains("実行モード"));
        assert!(html.contains("mode-driver"));
        assert!(html.contains("step-driver"));
    }
    #[test]
    fn top_snapshot_is_embedded_in_html() {
        use crate::action::{Action, Observation, TurnTrace};

        let blocks = PromptBlocks::default();
        let mut trace = TurnTrace::default();
        trace.push_thought("considered the latest input".into());
        trace.push_action(Action::new(1, "grep", serde_json::json!({"pattern": "README"})));
        trace.push_observation(Observation::success(1, "README.md"));

        let harness = HarnessState::new("{}", PlanArtifact::single_subtask("do"));
        let html = format_planner_fixed_zone_html(
            &blocks,
            &TaskRegistry::builtin(),
            Some(&harness),
            Some("{}"),
            Some("最新の入力"),
            Some(&crate::context_metrics::TurnContextSummary::default()),
            Some(&trace),
            &[],
            None,
            &[],
        );
        assert!(html.contains("今回の入力と内部状態"));
        assert!(html.contains("最新ユーザープロンプト"));
        assert!(html.contains("最新の入力"));
        assert!(html.contains("内部状態サマリ（Context）"));
        assert!(html.contains("今回の動き（Trace）"));
        assert!(html.contains("thoughts=1 / actions=1 / observations=1"));
        assert!(html.contains("last_action: grep"));
        assert!(html.contains("Harness内部状態:"));
    }
}
