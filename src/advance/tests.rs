use std::collections::HashSet;

use crate::context::PromptBlocks;
use crate::plan::{PlanArtifact, Subtask};
use serde_json::json;

use super::phase::extract_path_like_tokens;
use super::*;

#[test]
fn recalled_progress_lists_prior_phases() {
    let plan = PlanArtifact::single_subtask("mission");
    let mut progress = AdvanceProgress::new("mission", "plan sum");
    progress.push(1, "first goal", "first answer");
    let text = format_recalled_progress(&progress, &plan, 500);
    assert!(text.contains("Phase 1 — done"));
    assert!(text.contains("first answer"));
    assert!(text.contains("Mission: mission"));
}

#[test]
fn second_phase_recalled_contains_first_answer() {
    let plan = PlanArtifact {
        summary: "two steps".into(),
        skip_execution: false,
        subtasks: vec![
            Subtask {
                id: 1,
                task: None,
                params: json!({}),
                goal: "step one".into(),
                done_when: "done".into(),
                depends_on: vec![],
            },
            Subtask {
                id: 2,
                task: None,
                params: json!({}),
                goal: "step two".into(),
                done_when: "done".into(),
                depends_on: vec![],
            },
        ],
        knowledge_sufficient: None,
        user_reply: None,
    };
    let mut progress = AdvanceProgress::new("mission", "two steps");
    progress.push(1, "step one", "answer one");
    let text = format_recalled_progress(&progress, &plan, 500);
    assert!(text.contains("answer one"));
    assert!(text.contains("Phase 1 — done"));
    assert!(text.contains("Evidence grounding"));
    assert!(text.contains("Paths:") || text.contains("Claims:") || text.contains("Goal:"));
}

#[test]
fn prepare_phase_includes_directive() {
    let plan = PlanArtifact::single_subtask("do thing");
    let progress = AdvanceProgress::default();
    let mut blocks = PromptBlocks::new();
    blocks.push_recalled("host note");
    let base = blocks.recalled.clone();
    let st = plan.subtasks[0].clone();
    prepare_phase_recalled(
        &mut blocks,
        &base,
        &progress,
        &plan,
        &st,
        &AdvanceConfig::default(),
    );
    // base was cleared and re-pushed; should have host + directive
    assert!(blocks.recalled.iter().any(|c| c.contains("host note")));
    assert!(blocks.recalled.iter().any(|c| c.contains("Current phase")));
    assert!(!blocks
        .recalled
        .iter()
        .any(|c| c.contains("Evidence grounding")));
}

#[test]
fn prepare_later_phase_includes_evidence_grounding() {
    let plan = PlanArtifact {
        summary: "two".into(),
        skip_execution: false,
        subtasks: vec![
            Subtask {
                id: 1,
                task: None,
                params: json!({}),
                goal: "gather".into(),
                done_when: "done".into(),
                depends_on: vec![],
            },
            Subtask {
                id: 2,
                task: None,
                params: json!({}),
                goal: "judge".into(),
                done_when: "done".into(),
                depends_on: vec![],
            },
        ],
        knowledge_sufficient: None,
        user_reply: None,
    };
    let mut progress = AdvanceProgress::new("mission", "two");
    progress.push(1, "gather", "saw src/lib.rs and a replan bug");
    let mut blocks = PromptBlocks::new();
    let base = Vec::new();
    prepare_phase_recalled(
        &mut blocks,
        &base,
        &progress,
        &plan,
        &plan.subtasks[1],
        &AdvanceConfig::default(),
    );
    let joined = blocks.recalled.join("\n");
    assert!(joined.contains("Evidence grounding"));
    assert!(joined.contains("Claim audit"));
    assert!(joined.contains("saw src/lib.rs"));
    assert!(joined.contains("unverified candidate"));
}

#[test]
fn prior_evidence_thinness_threshold() {
    assert!(prior_evidence_is_thin(0, 4));
    assert!(prior_evidence_is_thin(3, 4));
    assert!(!prior_evidence_is_thin(
        MIN_SUBSTANTIVE_OK_OBSERVATIONS_BEFORE_JUDGMENT,
        MIN_SUBSTANTIVE_OK_OBSERVATIONS_BEFORE_JUDGMENT
    ));
    assert!(!prior_evidence_is_thin(10, 4));
    let boost = evidence_deepening_subtask(99);
    assert!(boost.goal.contains("thin"));
    assert!(boost.done_when.contains("concrete evidence"));
    assert!(boost.goal.contains("list_dir"));
    let audit = claim_falsification_subtask(42);
    assert!(audit.goal.contains("contradictory") || audit.goal.contains("unverified"));
    assert!(audit.done_when.contains("falsified"));
    assert!(audit.goal.contains("empty") || audit.goal.contains("curl"));
    assert!(claim_audit_rules().contains("falsified"));
}

#[test]
fn prior_has_auditable_claims_detects_paths_or_claims() {
    let mut progress = AdvanceProgress::new("m", "p");
    assert!(!prior_has_auditable_claims(&progress));
    progress.push(1, "g", "no paths here just words");
    // from_answer may still extract nothing path-like
    let emptyish = !progress.steps[0].claims.is_empty() || !progress.steps[0].paths.is_empty();
    if !emptyish {
        assert!(!prior_has_auditable_claims(&progress));
    }
    progress.steps[0].paths.push("src/lib.rs".into());
    assert!(prior_has_auditable_claims(&progress));
    progress.steps[0].paths.clear();
    progress.steps[0].claims.push("something happened".into());
    assert!(prior_has_auditable_claims(&progress));
}

#[test]
fn count_ok_tool_observations_ignores_failures() {
    use crate::action::{Observation, TurnTrace};
    let mut trace = TurnTrace::default();
    trace.push_observation(Observation::success(1, "ok"));
    trace.push_observation(Observation::failure(2, "err"));
    trace.push_observation(Observation::success(3, "ok2"));
    assert_eq!(count_ok_tool_observations(&trace), 2);
}

#[test]
fn count_substantive_ok_observations_skips_list_dir() {
    use crate::action::{Action, Observation, TurnTrace};
    use serde_json::json;
    let mut trace = TurnTrace::default();
    trace.push_action(Action::new(1, "list_dir", json!({ "path": "." })));
    trace.push_observation(Observation::success(1, "a b"));
    trace.push_action(Action::new(2, "list_dir", json!({ "path": "src" })));
    trace.push_observation(Observation::success(2, "lib.rs"));
    trace.push_action(Action::new(3, "read_file", json!({ "path": "src/lib.rs" })));
    trace.push_observation(Observation::success(3, "mod"));
    trace.push_action(Action::new(4, "grep", json!({ "pattern": "Advance" })));
    trace.push_observation(Observation::failure(4, "nope"));
    assert_eq!(count_ok_tool_observations(&trace), 3);
    assert_eq!(count_substantive_ok_observations(&trace), 1);
    assert!(prior_evidence_is_thin(
        count_substantive_ok_observations(&trace),
        MIN_SUBSTANTIVE_OK_OBSERVATIONS_BEFORE_JUDGMENT
    ));
}

#[test]
fn count_substantive_ok_observations_skips_empty_run_cmd() {
    use crate::action::{Action, Observation, TurnTrace};
    use serde_json::json;
    let mut trace = TurnTrace::default();
    trace.push_action(Action::new(1, "run_cmd", json!({ "command": "curl x" })));
    trace.push_observation(Observation::success(1, ""));
    trace.push_action(Action::new(2, "run_cmd", json!({ "command": "curl y" })));
    trace.push_observation(Observation::success(2, "   \n"));
    trace.push_action(Action::new(3, "run_cmd", json!({ "command": "echo hi" })));
    trace.push_observation(Observation::success(3, "hi\n"));
    assert_eq!(count_substantive_ok_observations(&trace), 1);
    assert_eq!(count_substantive_tool_attempts(&trace), 3);
}

#[test]
fn citation_gate_marks_unsupported_paths() {
    let mut evidence = HashSet::new();
    evidence.insert("src/lib.rs".into());
    evidence.insert("src/advance.rs".into());
    let answer = "See src/lib.rs and invented/feature.rs plus src/config.rs.";
    let gated = apply_citation_gate(answer, &evidence);
    assert!(gated.contains("## Citation check"));
    assert!(gated.contains("`invented/feature.rs`"));
    assert!(gated.contains("`src/config.rs`"));
    let unverified = unverified_cited_paths(answer, &evidence);
    assert!(!unverified.iter().any(|p| p == "src/lib.rs"));
    assert!(unverified.iter().any(|p| p == "invented/feature.rs"));
    assert!(unverified.iter().any(|p| p == "src/config.rs"));
    let clean = apply_citation_gate("Only src/lib.rs matters.", &evidence);
    assert!(!clean.contains("Citation check"));
}

#[test]
fn path_tokens_ignore_japanese_headings() {
    let answer =
        "### 3. CI/CDの整備\n- Actionsの設定ファイルが見当たらない（.github/\n- see src/lib.rs";
    let paths = extract_path_like_tokens(answer);
    assert!(
        !paths
            .iter()
            .any(|p| p.contains("整備") || p.contains("見当")),
        "paths={paths:?}"
    );
    assert!(paths.iter().any(|p| p == "src/lib.rs"));
    // bare `.github/` fragment without plausible segments may or may not pass;
    // Japanese-majority tokens must not.
}

#[test]
fn absence_gate_marks_unverified_and_contradicted() {
    use crate::action::{Action, Observation, TurnTrace};
    use serde_json::json;

    let answer = "\
- `src/` 配下には `#[test]` が一切存在しない
- CONTRIBUTING.md は存在しない
";
    let empty_trace = TurnTrace::default();
    let gated = apply_absence_gate(answer, &empty_trace);
    assert!(gated.contains("## Unverified absence"));
    assert!(gated.contains("#[test]"));

    let mut hit = TurnTrace::default();
    hit.push_action(Action::new(
        1,
        "grep",
        json!({ "pattern": "#[test]", "path": "src" }),
    ));
    hit.push_observation(Observation::success(
        1,
        "src/advance.rs:900:    #[test]\nsrc/lib.rs:10:    #[test]\n",
    ));
    let contra = apply_absence_gate("- `src/` 配下には `#[test]` が一切存在しない\n", &hit);
    assert!(contra.contains("## Contradicted absence"));
    assert!(!contra.contains("## Unverified absence"));
}

#[test]
fn build_phase_note_extracts_paths_claims_and_open_questions() {
    use crate::action::{Action, Observation, TurnTrace};
    use serde_json::json;
    let mut trace = TurnTrace::default();
    trace.push_action(Action::new(1, "read_file", json!({ "path": "src/lib.rs" })));
    trace.push_observation(Observation::success(1, "pub mod plan;"));
    trace.push_action(Action::new(2, "list_dir", json!({ "path": "doc" })));
    trace.push_observation(Observation::success(2, "ja\nen"));
    let answer = "\
- README.md covers the CLI overview
- src/config.rs parses many fields
- Redis backend support is unverified
- 設定の範囲チェックは未確認
";
    let note = build_phase_note(1, "gather", answer, Some(&trace));
    assert!(note.paths.iter().any(|p| p == "src/lib.rs"));
    assert!(note.paths.iter().any(|p| p == "doc"));
    assert!(note.paths.iter().any(|p| p.contains("README.md")));
    assert!(note.tools_ok.iter().any(|t| t == "read_file"));
    assert!(note.claims.iter().any(|c| c.contains("CLI overview")));
    assert!(note
        .open_questions
        .iter()
        .any(|q| q.contains("unverified") || q.contains("未確認")));
    let formatted = note.format_structured(800);
    assert!(formatted.contains("Paths:"));
    assert!(formatted.contains("Claims:"));
    assert!(formatted.contains("Open questions:"));
}
