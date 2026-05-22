//! プロンプトブロックと ReAct ループの統合。

use harness_seed::{
    PlanBrainMode, PromptBlocks, ReActLoop, SimpleRuleBrain, TurnPromptContext,
    REACT_SYSTEM_CORE,
};

#[test]
fn react_loop_exposes_blocks_for_external_rules() {
    let mut blocks = PromptBlocks::new();
    blocks.push_recalled("external fact: 42");
    let mut react = ReActLoop::with_blocks(
        SimpleRuleBrain::new(),
        PlanBrainMode::rule(),
        Default::default(),
        blocks,
    );
    react.run_turn("help").unwrap();
    assert!(react.blocks.recalled[0].contains("42"));
}

#[test]
fn turn_prompt_context_render_matches_core() {
    let blocks = PromptBlocks::default();
    let trace = harness_seed::TurnTrace::default();
    let session = harness_seed::SessionMemory::default();
    let ctx = TurnPromptContext::new(&blocks, "test", &trace, &session);
    let messages = ctx.render();
    let system = messages
        .iter()
        .find(|m| m.role == "system")
        .expect("system");
    assert!(system.content.starts_with(REACT_SYSTEM_CORE));
    assert!(system.content.contains("Tool catalog:"));
}
