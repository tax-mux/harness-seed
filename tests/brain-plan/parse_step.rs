//! 計画層 ReAct ステップ JSON のパース。

use harness_seed::{
    parse_plan, parse_plan_agent_step, AgentStep,
};

#[test]
fn parses_plan_thought_step() {
    let step = parse_plan_agent_step(r#"{"step":"thought","content":"分解する"}"#).unwrap();
    assert!(matches!(step, AgentStep::Thought(_)));
}

#[test]
fn action_becomes_thought_warning() {
    let step = parse_plan_agent_step(r#"{"step":"action","tool":"list_dir","args":{}}"#).unwrap();
    assert!(matches!(step, AgentStep::Thought(t) if t.contains("cannot execute")));
}

#[test]
fn picks_answer_over_thought_multiline() {
    let raw = r#"{"step":"thought","content":"The user wants an apology email."}
{"step":"answer","content":"{
  \"summary\": \"Check compose form then draft apology\",
  \"skip_execution\": false,
  \"subtasks\": [
    {\"id\": 1, \"task\": \"get_compose_form\", \"params\": {}, \"goal\": \"read form\", \"done_when\": \"success\"}
  ]
}"}"#;
    let step = parse_plan_agent_step(raw).unwrap();
    match &step {
        AgentStep::Answer(body) => {
            assert!(body.contains("get_compose_form"));
            let plan = parse_plan(body).expect("plan artifact in answer");
            assert!(!plan.skip_execution);
            assert_eq!(plan.subtasks.len(), 1);
        }
        other => panic!("expected Answer, got {other:?}"),
    }
}
