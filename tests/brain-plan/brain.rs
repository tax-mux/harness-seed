//! 計画層頭脳（RulePlanBrain / PlanLlmBrain）。

use harness_seed::{
    plan_artifact_from_answer, AgentBrain, AgentStep, MockLlmConnector, PlanLlmBrain,
    PromptBlocks, RulePlanBrain, SessionMemory, TaskRegistry, TurnPromptContext, TurnTrace,
};

#[test]
fn rule_plan_help_skips_in_one_step() {
    let mut brain = RulePlanBrain::new();
    let blocks = PromptBlocks::default();
    let trace = TurnTrace::default();
    let session = SessionMemory::default();
    let ctx = TurnPromptContext::new(&blocks, "help", &trace, &session);
    let step = brain.decide(&ctx);
    let answer = match step {
        AgentStep::Answer(a) => a,
        _ => panic!("expected answer"),
    };
    let plan = plan_artifact_from_answer(&answer, "help");
    assert!(plan.skip_execution);
}

#[test]
fn rule_plan_generic_two_steps() {
    let mut brain = RulePlanBrain::new();
    let blocks = PromptBlocks::default();
    let session = SessionMemory::default();
    let trace0 = TurnTrace::default();
    let ctx1 = TurnPromptContext::new(&blocks, "hello", &trace0, &session);
    assert!(matches!(
        brain.decide(&ctx1),
        AgentStep::Thought(_)
    ));

    let mut trace = TurnTrace::default();
    trace.push_thought("plan".into());
    let ctx2 = TurnPromptContext::new(&blocks, "hello", &trace, &session);
    assert!(matches!(brain.decide(&ctx2), AgentStep::Answer(_)));
}

#[test]
fn plan_llm_mock_thought_then_answer() {
    let reg = TaskRegistry::builtin();
    let mut brain = PlanLlmBrain::new(MockLlmConnector, &reg);
    let blocks = PromptBlocks::default();
    let session = SessionMemory::default();
    let s1 = brain.decide(&TurnPromptContext::new(
        &blocks,
        "do x",
        &TurnTrace::default(),
        &session,
    ));
    assert!(matches!(s1, AgentStep::Thought(_)));

    let mut trace = TurnTrace::default();
    trace.push_thought("t".into());
    let s2 = brain.decide(&TurnPromptContext::new(&blocks, "do x", &trace, &session));
    assert!(matches!(s2, AgentStep::Answer(_)));
}
