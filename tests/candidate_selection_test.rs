//! 候補選定 → カタログ登録の結合テスト（モック brain、LLM 不要）。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use harness_seed::{
    select_and_register_plan_candidates, AgentBrain, AgentStep, PromptBlocks, SessionMemory,
    TaskRegistry, ToolRuntime, TurnObserver, TurnPromptContext, TurnStepEvent,
};

struct ScriptedBrain {
    steps: Vec<AgentStep>,
    index: usize,
}

impl AgentBrain for ScriptedBrain {
    fn decide(&mut self, _ctx: &TurnPromptContext<'_>) -> AgentStep {
        let i = self.index.min(self.steps.len().saturating_sub(1));
        self.index += 1;
        self.steps[i].clone()
    }
}

fn tools() -> ToolRuntime {
    ToolRuntime::from_registry(
        harness_seed::RuntimeEnvironment::detect(),
        None,
        harness_seed::full_builtin_registry(false),
    )
}

#[test]
fn empty_candidates_empties_tool_catalog() {
    let mut brain = ScriptedBrain {
        steps: vec![AgentStep::Answer(
            r#"{"candidates":[],"reason":"chit-chat"}"#.into(),
        )],
        index: 0,
    };
    let tools = tools();
    let mut blocks = PromptBlocks::default();
    blocks.tool_catalog = tools.catalog();
    let before = blocks.tool_catalog.clone();
    assert!(!before.is_empty());

    let selected = select_and_register_plan_candidates(
        &mut brain,
        &tools,
        &mut blocks,
        &SessionMemory::default(),
        "こんにちは",
        &TaskRegistry::builtin(),
        false,
        false,
        None,
        None,
    );
    assert!(selected.is_empty());
    // sentinel allow-list yields empty catalog (no real tools)
    assert!(!blocks.tool_catalog.contains("run_cmd"));
    assert!(!blocks.tool_catalog.contains("list_dir"));
}

#[test]
fn selected_candidates_narrow_catalog() {
    let mut brain = ScriptedBrain {
        steps: vec![AgentStep::Answer(
            r#"{"candidates":["list_dir"],"reason":"inspect"}"#.into(),
        )],
        index: 0,
    };
    let tools = tools();
    let mut blocks = PromptBlocks::default();
    blocks.tool_catalog = tools.catalog();

    let selected = select_and_register_plan_candidates(
        &mut brain,
        &tools,
        &mut blocks,
        &SessionMemory::default(),
        "カレントのファイル一覧",
        &TaskRegistry::builtin(),
        false,
        false,
        None,
        None,
    );
    assert_eq!(selected, vec!["list_dir".to_string()]);
    assert!(blocks
        .plan_task_catalog
        .as_deref()
        .unwrap_or("")
        .contains("list_dir"));
}

#[test]
fn cancel_before_llm_yields_no_tools() {
    let mut brain = ScriptedBrain {
        steps: vec![AgentStep::Answer(
            r#"{"candidates":["list_dir"],"reason":"x"}"#.into(),
        )],
        index: 0,
    };
    let tools = tools();
    let mut blocks = PromptBlocks::default();
    blocks.tool_catalog = tools.catalog();
    let stop = Arc::new(AtomicBool::new(true));

    let selected = select_and_register_plan_candidates(
        &mut brain,
        &tools,
        &mut blocks,
        &SessionMemory::default(),
        "list files",
        &TaskRegistry::builtin(),
        false,
        false,
        None,
        Some(stop.as_ref()),
    );
    assert!(selected.is_empty());
    assert_eq!(brain.index, 0); // LLM not called
    assert!(!blocks.tool_catalog.contains("run_cmd"));
}

#[test]
fn thought_fallback_does_not_over_permit() {
    let mut brain = ScriptedBrain {
        steps: vec![AgentStep::Thought("umm".into())],
        index: 0,
    };
    let tools = tools();
    let mut blocks = PromptBlocks::default();
    blocks.tool_catalog = tools.catalog();

    let selected = select_and_register_plan_candidates(
        &mut brain,
        &tools,
        &mut blocks,
        &SessionMemory::default(),
        "do something",
        &TaskRegistry::builtin(),
        false,
        false,
        None,
        None,
    );
    assert!(selected.is_empty());
    assert!(!blocks.tool_catalog.contains("run_cmd"));
}

#[test]
fn observer_emits_phase_llm_and_candidates() {
    let events: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let events_c = events.clone();
    let observer: TurnObserver = Arc::new(move |ev: TurnStepEvent| {
        let mut g = events_c.lock().unwrap();
        match ev {
            TurnStepEvent::PhaseStarted { layer, .. } => g.push(format!("phase:{layer}")),
            TurnStepEvent::Llm { layer, .. } => g.push(format!("llm:{layer}")),
            TurnStepEvent::Candidates {
                layer,
                ids,
                chitchat,
                ok,
            } => g.push(format!(
                "candidates:{layer}:{}:chitchat={chitchat}:ok={ok}",
                ids.join(",")
            )),
            _ => {}
        }
    });

    let mut brain = ScriptedBrain {
        steps: vec![AgentStep::Answer(
            r#"{"candidates":[],"reason":"chit-chat"}"#.into(),
        )],
        index: 0,
    };
    let tools = tools();
    let mut blocks = PromptBlocks::default();

    let _ = select_and_register_plan_candidates(
        &mut brain,
        &tools,
        &mut blocks,
        &SessionMemory::default(),
        "hi",
        &TaskRegistry::builtin(),
        false,
        false,
        Some(&observer),
        None,
    );

    let got = events.lock().unwrap().clone();
    assert!(got.iter().any(|s| s == "phase:candidates"));
    assert!(got.iter().any(|s| s == "llm:candidates"));
    assert!(got
        .iter()
        .any(|s| s.starts_with("candidates:candidates:") && s.contains("chitchat=true") && s.contains("ok=true")));
}

#[test]
fn stop_flag_can_be_cleared() {
    let stop = Arc::new(AtomicBool::new(false));
    stop.store(false, Ordering::Release);
    assert!(!stop.load(Ordering::Acquire));
}
