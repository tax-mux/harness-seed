use super::*;
use crate::brain::SimpleRuleBrain;
use crate::context::TurnPromptContext;

#[test]
fn help_turn_single_step() {
    let mut react = ReActLoop::with_defaults(SimpleRuleBrain::new());
    let result = react.run_turn("help").unwrap();
    assert_eq!(result.steps_used, 1);
    assert!(result.answer.contains("echo"));
}

#[test]
fn generic_input_runs_thought_echo_answer() {
    let mut react = ReActLoop::with_defaults(SimpleRuleBrain::new());
    let result = react.run_turn("hello world").unwrap();
    assert_eq!(result.steps_used, 3);
    assert_eq!(result.trace.thoughts.len(), 1);
    assert_eq!(result.trace.actions.len(), 1);
    assert!(result.answer.contains("hello world"));
}

#[test]
fn echo_command_skips_thought() {
    let mut react = ReActLoop::with_defaults(SimpleRuleBrain::new());
    let result = react.run_turn("echo ping").unwrap();
    assert_eq!(result.steps_used, 2);
    assert!(result.trace.thoughts.is_empty());
    assert!(result.answer.contains("ping"));
}

#[test]
fn blocks_recalled_visible_in_llm_system_when_rendered() {
    let mut blocks = PromptBlocks::new();
    blocks.push_recalled("note from host");
    let trace = TurnTrace::default();
    let session = SessionMemory::default();
    let ctx = TurnPromptContext::new(&blocks, "hi", &trace, &session);
    let system = ctx
        .render()
        .into_iter()
        .find(|m| m.role == "system")
        .expect("system");
    assert!(system.content.as_text().contains("note from host"));
}

#[test]
fn session_accumulates_completed_turns() {
    let mut react = ReActLoop::with_defaults(SimpleRuleBrain::new());
    react.run_turn("help").unwrap();
    react.run_turn("help").unwrap();
    assert_eq!(react.session.len(), 2);
    react
        .session
        .set_prompt_policy(SessionPromptPolicy::IncludePrior);
    assert!(react.session.format_for_prompt().contains("利用可能"));
}

#[test]
fn two_phase_help_still_single_exec() {
    let mut config = ReActConfig::default();
    config.two_phase = true;
    let mut react = ReActLoop::new(SimpleRuleBrain::new(), PlanBrainMode::rule(), config);
    let result = react.run_turn("help").unwrap();
    // 計画層 1 + 実行層 1（ルール頭脳は context_usages なし）
    assert_eq!(result.steps_used, 2);
    assert!(result.plan.as_ref().unwrap().skip_execution);
    assert!(result.answer.contains("echo"));
}

#[test]
fn two_phase_generic_runs_subtask_mission() {
    let mut config = ReActConfig::default();
    config.two_phase = true;
    let mut react = ReActLoop::new(SimpleRuleBrain::new(), PlanBrainMode::rule(), config);
    let result = react.run_turn("hello world").unwrap();
    assert_eq!(result.subtask_results.len(), 1);
    assert_eq!(result.subtask_results[0].id, 1);
    assert_eq!(result.steps_used, 5);
    assert!(!result.subtask_results[0].used_step_driver);
    assert!(result.answer.contains("hello world"));
}

#[test]
fn lifecycle_panic_does_not_abort_turn() {
    use crate::lifecycle::{HostView, TurnLifecycle};

    struct Boom;
    impl TurnLifecycle for Boom {
        fn on_plan_finished(&self, _: &str, _: &PlanArtifact, _: HostView<'_>) {
            panic!("host hook exploded");
        }
    }

    let mut config = ReActConfig::default();
    config.two_phase = true;
    let mut react = ReActLoop::new(SimpleRuleBrain::new(), PlanBrainMode::rule(), config);
    react.set_lifecycle(Some(Arc::new(Boom)));
    let result = react.run_turn("hello world").unwrap();
    assert!(result.answer.contains("hello world"));
}

#[test]
fn lifecycle_hooks_fire_without_changing_answer() {
    use crate::lifecycle::{HostScratch, HostView, TurnLifecycle};
    use std::sync::Mutex;

    #[derive(Default)]
    struct Rec {
        events: Mutex<Vec<String>>,
    }
    impl TurnLifecycle for Rec {
        fn on_turn_started(&self, _: &str, host: HostView<'_>) {
            let ticket = host.turn_get_i64("ticket_id").unwrap_or(-1);
            self.events
                .lock()
                .unwrap()
                .push(format!("turn_started:{ticket}"));
        }
        fn on_plan_finished(&self, _: &str, _: &PlanArtifact, mut host: HostView<'_>) {
            host.insert("parent_ticket", 42);
            self.events.lock().unwrap().push("plan_finished".into());
        }
        fn on_subtask_started(
            &self,
            _: &str,
            _: &PlanArtifact,
            subtask: &Subtask,
            _: usize,
            mut host: HostView<'_>,
        ) {
            let parent = host.turn_get_i64("parent_ticket").unwrap_or(-1);
            host.insert("child_ticket", 7);
            self.events
                .lock()
                .unwrap()
                .push(format!("subtask_started:{}:{parent}", subtask.id));
        }
        fn on_subtask_finished(
            &self,
            _: &str,
            _: &PlanArtifact,
            subtask: &Subtask,
            outcome: &crate::lifecycle::SubtaskOutcome,
            host: HostView<'_>,
        ) {
            let child = host.get_i64("child_ticket").unwrap_or(-1);
            self.events.lock().unwrap().push(format!(
                "subtask_finished:{}:{child}:{:?}",
                subtask.id, outcome.status
            ));
        }
        fn on_turn_finished(
            &self,
            _: &str,
            _: Option<&PlanArtifact>,
            outcome: &crate::lifecycle::TurnOutcome,
            host: HostView<'_>,
        ) {
            let parent = host.turn_get_i64("parent_ticket").unwrap_or(-1);
            let child = host.subtask_get_i64(1, "child_ticket").unwrap_or(-1);
            self.events.lock().unwrap().push(format!(
                "turn_finished:{parent}:{child}:{:?}",
                outcome.status
            ));
        }
    }

    let rec = Arc::new(Rec::default());
    let mut config = ReActConfig::default();
    config.two_phase = true;
    let mut react = ReActLoop::new(SimpleRuleBrain::new(), PlanBrainMode::rule(), config);
    react.set_lifecycle(Some(rec.clone()));
    let mut seed = HostScratch::new();
    seed.turn_insert("ticket_id", 10);
    react.seed_host_scratch(seed);
    let result = react.run_turn("hello world").unwrap();
    assert!(result.answer.contains("hello world"));
    assert_eq!(
        rec.events.lock().unwrap().as_slice(),
        [
            "turn_started:10",
            "plan_finished",
            "subtask_started:1:42",
            "subtask_finished:1:7:Completed",
            "turn_finished:42:7:Completed",
        ]
    );
    assert_eq!(react.host_scratch().turn_get_i64("ticket_id"), Some(10));
    assert_eq!(react.host_scratch().turn_get_i64("parent_ticket"), Some(42));
    assert_eq!(
        react.host_scratch().subtask_get_i64(1, "child_ticket"),
        Some(7)
    );
    let json = react.host_scratch().to_value();
    assert_eq!(json["turn"]["parent_ticket"], 42);
    assert_eq!(json["subtasks"]["1"]["child_ticket"], 7);
}

#[test]
fn lifecycle_emits_cancelled_turn_on_abort() {
    use crate::lifecycle::{HostView, TurnLifecycle};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;

    struct Rec {
        events: Mutex<Vec<String>>,
        stop: Arc<AtomicBool>,
    }
    impl TurnLifecycle for Rec {
        fn on_plan_finished(&self, _: &str, _: &PlanArtifact, _: HostView<'_>) {
            self.events.lock().unwrap().push("plan_finished".into());
            self.stop.store(true, Ordering::Relaxed);
        }
        fn on_turn_finished(
            &self,
            _: &str,
            _: Option<&PlanArtifact>,
            outcome: &crate::lifecycle::TurnOutcome,
            _: HostView<'_>,
        ) {
            self.events
                .lock()
                .unwrap()
                .push(format!("turn_finished:{:?}", outcome.status));
        }
    }

    let stop = Arc::new(AtomicBool::new(false));
    let rec = Arc::new(Rec {
        events: Mutex::new(Vec::new()),
        stop: stop.clone(),
    });
    let mut config = ReActConfig::default();
    config.two_phase = true;
    config.show_plan = false;
    config.show_task_execution = false;
    let mut react = ReActLoop::new(SimpleRuleBrain::new(), PlanBrainMode::rule(), config);
    react.set_lifecycle(Some(rec.clone()));
    react.set_stop_requested(Some(stop));
    let err = react.run_turn("hello world").unwrap_err();
    assert_eq!(err, ReActError::Cancelled);
    assert_eq!(
        rec.events.lock().unwrap().as_slice(),
        ["plan_finished", "turn_finished:Cancelled"]
    );
}

#[test]
fn advance_enabled_runs_single_phase_with_rule_brain() {
    let mut config = ReActConfig::default();
    config.advance.mode = crate::advance::AdvanceMode::Always;
    config.advance.max_phases = 1;
    config.advance.show_phases = false;
    config.show_plan = false;
    config.show_task_execution = false;
    let mut react = ReActLoop::new(SimpleRuleBrain::new(), PlanBrainMode::rule(), config);
    let result = react.run_turn("hello world").unwrap();
    assert_eq!(result.advance_phases.len(), 1);
    assert_eq!(result.advance_phases[0].id, 1);
    assert!(result.answer.contains("hello world"));
}

#[test]
fn plan_preview_runs_plan_layer_only() {
    let mut react = ReActLoop::new(
        SimpleRuleBrain::new(),
        PlanBrainMode::rule(),
        ReActConfig::default(),
    );
    let preview = react.run_plan_preview("hello world").unwrap();
    assert!(!preview.planner_text.is_empty());
    assert_eq!(preview.harness.plan.subtasks.len(), 1);
    assert!(preview.steps_used >= 1);
}

#[test]
fn local_memory_survives_across_turns_without_panic() {
    use crate::memory::LocalDiaryBridge;
    let mut config = ReActConfig::default();
    config.advance.mode = crate::advance::AdvanceMode::Always;
    config.advance.show_phases = false;
    config.show_plan = false;
    config.show_task_execution = false;
    let mut react = ReActLoop::new(SimpleRuleBrain::new(), PlanBrainMode::rule(), config);
    react.set_memory_bridge(Box::new(LocalDiaryBridge::new()));
    react.run_turn("echo first-unique-token").unwrap();
    let second = react.run_turn("続きやって").unwrap();
    assert!(!second.answer.is_empty());
    // host recalled はターン終了後に復元される
    assert!(react.blocks.recalled.is_empty());
}

#[test]
fn answer_looks_user_ready_accepts_plain_sentence() {
    assert!(synthesis::answer_looks_user_ready("実装可能です。"));
    assert!(synthesis::answer_looks_user_ready("  hello world  "));
}

#[test]
fn answer_looks_user_ready_rejects_structured_or_multiline() {
    assert!(!synthesis::answer_looks_user_ready(""));
    assert!(!synthesis::answer_looks_user_ready("line1\nline2"));
    assert!(!synthesis::answer_looks_user_ready(r#"{"step":"answer"}"#));
    assert!(!synthesis::answer_looks_user_ready("a\tb"));
    assert!(!synthesis::answer_looks_user_ready("[a, b]"));
}

#[test]
fn needs_user_answer_synthesis_skips_when_driver_answer_is_ready() {
    let results = vec![SubtaskExecResult {
        id: 1,
        answer: "一覧を取得しました。".into(),
        steps_used: 1,
        used_step_driver: true,
    }];
    assert!(!ReActLoop::<SimpleRuleBrain>::needs_user_answer_synthesis(
        &results
    ));
}

#[test]
fn needs_user_answer_synthesis_when_driver_output_is_raw() {
    let results = vec![SubtaskExecResult {
        id: 1,
        answer: "README.md\nCargo.toml\nsrc/".into(),
        steps_used: 1,
        used_step_driver: true,
    }];
    assert!(ReActLoop::<SimpleRuleBrain>::needs_user_answer_synthesis(
        &results
    ));
}

#[test]
fn build_synthesis_evidence_caps_total_chars() {
    let results = vec![
        SubtaskExecResult {
            id: 1,
            answer: "a".repeat(500),
            steps_used: 1,
            used_step_driver: true,
        },
        SubtaskExecResult {
            id: 2,
            answer: "b".repeat(500),
            steps_used: 1,
            used_step_driver: true,
        },
    ];
    let evidence = synthesis::build_synthesis_evidence(&results, &[], 600, 400);
    assert!(evidence.chars().count() <= 401);
}

fn three_subtask_plan_json() -> &'static str {
    r#"{"summary":"three","skip_execution":false,"knowledge_sufficient":false,"subtasks":[{"id":1,"goal":"a","done_when":"user request satisfied"},{"id":2,"goal":"b","done_when":"user request satisfied"},{"id":3,"goal":"c","done_when":"user request satisfied"}]}"#
}

fn from_plan_config() -> ReActConfig {
    let mut config = ReActConfig::default();
    config.advance.mode = crate::advance::AdvanceMode::FromPlan;
    config.advance.show_phases = false;
    config.advance.claim_check = false;
    config.advance.citation_check = false;
    config.advance.absence_check = false;
    config.show_plan = false;
    config.show_task_execution = false;
    config.show_tool_output = false;
    config.plan_candidate_selection = false;
    config
}

#[test]
fn from_plan_single_subtask_stays_two_phase() {
    let config = from_plan_config();
    let mut react = ReActLoop::new(SimpleRuleBrain::new(), PlanBrainMode::rule(), config);
    let result = react.run_turn("hello world").unwrap();
    assert!(result.advance_phases.is_empty());
    assert_eq!(result.subtask_results.len(), 1);
}

#[test]
fn from_plan_three_subtasks_escalates_without_replanning() {
    use crate::lifecycle::{HostView, TurnLifecycle};
    use std::sync::{Arc, Mutex};

    struct PlanFinishCounter {
        n: Mutex<usize>,
    }
    impl TurnLifecycle for PlanFinishCounter {
        fn on_plan_finished(&self, _: &str, _: &PlanArtifact, _: HostView<'_>) {
            *self.n.lock().unwrap() += 1;
        }
    }

    let config = from_plan_config();
    let mut react = ReActLoop::new(
        SimpleRuleBrain::new(),
        PlanBrainMode::fixed_plan(three_subtask_plan_json()),
        config,
    );
    let counter = Arc::new(PlanFinishCounter {
        n: Mutex::new(0),
    });
    react.set_lifecycle(Some(counter.clone()));
    let result = react.run_turn("hello world").unwrap();
    assert_eq!(*counter.n.lock().unwrap(), 1);
    assert!(!result.advance_phases.is_empty());
}

#[test]
fn off_does_not_escalate_three_subtasks() {
    let mut config = from_plan_config();
    config.two_phase = true;
    config.advance.mode = crate::advance::AdvanceMode::Off;
    let mut react = ReActLoop::new(
        SimpleRuleBrain::new(),
        PlanBrainMode::fixed_plan(three_subtask_plan_json()),
        config,
    );
    let result = react.run_turn("hello world").unwrap();
    assert!(result.advance_phases.is_empty());
    assert_eq!(result.subtask_results.len(), 3);
}
