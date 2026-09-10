//! 計画フェーズ（Phase 1）の `--plan-zone` 表示 — 図 [`doc/ja/architecture/full_agent_architecture_v2.svg`] の用語で枠囲む。

use crate::action::TurnTrace;
use crate::context::PromptBlocks;
use crate::context_metrics::TurnContextSummary;
use crate::harness::HarnessState;
use crate::tasks::TaskRegistry;

use super::brain::PLAN_REACT_SYSTEM_CORE;
use super::prompt::{
    catalog_has_skill_entries, catalog_has_tool_entries, plan_task_catalog_for_blocks,
};

mod html;
use html::*;

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
    out.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    out.push_str("<title>Planner監視ビュー</title>\n");
    out.push_str("<style>\n");
    out.push_str(":root{color-scheme:light dark;}\n");
    out.push_str(
        "body{font-family:\"Segoe UI\",\"Hiragino Kaku Gothic ProN\",Meiryo,sans-serif;margin:24px;line-height:1.5;}\n",
    );
    out.push_str("main{max-width:1100px;margin:0 auto;}\n");
    out.push_str("h1{font-size:1.4rem;margin:0 0 8px;}\n");
    out.push_str(".subtitle{margin:0 0 16px;color:#777;font-size:0.92rem;}\n");
    out.push_str(
        "details{margin:0 0 14px;border:1px solid #9996;border-radius:10px;padding:10px 12px;}\n",
    );
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
    out.push_str(
        ".event-body{margin:0;font-size:0.92rem;white-space:pre-wrap;word-break:break-word;}\n",
    );
    out.push_str("</style>\n</head>\n<body>\n<main>\n");
    out.push_str("<h1>Planner監視ビュー</h1>\n");
    out.push_str("<p class=\"subtitle\">Phase 1→2→3 を時系列で追えるように並べ替えた表示です。上から読むだけで流れを追跡できます。</p>\n");

    push_html_section(
        &mut out,
        "今回の入力と内部状態",
        &format_turn_snapshot_for_html(latest_user_input, turn_context, turn_trace, harness),
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
    push_section(
        out,
        "Planner指令（システム）",
        &sections.planner_instructions,
    );
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
        out.push_str("\n- Web search が有効: 外部・時事向けは task `web_research` を検討。\n");
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
mod tests;
