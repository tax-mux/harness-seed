//! 計画層・実行層向けプロンプト（作業指示書 / 現在ステップ / Planner 固定ゾーン）。

use harness_seed::{
    format_plan_fixed_zone_system, format_plan_layer_prompt, format_plan_rule_prompt_preview,
    HarnessState, PlanArtifact, PromptBlocks, SessionMemory, TaskRegistry, TurnPromptContext,
    TurnTrace,
};

#[test]
fn plan_rule_prompt_preview_includes_plan_request() {
    let blocks = PromptBlocks::default();
    let trace = TurnTrace::default();
    let session = SessionMemory::default();
    let ctx = TurnPromptContext::new(&blocks, "list src", &trace, &session);
    let body = format_plan_rule_prompt_preview(&ctx);
    assert!(body.contains("ゴール:"));
    assert!(body.contains("list src"));
}

#[test]
fn plan_fixed_zone_includes_task_catalog_and_core() {
    let blocks = PromptBlocks::default();
    let reg = TaskRegistry::builtin();
    let system = format_plan_fixed_zone_system(&blocks, &reg);
    assert!(system.contains("planning agent"));
    assert!(system.contains("ツール定義:"));
    assert!(system.contains("スキル一覧:"));
    assert!(system.contains("list_dir"));
    assert!(system.contains("Execution environment"));
}

#[test]
fn plan_layer_prompt_includes_plan_request() {
    let blocks = PromptBlocks::default();
    let session = SessionMemory::default();
    let reg = TaskRegistry::builtin();
    let body = format_plan_layer_prompt(&blocks, "list src", &session, &reg);
    assert!(body.contains("ゴール:"));
    assert!(body.contains("list src"));
    assert!(body.contains("Next plan step JSON"));
}

#[test]
fn exec_render_includes_work_instructions_and_current_step() {
    let plan = PlanArtifact::single_subtask("do thing");
    let hs = HarnessState::new("1. Do the thing", plan);
    let reg = TaskRegistry::builtin();

    let mut blocks = PromptBlocks::default();
    blocks.work_instructions_text = Some(hs.format_work_instructions_for_prompt());
    blocks.current_step_text = Some(hs.format_current_step_for_prompt(&reg));

    let trace = TurnTrace::default();
    let session = SessionMemory::default();
    let ctx = TurnPromptContext::new(&blocks, "mission", &trace, &session);
    let system = ctx
        .render()
        .into_iter()
        .find(|m| m.role == "system")
        .expect("system");
    assert!(system.content.contains("Work instructions (from planner"));
    assert!(system.content.contains("Do the thing"));
    assert!(system.content.contains("Current step (harness"));
    assert!(system.content.contains("Step 1/1"));
}
