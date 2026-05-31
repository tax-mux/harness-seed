//! 計画層プロンプト（Planner 固定ゾーン）の組み立て — LLM 呼び出しなし。

use crate::action::TurnTrace;
use crate::context::{format_trace, PromptBlocks, TurnPromptContext};
use crate::context_metrics::format_messages_body;
use crate::llm::ChatMessage;
use crate::session::SessionMemory;
use crate::tasks::TaskRegistry;

use super::brain::PLAN_REACT_SYSTEM_CORE;

/// 計画層 ReAct ループ 1 ステップ目に渡すメッセージ列（LLM 未使用）。
pub fn build_plan_layer_messages(
    blocks: &PromptBlocks,
    user_input: &str,
    session: &SessionMemory,
    trace: &TurnTrace,
    task_registry: &TaskRegistry,
) -> Vec<ChatMessage> {
    let ctx = TurnPromptContext::new(blocks, user_input, trace, session);
    let catalog = plan_task_catalog_for_blocks(blocks, task_registry);
    build_plan_layer_messages_with_catalog(&ctx, &catalog)
}

/// [`TurnPromptContext`] とタスクカタログから計画層メッセージを組み立てる。
pub fn build_plan_layer_messages_with_catalog(
    ctx: &TurnPromptContext<'_>,
    task_catalog: &str,
) -> Vec<ChatMessage> {
    let mut system = String::from(PLAN_REACT_SYSTEM_CORE);
    if ctx.blocks.web_search_enabled {
        system.push_str(
            "\n- Web search is enabled: assign task `web_research` with params {\"query\":\"...\"} for external/current-events questions.\n",
        );
    }
    if !ctx.blocks.rules.is_empty() {
        system.push_str("\n\nAdditional rules:\n");
        for (i, rule) in ctx.blocks.rules.iter().enumerate() {
            system.push_str(&format!("\n[rule {}]\n{rule}\n", i + 1));
        }
    }
    if !ctx.blocks.recalled.is_empty() {
        system.push_str("\n\nRecalled context:\n");
        for (i, chunk) in ctx.blocks.recalled.iter().enumerate() {
            system.push_str(&format!("\n[recalled {}]\n{chunk}\n", i + 1));
        }
    }
    if let Some(contract) = &ctx.blocks.plan_data_contract {
        system.push_str("\n\n");
        system.push_str(&contract.format_for_planner());
    }
    system.push_str("\n\n");
    system.push_str(&format_tool_definitions_block(&ctx.blocks.tool_catalog));
    system.push_str("\n\n");
    system.push_str(&format_skills_block(task_catalog));
    system.push_str("\n\nExecution environment:\n");
    system.push_str(&ctx.blocks.runtime.prompt_hint());

    let previous = ctx.session.format_for_prompt();
    let previous_block = if previous.is_empty() {
        String::new()
    } else {
        format!("{previous}\n")
    };
    let trace_text = format_trace(ctx.trace);
    let user = format!(
        "{previous_block}ゴール:\n{}\n\nPlan trace so far:\n{trace_text}\n\nNext plan step JSON:",
        ctx.user_input
    );

    vec![ChatMessage::system(system), ChatMessage::user(user)]
}

/// Planner 固定ゾーン（system ブロック）のみ。
pub fn format_plan_fixed_zone_system(
    blocks: &PromptBlocks,
    task_registry: &TaskRegistry,
) -> String {
    let catalog = plan_task_catalog_for_blocks(blocks, task_registry);
    let empty_trace = TurnTrace::default();
    let session = SessionMemory::default();
    let ctx = TurnPromptContext::new(blocks, "", &empty_trace, &session);
    build_plan_layer_messages_with_catalog(&ctx, &catalog)
        .into_iter()
        .find(|m| m.role == "system")
        .map(|m| m.content)
        .unwrap_or_default()
}

/// 計画層 1 ステップ目プロンプトのログ用テキスト。
pub fn format_plan_layer_prompt(
    blocks: &PromptBlocks,
    user_input: &str,
    session: &SessionMemory,
    task_registry: &TaskRegistry,
) -> String {
    format_messages_body(&build_plan_layer_messages(
        blocks,
        user_input,
        session,
        &TurnTrace::default(),
        task_registry,
    ))
}

pub(crate) fn plan_task_catalog_for_blocks(blocks: &PromptBlocks, task_registry: &TaskRegistry) -> String {
    blocks.plan_task_catalog.clone().unwrap_or_else(|| {
        task_registry.catalog_for_planner_opts(blocks.web_search_enabled)
    })
}

fn format_tool_definitions_block(catalog: &str) -> String {
    if catalog_has_tool_entries(catalog) {
        format!("ツール定義:\n{}", catalog.trim())
    } else {
        "ツール定義:\n（なし）".into()
    }
}

fn format_skills_block(catalog: &str) -> String {
    if catalog_has_skill_entries(catalog) {
        format!("スキル一覧:\n{}", catalog.trim())
    } else {
        "スキル一覧:\n（なし）".into()
    }
}

pub(crate) fn catalog_has_tool_entries(catalog: &str) -> bool {
    catalog
        .lines()
        .any(|line| line.trim_start().starts_with("- "))
}

pub(crate) fn catalog_has_skill_entries(catalog: &str) -> bool {
    catalog
        .lines()
        .any(|line| line.trim_start().starts_with("- "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::TaskRegistry;

    #[test]
    fn empty_tool_and_skill_blocks_show_none() {
        assert_eq!(
            format_tool_definitions_block("Tool catalog:\n"),
            "ツール定義:\n（なし）"
        );
        assert_eq!(
            format_skills_block("Registered tasks for this session:\n"),
            "スキル一覧:\n（なし）"
        );
    }

    #[test]
    fn plan_fixed_zone_always_lists_tool_and_skill_sections() {
        let mut blocks = PromptBlocks::default();
        blocks.tool_catalog = String::new();
        blocks.plan_task_catalog = Some(String::new());
        let system = format_plan_fixed_zone_system(&blocks, &TaskRegistry::builtin());
        assert!(system.contains("ツール定義:\n（なし）"));
        assert!(system.contains("スキル一覧:\n（なし）"));
    }

    #[test]
    fn plan_fixed_zone_includes_catalogs_when_present() {
        let blocks = PromptBlocks::default();
        let system = format_plan_fixed_zone_system(&blocks, &TaskRegistry::builtin());
        assert!(system.contains("ツール定義:\n"));
        assert!(!system.contains("ツール定義:\n（なし）"));
        assert!(system.contains("スキル一覧:\n"));
        assert!(system.contains("list_dir"));
    }
}
