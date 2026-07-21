//! 計画フェーズ: 課題に有効なタスク候補を選び、コンテキストへ登録する。
//!
//! タスク id・ドメイン語彙はホスト／`tasks/*.json` 側の責務。
//! このモジュールは候補選定の手続きだけを持ち、特定アプリ（メール等）に依存しない。

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::action::{AgentStep, TurnTrace};
use crate::brain::AgentBrain;
use crate::context::{PromptBlocks, TurnPromptContext};
use crate::context_metrics::ContextUsage;
use crate::session::SessionMemory;
use crate::tasks::TaskRegistry;
use crate::tool::ToolRuntime;
use crate::turn_observer::{
    emit_candidates, emit_llm_step, emit_phase_started, TurnObserver,
};

use super::prompt::build_candidate_selection_messages;

/// 候補選定用 system 指示（Plan ReAct 本体とは別）。
pub const CANDIDATE_SELECTION_SYSTEM: &str = r#"You select which registered tasks are useful for the user's goal.
Reply with ONE JSON object only (no markdown):

{"candidates":["task_id_1","task_id_2"],"reason":"<short why>"}

Rules:
- Choose 1–5 task ids from the summary catalog only.
- Prefer specialized tasks over `generic` when they fit.
- Include `generic` only when freeform tool use is needed or nothing else fits.
- For greetings / pure chit-chat with no tools: {"candidates":[],"reason":"chit-chat"}
- Do NOT emit a full plan yet. Do NOT emit thought/action steps.
"#;

/// 候補選定用: summary カタログの件数・文字数上限（汎用）。
pub const PLAN_CATALOG_SUMMARY_MAX_ENTRIES: usize = 40;
pub const PLAN_CATALOG_SUMMARY_MAX_CHARS: usize = 8_000;

/// LLM / ルール頭脳の応答から候補 id を取り出す。
pub fn parse_candidate_selection(text: &str) -> Option<Vec<String>> {
    let trimmed = text.trim();
    let json_str = extract_json_object(trimmed)?;
    let value: serde_json::Value = serde_json::from_str(json_str).ok()?;
    let arr = value.get("candidates")?.as_array()?;
    let mut out = Vec::new();
    for item in arr {
        if let Some(s) = item.as_str() {
            let id = s.trim();
            if !id.is_empty() {
                out.push(id.to_string());
            }
        }
    }
    Some(out)
}

fn extract_json_object(text: &str) -> Option<&str> {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(text) {
        if v.get("candidates").is_some() {
            return Some(text);
        }
    }
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    if end > start {
        Some(&text[start..=end])
    } else {
        None
    }
}

/// 不正・空の候補をレジストリ上の利用可能 id に正規化する。
pub fn normalize_candidates(
    raw: &[String],
    allowed: &[String],
    allow_empty_for_chitchat: bool,
) -> Vec<String> {
    let allow: HashSet<&str> = allowed.iter().map(String::as_str).collect();
    let mut out: Vec<String> = raw
        .iter()
        .filter(|id| allow.contains(id.as_str()))
        .cloned()
        .collect();
    out.sort();
    out.dedup();
    if out.is_empty() {
        if allow_empty_for_chitchat && raw.is_empty() {
            return vec![];
        }
        if allow.contains("generic") {
            return vec!["generic".into()];
        }
        if let Some(first) = allowed.first() {
            return vec![first.clone()];
        }
    }
    out
}

fn apply_no_tools_catalog(tools: &ToolRuntime, blocks: &mut PromptBlocks) {
    blocks.plan_task_catalog = Some(String::new());
    blocks.plan_selected_candidates = Some(vec![]);
    // allow 空は「全許可」なので、存在しない id だけ許可してカタログを空にする
    let no_tools = crate::tasks::SubtaskToolPolicy {
        allow: vec!["__chitchat_no_tools__".into()],
        deny: vec![],
    };
    blocks.tool_catalog = tools.format_catalog_filtered(Some(&no_tools));
}

/// 計画ループ前: summary カタログで候補を選び、詳細カタログとツールをコンテキストへ登録する。
pub fn select_and_register_plan_candidates<B: AgentBrain>(
    brain: &mut B,
    tools: &ToolRuntime,
    blocks: &mut PromptBlocks,
    session: &SessionMemory,
    user_input: &str,
    task_registry: &TaskRegistry,
    verbose: bool,
    show_prompt: bool,
    turn_observer: Option<&TurnObserver>,
    stop_requested: Option<&AtomicBool>,
) -> Vec<String> {
    select_and_register_plan_candidates_with_budget(
        brain,
        tools,
        blocks,
        session,
        user_input,
        task_registry,
        verbose,
        show_prompt,
        turn_observer,
        stop_requested,
        PLAN_CATALOG_SUMMARY_MAX_ENTRIES,
        PLAN_CATALOG_SUMMARY_MAX_CHARS,
    )
}

/// カタログ予算付き候補選定（設定から上限を渡すとき用）。
pub fn select_and_register_plan_candidates_with_budget<B: AgentBrain>(
    brain: &mut B,
    tools: &ToolRuntime,
    blocks: &mut PromptBlocks,
    session: &SessionMemory,
    user_input: &str,
    task_registry: &TaskRegistry,
    verbose: bool,
    show_prompt: bool,
    turn_observer: Option<&TurnObserver>,
    stop_requested: Option<&AtomicBool>,
    catalog_max_entries: usize,
    catalog_max_chars: usize,
) -> Vec<String> {
    let available: HashSet<String> = tools.registry().names().into_iter().collect();
    let exclude: Vec<&str> = blocks
        .plan_data_contract
        .as_ref()
        .map(|c| c.excluded_task_ids.iter().map(String::as_str).collect())
        .unwrap_or_default();
    let allowed = task_registry.available_task_ids(
        &available,
        blocks.web_search_enabled,
        &exclude,
        true,
    );
    if allowed.is_empty() {
        emit_candidates(turn_observer, "candidates", &[], false, true);
        return vec![];
    }

    let summary_catalog = task_registry.catalog_summaries_for_planner_budgeted(
        &available,
        blocks.web_search_enabled,
        &exclude,
        true,
        catalog_max_entries,
        catalog_max_chars,
    );

    blocks.candidate_selection_turn = true;
    blocks.plan_task_catalog = Some(summary_catalog);

    let cancelled = stop_requested.is_some_and(|f| f.load(Ordering::Acquire));
    if cancelled {
        blocks.candidate_selection_turn = false;
        if verbose {
            eprintln!("[plan] candidate selection cancelled before LLM");
        }
        apply_no_tools_catalog(tools, blocks);
        emit_candidates(turn_observer, "candidates", &[], false, false);
        return vec![];
    }

    emit_phase_started(
        turn_observer,
        "candidates",
        "Selecting plan task candidates",
    );

    let trace = TurnTrace::default();
    let ctx = TurnPromptContext::new(blocks, user_input, &trace, session);
    let prompt_body = if show_prompt {
        let messages = build_candidate_selection_messages(&ctx);
        let body = crate::context_metrics::format_messages_body(&messages);
        eprintln!("--- candidate-selection prompt ---\n{body}\n---");
        body
    } else {
        String::new()
    };

    let step = brain.decide(&ctx);
    blocks.candidate_selection_turn = false;

    let completion = match &step {
        AgentStep::Answer(t) | AgentStep::Thought(t) => t.clone(),
        AgentStep::Recall(t) => format!("recall: {t}"),
        AgentStep::Action(a) => format!("action: {}", a.tool),
    };
    let usage = ContextUsage::from_parts(&prompt_body, &completion, None, None);
    emit_llm_step(turn_observer, "candidates", 0, &usage, &step);

    // Phase C: Thought / unexpected / parse 失敗では全タスク許可に広げない。
    let (raw, chitchat, ok) = match &step {
        AgentStep::Answer(text) => match parse_candidate_selection(text) {
            Some(parsed) => {
                let empty = parsed.is_empty();
                (parsed, empty, true)
            }
            None => {
                if verbose {
                    eprintln!("[plan] candidate selection parse failed — no tools");
                }
                (Vec::new(), false, false)
            }
        },
        AgentStep::Thought(thought) => {
            if verbose {
                eprintln!("[plan] candidate selection thought (no-tools): {thought}");
            }
            (Vec::new(), false, false)
        }
        other => {
            if verbose {
                eprintln!("[plan] candidate selection unexpected step: {other:?}");
            }
            (Vec::new(), false, false)
        }
    };

    let selected = if ok {
        normalize_candidates(&raw, &allowed, chitchat)
    } else {
        Vec::new()
    };

    if verbose {
        eprintln!(
            "[plan] selected candidates: {}",
            if selected.is_empty() {
                if chitchat {
                    "(none — chit-chat)".into()
                } else if !ok {
                    "(none — selection failed)".into()
                } else {
                    "(none)".into()
                }
            } else {
                selected.join(", ")
            }
        );
    }

    emit_candidates(
        turn_observer,
        "candidates",
        &selected,
        chitchat && ok,
        ok,
    );

    if selected.is_empty() {
        apply_no_tools_catalog(tools, blocks);
        return vec![];
    }

    let detail = task_registry.catalog_for_candidate_ids(
        &selected,
        &available,
        blocks.web_search_enabled,
        true,
    );
    blocks.plan_task_catalog = Some(detail);
    blocks.plan_selected_candidates = Some(selected.clone());
    blocks.push_recalled(format!(
        "[plan candidates registered]\nSelected for this turn: {}\nUse only these task ids in the plan PROCEDURE.",
        selected.join(", ")
    ));

    if let Some(tool_names) = task_registry.tools_for_candidate_ids(&selected) {
        let policy = crate::tasks::SubtaskToolPolicy {
            allow: tool_names.into_iter().collect(),
            deny: vec![],
        };
        blocks.tool_catalog = tools.format_catalog_filtered(Some(&policy));
    }

    selected
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_candidates_json() {
        let v = parse_candidate_selection(
            r#"{"candidates":["list_dir","generic"],"reason":"inspect then answer"}"#,
        )
        .unwrap();
        assert_eq!(v, vec!["list_dir", "generic"]);
    }

    #[test]
    fn parses_embedded_json() {
        let v = parse_candidate_selection(
            "Here you go:\n{\"candidates\":[\"list_dir\"],\"reason\":\"x\"}\n",
        )
        .unwrap();
        assert_eq!(v, vec!["list_dir"]);
    }

    #[test]
    fn normalize_drops_unknown_and_falls_back() {
        let allowed = vec!["generic".into(), "list_dir".into()];
        let got = normalize_candidates(&["nope".into()], &allowed, false);
        assert_eq!(got, vec!["generic"]);
    }

    #[test]
    fn normalize_keeps_explicit_empty() {
        let allowed = vec!["generic".into()];
        let got = normalize_candidates(&[], &allowed, true);
        assert!(got.is_empty());
    }
}
