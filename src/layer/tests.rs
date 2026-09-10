use super::*;
use crate::brain::AgentBrain;
use crate::context::PromptBlocks;
use crate::session::SessionMemory;

struct SeqBrain {
    steps: Vec<AgentStep>,
    index: usize,
}

impl AgentBrain for SeqBrain {
    fn decide(&mut self, _ctx: &TurnPromptContext<'_>) -> AgentStep {
        let step = self
            .steps
            .get(self.index)
            .cloned()
            .unwrap_or_else(|| AgentStep::Answer("fallback".into()));
        self.index += 1;
        step
    }
}

#[test]
fn rejects_second_thought_with_loop_guard_observation() {
    let mut brain = SeqBrain {
        steps: vec![
            AgentStep::Thought("first".into()),
            AgentStep::Thought("second".into()),
            AgentStep::Answer("done".into()),
        ],
        index: 0,
    };
    let mut tools = ToolRuntime::from_registry(
        crate::runtime::RuntimeEnvironment::detect(),
        None,
        crate::tool::full_builtin_registry(false),
    );
    let mut blocks = PromptBlocks::default();
    let session = SessionMemory::default();

    let turn = run_layer_loop(
        &mut brain,
        &mut tools,
        &mut blocks,
        &session,
        "test",
        LayerLoopOptions::exec(8, 1),
        false,
        false,
        false,
        None,
        vec![],
        None,
        None,
        None,
        0,
    )
    .unwrap();

    assert_eq!(turn.answer, "done");
    assert_eq!(turn.trace.thoughts.len(), 1);
    assert_eq!(turn.trace.thoughts[0], "first");
    assert_eq!(turn.trace.actions.len(), 1);
    assert_eq!(turn.trace.actions[0].tool, THOUGHT_LIMIT_TOOL);
    assert!(turn
        .trace
        .observations
        .iter()
        .any(|o| !o.ok && o.output.contains("Thought limit reached")));
}

#[test]
fn plain_text_plan_output_falls_back_generically() {
    let mut brain = SeqBrain {
        steps: vec![AgentStep::Answer(
            "自己紹介します。私はハーネスの案内役です。".into(),
        )],
        index: 0,
    };
    let mut tools = ToolRuntime::from_registry(
        crate::runtime::RuntimeEnvironment::detect(),
        None,
        crate::tool::full_builtin_registry(false),
    );
    let mut blocks = PromptBlocks::default();
    let session = SessionMemory::default();

    let (harness, trace, steps_used) = run_plan_layer(
        &mut brain,
        &mut tools,
        &mut blocks,
        &session,
        "自己紹介して",
        4,
        false,
        false,
        false,
        false,
        None,
        None,
        None,
        0,
        &crate::tasks::TaskRegistry::builtin(),
        false,
        40,
        8000,
    )
    .unwrap();

    assert!(harness.plan.skip_execution);
    assert_eq!(harness.plan.subtasks.len(), 0);
    assert_eq!(steps_used, 1);
    assert!(trace.thoughts.is_empty());
}

#[test]
fn plan_recall_injects_memory_hits() {
    use crate::memory::{DiaryEntry, LocalDiaryBridge};

    let mut memory = LocalDiaryBridge::new();
    memory
        .diary(&DiaryEntry {
            user_input: "ファルモ導入".into(),
            summary: "事例メモ".into(),
            answer: "導入成功".into(),
            phases: vec![],
        })
        .unwrap();

    let mut brain = SeqBrain {
        steps: vec![
            AgentStep::Recall("ファルモ".into()),
            AgentStep::Answer(
                r#"{"summary":"ok","skip_execution":true,"knowledge_sufficient":true,"subtasks":[]}"#.into(),
            ),
        ],
        index: 0,
    };
    let mut tools = ToolRuntime::from_registry(
        crate::runtime::RuntimeEnvironment::detect(),
        None,
        crate::tool::full_builtin_registry(false),
    );
    let mut blocks = PromptBlocks::default();
    let session = SessionMemory::default();

    let (harness, trace, steps_used) = run_plan_layer(
        &mut brain,
        &mut tools,
        &mut blocks,
        &session,
        "続き",
        4,
        false,
        false,
        false,
        false,
        None,
        None,
        Some(&memory),
        2,
        &crate::tasks::TaskRegistry::builtin(),
        false,
        40,
        8000,
    )
    .unwrap();

    assert!(harness.plan.skip_execution);
    assert_eq!(steps_used, 2);
    assert!(blocks
        .recalled
        .iter()
        .any(|c| c.contains("plan recall") && c.contains("ファルモ")));
    assert!(trace
        .thoughts
        .iter()
        .any(|t| t.contains("recall[1/2]") && t.contains("hits=1")));
}

struct AlwaysThought;

impl AgentBrain for AlwaysThought {
    fn decide(&mut self, _ctx: &TurnPromptContext<'_>) -> AgentStep {
        AgentStep::Thought("still exploring in plan layer".into())
    }
}

#[test]
fn plan_loop_without_answer_requests_mandatory_plan_then_freeform_fallback() {
    let mut brain = AlwaysThought;
    let mut tools = ToolRuntime::from_registry(
        crate::runtime::RuntimeEnvironment::detect(),
        None,
        crate::tool::full_builtin_registry(false),
    );
    let mut blocks = PromptBlocks::default();
    let session = SessionMemory::default();

    let (harness, trace, steps_used) = run_plan_layer(
        &mut brain,
        &mut tools,
        &mut blocks,
        &session,
        "どういう改造が計画されているの？",
        4,
        false,
        false,
        false,
        false,
        None,
        None,
        None,
        2,
        &crate::tasks::TaskRegistry::builtin(),
        false,
        40,
        8000,
    )
    .unwrap();

    // 4 ステップ + 強制 answer 要求の 1 回
    assert_eq!(steps_used, 5);
    assert!(!harness.plan.skip_execution);
    assert_eq!(harness.plan.knowledge_sufficient, Some(false));
    assert_eq!(harness.plan.subtasks.len(), 1);
    assert!(harness.plan.subtasks[0].task.is_none());
    assert_eq!(
        harness.plan.subtasks[0].goal,
        "どういう改造が計画されているの？"
    );
    assert!(trace
        .thoughts
        .iter()
        .any(|t| t.contains("Emit") && t.contains("answer")));
    assert!(trace
        .thoughts
        .iter()
        .any(|t| t.contains("mandatory answer not produced")));
}

struct FinalizeAnswerBrain {
    calls: usize,
}

impl AgentBrain for FinalizeAnswerBrain {
    fn decide(&mut self, _ctx: &TurnPromptContext<'_>) -> AgentStep {
        self.calls += 1;
        if self.calls <= 4 {
            return AgentStep::Thought("not yet".into());
        }
        AgentStep::Answer(
            r#"{"summary":"ok","skip_execution":true,"knowledge_sufficient":true,"subtasks":[],"output":"done"}"#
                .into(),
        )
    }
}

#[test]
fn plan_loop_mandatory_answer_can_skip_when_sufficient() {
    let mut brain = FinalizeAnswerBrain { calls: 0 };
    let mut tools = ToolRuntime::from_registry(
        crate::runtime::RuntimeEnvironment::detect(),
        None,
        crate::tool::full_builtin_registry(false),
    );
    let mut blocks = PromptBlocks::default();
    let session = SessionMemory::default();

    let (harness, _, steps_used) = run_plan_layer(
        &mut brain,
        &mut tools,
        &mut blocks,
        &session,
        "hello",
        4,
        false,
        false,
        false,
        false,
        None,
        None,
        None,
        2,
        &crate::tasks::TaskRegistry::builtin(),
        false,
        40,
        8000,
    )
    .unwrap();

    assert_eq!(steps_used, 5);
    assert!(harness.plan.skip_execution);
    assert_eq!(harness.plan.knowledge_sufficient, Some(true));
    assert!(harness.plan.subtasks.is_empty());
}

#[test]
fn exec_loop_finalizes_instead_of_max_steps_error() {
    let mut brain = SeqBrain {
        steps: vec![
            AgentStep::Action(Action::new(
                1,
                "echo",
                serde_json::json!({ "message": "one" }),
            )),
            AgentStep::Action(Action::new(
                2,
                "echo",
                serde_json::json!({ "message": "two" }),
            )),
            AgentStep::Action(Action::new(
                3,
                "echo",
                serde_json::json!({ "message": "three" }),
            )),
            // finalize decide still refuses to answer → trace fallback
            AgentStep::Action(Action::new(
                4,
                "echo",
                serde_json::json!({ "message": "four" }),
            )),
        ],
        index: 0,
    };
    let mut tools = ToolRuntime::from_registry(
        crate::runtime::RuntimeEnvironment::detect(),
        None,
        crate::tool::full_builtin_registry(false),
    );
    let mut blocks = PromptBlocks::default();
    let session = SessionMemory::default();

    let turn = run_layer_loop(
        &mut brain,
        &mut tools,
        &mut blocks,
        &session,
        "summarize evidence",
        LayerLoopOptions::exec(3, 1),
        false,
        false,
        false,
        None,
        vec![],
        None,
        None,
        None,
        0,
    )
    .expect("exec should finalize, not MaxStepsExceeded");

    assert!(
        turn.answer.contains("step limit") || turn.answer.contains("Evidence"),
        "got: {}",
        turn.answer
    );
    assert!(turn.answer.contains("summarize evidence"));
    assert_eq!(turn.steps_used, 4);
    assert!(!turn.trace.observations.is_empty());
}
