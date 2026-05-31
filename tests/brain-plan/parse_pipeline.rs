//! 計画パースの E2E: LLM 風応答 → ReAct ステップ → HarnessState。

use harness_seed::{
    harness_state_from_plan_turn, parse_harness, parse_plan, parse_plan_agent_step, AgentStep,
    HarnessState,
};

/// `Answer` 本文（作業指示書）から Harness まで（`run_plan_layer` と同じ経路）。
fn harness_from_answer_body(answer_body: &str, user_input: &str) -> HarnessState {
    harness_state_from_plan_turn(answer_body, user_input)
}

/// LLM 生テキスト（`{"step":"answer",...}` 含む）から Harness まで。
fn harness_from_llm_raw(llm_raw: &str, user_input: &str) -> HarnessState {
    let step = parse_plan_agent_step(llm_raw).expect("plan react step should parse");
    let AgentStep::Answer(body) = step else {
        panic!("expected Answer step, got {step:?}");
    };
    harness_from_answer_body(&body, user_input)
}

fn assert_plan_matches(hs: &HarnessState, expected: &PlanExpect) {
    assert_eq!(
        hs.plan.skip_execution,
        expected.skip_execution,
        "skip_execution"
    );
    assert_eq!(hs.plan.subtasks.len(), expected.subtasks.len(), "subtask count");
    for (got, exp) in hs.plan.subtasks.iter().zip(expected.subtasks.iter()) {
        assert_eq!(got.id, exp.id, "subtask id");
        assert_eq!(got.task.as_deref(), exp.task.as_deref(), "task id");
        assert_eq!(got.goal, exp.goal, "goal");
        if let Some(done) = &exp.done_when_contains {
            assert!(
                got.done_when.contains(done),
                "done_when should contain {done:?}, got {}",
                got.done_when
            );
        }
    }
    assert_eq!(hs.total_steps, expected.total_steps, "total_steps");
    if !expected.summary_contains.is_empty() {
        assert!(
            hs.plan.summary.contains(&expected.summary_contains),
            "summary should contain {:?}, got {}",
            expected.summary_contains,
            hs.plan.summary
        );
    }
}

struct SubtaskExpect {
    id: u32,
    task: Option<&'static str>,
    goal: &'static str,
    done_when_contains: Option<&'static str>,
}

struct PlanExpect {
    skip_execution: bool,
    total_steps: u32,
    summary_contains: &'static str,
    subtasks: Vec<SubtaskExpect>,
}

// --- 典型: Mock LLM と同じ input/steps/output 経路 ---

#[test]
fn pipeline_input_steps_output_flow() {
    let body = r#"{
        "input": ["read: revision_context (UID 9)"],
        "steps": [
            {"id": 1, "task": "generic", "params": {}, "goal": "日本語化", "done_when": "件名本文を返した"}
        ],
        "output": "write: outgoing_pending_db (UID 9)",
        "skip_execution": false
    }"#;
    let hs = harness_from_answer_body(body, "user");
    assert_plan_matches(
        &hs,
        &PlanExpect {
            skip_execution: false,
            total_steps: 1,
            summary_contains: "outgoing_pending_db",
            subtasks: vec![SubtaskExpect {
                id: 1,
                task: Some("generic"),
                goal: "日本語化",
                done_when_contains: Some("件名"),
            }],
        },
    );
}

// --- 典型: subtasks 形式 + ReAct ラッパ ---

#[test]
fn pipeline_mock_style_llm_response() {
    let llm = r#"{"step":"thought","content":"分解"}
{"step":"answer","content":"{\"summary\":\"mock plan\",\"skip_execution\":false,\"subtasks\":[{\"id\":1,\"task\":\"list_dir\",\"params\":{\"path\":\"src\"},\"goal\":\"\",\"done_when\":\"listed\"},{\"id\":2,\"goal\":\"summarize\",\"done_when\":\"ok\"}]}"}"#;
    let hs = harness_from_llm_raw(llm, "do something");
    assert_plan_matches(
        &hs,
        &PlanExpect {
            skip_execution: false,
            total_steps: 2,
            summary_contains: "mock plan",
            subtasks: vec![
                SubtaskExpect {
                    id: 1,
                    task: Some("list_dir"),
                    goal: "",
                    done_when_contains: Some("listed"),
                },
                SubtaskExpect {
                    id: 2,
                    task: None,
                    goal: "summarize",
                    done_when_contains: Some("ok"),
                },
            ],
        },
    );
}

// --- markdown フェンス付き JSON ---

#[test]
fn pipeline_json_in_markdown_fence() {
    let body = r#"```json
{
  "summary": "fenced plan",
  "skip_execution": false,
  "subtasks": [{"id": 1, "goal": "only step", "done_when": "done"}]
}
```"#;
    let hs = harness_from_answer_body(body, "fallback");
    assert_eq!(hs.total_steps, 1);
    assert_eq!(hs.plan.subtasks[0].goal, "only step");
}

// --- 番号付き作業指示書（JSON なし）---

#[test]
fn pipeline_numbered_work_instructions() {
    let body = "作業指示書\n1. get_email で参照メールを読む\n2. generic で下書きを作成する\n";
    let hs = harness_from_answer_body(body, "fallback");
    assert_eq!(hs.total_steps, 2);
    assert_eq!(hs.plan.subtasks[0].goal, "get_email で参照メールを読む");
    assert_eq!(hs.plan.subtasks[1].goal, "generic で下書きを作成する");
    assert_eq!(hs.plan.subtasks[0].id, 1);
    assert_eq!(hs.plan.subtasks[1].id, 2);
}

#[test]
fn pipeline_step_prefix_japanese() {
    let body = "ステップ1: フォームを読む\nステップ2: 本文を書く\n";
    let hs = harness_from_answer_body(body, "fallback");
    assert_eq!(hs.total_steps, 2);
    assert_eq!(hs.plan.subtasks[0].goal, "フォームを読む");
}

// --- skip_execution ---

#[test]
fn pipeline_skip_execution_no_steps() {
    let body = r#"{"summary":"hi","skip_execution":true,"subtasks":[]}"#;
    let hs = harness_from_answer_body(body, "hello");
    assert!(hs.plan.skip_execution);
    assert_eq!(hs.total_steps, 0);
    assert_eq!(hs.current_step, 0);
}

// --- パース失敗時はフォールバック（意図を明示）---

#[test]
fn pipeline_invalid_json_single_line_falls_back_to_user_input() {
    let body = "Please list files in src and summarize.";
    let hs = harness_from_answer_body(body, "list and summarize");
    assert_eq!(hs.total_steps, 1);
    assert_eq!(hs.plan.subtasks[0].goal, "list and summarize");
    assert!(!hs.plan.skip_execution);
}

// --- parse_plan / parse_harness 直叩き一致 ---

#[test]
fn parse_plan_and_harness_agree_on_same_body() {
    let body = r#"{
        "summary": "sync",
        "skip_execution": false,
        "subtasks": [
            {"id": 1, "task": "mail_read", "params": {"uid": 42}, "goal": "", "done_when": "read"}
        ]
    }"#;
    let plan = parse_plan(body).expect("parse_plan");
    let hs = parse_harness(body, "user").expect("parse_harness");
    assert_eq!(plan, hs.plan);
    assert_eq!(hs.work_instructions, body);
}

// --- 壊れた JSON は Err、Harness はフォールバック ---

#[test]
fn parse_plan_rejects_invalid_json() {
    let err = parse_plan(r#"{"summary": "x", "subtasks": [}"#).unwrap_err();
    assert!(err.to_string().contains("invalid JSON"));
}

#[test]
fn harness_empty_input_errors() {
    let err = parse_harness("  ", "user").unwrap_err();
    assert!(err.to_string().contains("empty"));
}
