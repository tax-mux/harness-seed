use super::*;
use crate::harness::HarnessState;
use crate::plan::PlanArtifact;

#[test]
fn plan_zone_uses_diagram_japanese_section_titles() {
    let blocks = PromptBlocks::default();
    let reg = TaskRegistry::builtin();
    let hs = HarnessState::new("1. step", PlanArtifact::single_subtask("do"));
    let text = format_plan_zone_after_preview(&blocks, &reg, "フォルダ一覧", "{}", &hs);
    assert!(text.contains("### Phase 1　計画フェーズ ###"));
    assert!(text.contains("### ゴール ###"));
    assert!(text.contains("### Planner固定ゾーン ###"));
    assert!(text.contains("### ツール定義 ###"));
    assert!(text.contains("### スキル一覧 ###"));
    assert!(text.contains("### 参照情報 ###"));
    assert!(text.contains("### 作業指示書 ###"));
    assert!(text.contains("### Harness内部状態（JSON） ###"));
    assert!(!text.contains("Plan request"));
    assert!(!text.contains("Planner fixed zone (system)"));
}

#[test]
fn empty_tool_skill_mail_show_nashi() {
    let mut blocks = PromptBlocks::default();
    blocks.tool_catalog.clear();
    blocks.plan_task_catalog = Some(String::new());
    blocks.recalled.clear();
    let sections = planner_fixed_zone_sections(&blocks, &TaskRegistry::builtin(), None);
    assert_eq!(sections.tool_definitions, "（なし）");
    assert_eq!(sections.skills, "（なし）");
    assert_eq!(sections.reference_info, "（なし）");
}

#[test]
fn planner_fixed_zone_html_escapes_body() {
    let mut blocks = PromptBlocks::default();
    blocks
        .rules
        .push("allow <tag> & \"quote\" 'single'".to_string());
    let html = format_planner_fixed_zone_html(
        &blocks,
        &TaskRegistry::builtin(),
        None,
        Some("allow <tag> & \"quote\" 'single'"),
        None,
        None,
        None,
        &[],
        None,
        &[],
    );
    assert!(html.contains("&lt;tag&gt; &amp; &quot;quote&quot;"));
    assert!(html.contains("&#39;single&#39;"));
    assert!(!html.contains("<tag>"));
}

#[test]
fn planner_output_json_is_pretty_formatted() {
    let blocks = PromptBlocks::default();
    let html = format_planner_fixed_zone_html(
        &blocks,
        &TaskRegistry::builtin(),
        None,
        Some("{\"a\":1,\"nested\":{\"b\":2}}"),
        None,
        None,
        None,
        &[],
        None,
        &[],
    );
    assert!(html.contains("\n  &quot;a&quot;: 1,"));
    assert!(html.contains("\n  &quot;nested&quot;: {"));
}

#[test]
fn harness_state_is_embedded_in_html() {
    let blocks = PromptBlocks::default();
    let harness = HarnessState::new("1. step", PlanArtifact::single_subtask("do"));
    let html = format_planner_fixed_zone_html(
        &blocks,
        &TaskRegistry::builtin(),
        Some(&harness),
        Some("{}"),
        None,
        None,
        None,
        &[],
        None,
        &[],
    );
    assert!(html.contains("Harness内部状態（JSON）"));
    assert!(html.contains("&quot;current_step&quot;"));
}

#[test]
fn harness_state_after_mini_planner_is_embedded_in_html() {
    let blocks = PromptBlocks::default();
    let harness = HarnessState::new(
        "{}",
        PlanArtifact {
            summary: "single task".into(),
            skip_execution: false,
            subtasks: vec![crate::plan::Subtask {
                id: 1,
                task: Some("list_dir".into()),
                params: serde_json::json!({}),
                goal: "dir".into(),
                done_when: "done".into(),
                depends_on: vec![],
            }],
            knowledge_sufficient: None,
            user_reply: None,
        },
    );
    let html = format_planner_fixed_zone_html(
        &blocks,
        &TaskRegistry::builtin(),
        Some(&harness),
        Some("{}"),
        None,
        None,
        None,
        &[],
        None,
        &[],
    );
    assert!(html.contains("Harness内部状態（ミニPlanner適用後）"));
    assert!(html.contains("&quot;tool_set&quot;: ["));
    assert!(html.contains("&quot;list_dir&quot;"));
}

#[test]
fn html_wraps_sections_with_phase_accordions() {
    let blocks = PromptBlocks::default();
    let html = format_planner_fixed_zone_html(
        &blocks,
        &TaskRegistry::builtin(),
        None,
        Some("{}"),
        None,
        None,
        None,
        &[],
        None,
        &[],
    );
    assert!(html.contains("<summary>時系列トレース（Phase 1 → 2 → 3）</summary>"));
    assert!(html.contains("<summary>Phase 1 詳細ログ</summary>"));
    assert!(html.contains("<summary>Phase 2 詳細ログ</summary>"));
    assert!(html.contains("<summary>Phase 3 詳細ログ</summary>"));
}

#[test]
fn phase3_tasks_are_embedded_in_html() {
    let blocks = PromptBlocks::default();
    let harness = HarnessState::new(
        "{}",
        PlanArtifact {
            summary: "single task".into(),
            skip_execution: false,
            subtasks: vec![crate::plan::Subtask {
                id: 1,
                task: Some("list_dir".into()),
                params: serde_json::json!({}),
                goal: "dir".into(),
                done_when: "done".into(),
                depends_on: vec![],
            }],
            knowledge_sufficient: None,
            user_reply: None,
        },
    );
    let html = format_planner_fixed_zone_html(
        &blocks,
        &TaskRegistry::builtin(),
        Some(&harness),
        Some("{}"),
        None,
        None,
        None,
        &[],
        None,
        &[],
    );
    assert!(html.contains("タスク実行プラン"));
    assert!(html.contains("<details class=\"subtask-zone\" open>"));
    assert!(html.contains("今のステップ（Harnessがテキスト変換）"));
    assert!(html.contains("スキーマ・ツール定義（ステップ別）"));
    assert!(!html.contains("&lt;details class=&quot;subtask-zone&quot;"));
}

#[test]
fn compressed_and_recent_zones_are_embedded_in_html() {
    let blocks = PromptBlocks::default();
    let compressed = vec!["phase1 summary".to_string(), "phase2 summary".to_string()];
    let recent = "Previous turns:\n[turn 1]\nUser: hi\nAssistant: hello";
    let html = format_planner_fixed_zone_html(
        &blocks,
        &TaskRegistry::builtin(),
        None,
        Some("{}"),
        None,
        None,
        None,
        &compressed,
        Some(recent),
        &[],
    );
    assert!(html.contains("圧縮ゾーン"));
    assert!(html.contains("[recalled 1]"));
    assert!(html.contains("phase2 summary"));
    assert!(html.contains("直近ゾーン"));
    assert!(html.contains("Previous turns:"));
    assert!(html.contains("User: hi"));
}

#[test]
fn phase3_subtask_mode_badges_are_embedded_in_html() {
    let blocks = PromptBlocks::default();
    let harness = HarnessState::new(
        "{}",
        PlanArtifact {
            summary: "single task".into(),
            skip_execution: false,
            subtasks: vec![crate::plan::Subtask {
                id: 1,
                task: Some("list_dir".into()),
                params: serde_json::json!({}),
                goal: "dir".into(),
                done_when: "done".into(),
                depends_on: vec![],
            }],
            knowledge_sufficient: None,
            user_reply: None,
        },
    );
    let html = format_planner_fixed_zone_html(
        &blocks,
        &TaskRegistry::builtin(),
        Some(&harness),
        Some("{}"),
        None,
        None,
        None,
        &[],
        None,
        &[(1, true)],
    );
    assert!(html.contains("実行モード"));
    assert!(html.contains("mode-driver"));
    assert!(html.contains("step-driver"));
}
#[test]
fn top_snapshot_is_embedded_in_html() {
    use crate::action::{Action, Observation, TurnTrace};

    let blocks = PromptBlocks::default();
    let mut trace = TurnTrace::default();
    trace.push_thought("considered the latest input".into());
    trace.push_action(Action::new(
        1,
        "grep",
        serde_json::json!({"pattern": "README"}),
    ));
    trace.push_observation(Observation::success(1, "README.md"));

    let harness = HarnessState::new("{}", PlanArtifact::single_subtask("do"));
    let html = format_planner_fixed_zone_html(
        &blocks,
        &TaskRegistry::builtin(),
        Some(&harness),
        Some("{}"),
        Some("最新の入力"),
        Some(&crate::context_metrics::TurnContextSummary::default()),
        Some(&trace),
        &[],
        None,
        &[],
    );
    assert!(html.contains("今回の入力と内部状態"));
    assert!(html.contains("最新ユーザープロンプト"));
    assert!(html.contains("最新の入力"));
    assert!(html.contains("内部状態サマリ（Context）"));
    assert!(html.contains("今回の動き（Trace）"));
    assert!(html.contains("thoughts=1 / actions=1 / observations=1"));
    assert!(html.contains("last_action: grep"));
    assert!(html.contains("Harness内部状態:"));
}
