use super::*;
use crate::plan::{PlanArtifact, PlanProgress, Subtask};
use std::collections::HashSet;

#[test]
fn catalog_hides_web_research_when_disabled() {
    let reg = TaskRegistry::builtin();
    let off = reg.catalog_for_planner_opts(false);
    let on = reg.catalog_for_planner_opts(true);
    assert!(!off.contains("web_research"));
    assert!(on.contains("web_research"));
}

#[test]
fn builtin_has_ordered_steps() {
    let reg = TaskRegistry::builtin();
    assert!(reg.get("web_research").is_some());
    let def = reg.get("write_file_verify").unwrap();
    let methods: Vec<_> = def
        .ordered_required_steps()
        .iter()
        .map(|s| s.method.as_str())
        .collect();
    assert_eq!(methods, vec!["write_file", "read_file"]);
}

#[test]
fn render_mission_is_scoped_to_current_subtask_only() {
    let reg = TaskRegistry::builtin();
    let plan = PlanArtifact {
        summary: "end goal".into(),
        skip_execution: false,
        subtasks: vec![
            Subtask {
                id: 1,
                task: Some("list_dir".into()),
                params: serde_json::json!({ "path": "src" }),
                goal: "list".into(),
                done_when: "listed".into(),
                depends_on: vec![],
            },
            Subtask {
                id: 2,
                task: Some("write_file_verify".into()),
                params: serde_json::json!({}),
                goal: "write".into(),
                done_when: "verified".into(),
                depends_on: vec![],
            },
        ],
        knowledge_sufficient: None,
        user_reply: None,
    };
    let st = plan.subtasks[0].clone();
    let m = reg
        .render_mission("user asks for much", &plan, &st, &PlanProgress::default())
        .unwrap();
    assert!(m.contains("id: 1"));
    assert!(!m.contains("id: 2"));
    assert!(!m.contains("end goal"));
    assert!(!m.contains("All subtasks"));
    assert!(!m.contains("write_file_verify"));
}

#[test]
fn render_mission_lists_required_order() {
    let reg = TaskRegistry::builtin();
    let plan = PlanArtifact {
        summary: "list".into(),
        skip_execution: false,
        subtasks: vec![Subtask {
            id: 1,
            task: Some("list_dir".into()),
            params: serde_json::json!({ "path": "src" }),
            goal: String::new(),
            done_when: String::new(),
            depends_on: vec![],
        }],
        knowledge_sufficient: None,
        user_reply: None,
    };
    let st = plan.subtasks[0].clone();
    let m = reg
        .render_mission("list files", &plan, &st, &PlanProgress::default())
        .unwrap();
    assert!(m.contains("Required execution order"));
    assert!(m.contains("1. list_dir"));
}

#[test]
fn render_mission_adds_evidence_grounding_when_prior_results() {
    let reg = TaskRegistry::builtin();
    let plan = PlanArtifact {
        summary: "work".into(),
        skip_execution: false,
        subtasks: vec![
            Subtask {
                id: 1,
                task: Some("list_dir".into()),
                params: serde_json::json!({}),
                goal: "list".into(),
                done_when: "listed".into(),
                depends_on: vec![],
            },
            Subtask {
                id: 2,
                task: None,
                params: serde_json::json!({}),
                goal: "judge from evidence".into(),
                done_when: "judged".into(),
                depends_on: vec![],
            },
        ],
        knowledge_sufficient: None,
        user_reply: None,
    };
    let mut progress = PlanProgress::default();
    progress.push(1, "listed src/ and Cargo.toml");
    let m = reg
        .render_mission("user goal", &plan, &plan.subtasks[1], &progress)
        .unwrap();
    assert!(m.contains("Evidence grounding"));
    assert!(m.contains("listed src/"));
    assert!(m.contains("unverified candidate"));
}

#[test]
fn format_subtask_execution_shows_steps() {
    let reg = TaskRegistry::builtin();
    let sub = Subtask {
        id: 1,
        task: Some("list_dir".into()),
        params: serde_json::json!({ "path": "src" }),
        goal: String::new(),
        done_when: String::new(),
        depends_on: vec![],
    };
    let text = reg.format_subtask_execution_for_display(&sub);
    assert!(
        text.contains("list_dir"),
        "expected list_dir in display, got: {text}"
    );
    assert!(
        text.contains("step-driver") || text.contains("ReAct"),
        "expected execution mode label, got: {text}"
    );
}

#[test]
fn catalog_shows_method_chain() {
    let reg = TaskRegistry::builtin();
    let cat = reg.catalog_for_planner();
    assert!(cat.contains("write_file → read_file"));
}

#[test]
fn resolve_plan_strengthens_weak_freeform_done_when() {
    let reg = TaskRegistry::builtin();
    let mut plan = PlanArtifact {
        summary: "work".into(),
        skip_execution: false,
        subtasks: vec![Subtask {
            id: 1,
            task: None,
            params: serde_json::json!({}),
            goal: "explore then judge".into(),
            done_when: "step completed".into(),
            depends_on: vec![],
        }],
        knowledge_sufficient: None,
        user_reply: None,
    };
    reg.resolve_plan(&mut plan, "user", None);
    assert!(
        plan.subtasks[0].done_when.contains("concrete evidence"),
        "got: {}",
        plan.subtasks[0].done_when
    );
}

#[test]
fn resolve_plan_strips_unknown_task_id_as_freeform() {
    let reg = TaskRegistry::builtin();
    let mut plan = PlanArtifact {
        summary: "work".into(),
        skip_execution: false,
        subtasks: vec![Subtask {
            id: 1,
            task: Some("not_a_registered_task".into()),
            params: serde_json::json!({}),
            goal: "do it".into(),
            done_when: "done".into(),
            depends_on: vec![],
        }],
        knowledge_sufficient: None,
        user_reply: None,
    };
    reg.resolve_plan(&mut plan, "user input", None);
    let st = &plan.subtasks[0];
    assert!(st.task.is_none());
    assert!(st.goal.contains("not_a_registered_task"));
    assert!(st.goal.contains("do it"));

    // Unknown demoted id must not become a phantom allow-list tool.
    assert!(reg.tool_policy_for_subtask(st).is_none());
    let available: HashSet<_> = ["list_dir".into()].into_iter().collect();
    assert!(reg
        .tool_policy_for_subtask_with_tools(st, Some(&available))
        .is_none());
}

#[test]
fn resolve_plan_keeps_reserved_replan_task() {
    let reg = TaskRegistry::builtin();
    let mut plan = PlanArtifact {
        summary: "work".into(),
        skip_execution: false,
        subtasks: vec![
            Subtask {
                id: 1,
                task: Some("web_research".into()),
                params: serde_json::json!({}),
                goal: "gather".into(),
                done_when: "have hits".into(),
                depends_on: vec![],
            },
            Subtask {
                id: 2,
                task: Some("replan".into()),
                params: serde_json::json!({}),
                goal: "decide next from evidence".into(),
                done_when: "revised plan".into(),
                depends_on: vec![],
            },
        ],
        knowledge_sufficient: None,
        user_reply: None,
    };
    reg.resolve_plan(&mut plan, "user input", None);
    let st = &plan.subtasks[1];
    assert_eq!(st.task.as_deref(), Some("replan"));
    assert!(crate::plan::is_replan_subtask(st));
    assert!(!st.goal.contains("not a registered task"));
    assert!(reg.tool_policy_for_subtask(st).is_none());
}

#[test]
fn freeform_hint_allows_only_real_tools() {
    let reg = TaskRegistry::builtin();
    let st = Subtask {
        id: 1,
        task: None,
        params: serde_json::json!({}),
        goal: "Execute with ReAct tools (not a registered task id): list_dir. list root".into(),
        done_when: "done".into(),
        depends_on: vec![],
    };
    let available: HashSet<_> = ["list_dir".into(), "read_file".into()]
        .into_iter()
        .collect();
    let policy = reg
        .tool_policy_for_subtask_with_tools(&st, Some(&available))
        .expect("real tool hint");
    assert_eq!(policy.allow, vec!["list_dir".to_string()]);
}

#[test]
fn catalog_includes_control_plane_footer() {
    let reg = TaskRegistry::builtin();
    let cat = reg.catalog_for_planner();
    assert!(cat.contains("Control-plane tasks"));
    assert!(cat.contains("replan:"));
}

#[test]
fn resolve_plan_demotes_task_with_unavailable_tools() {
    let reg = TaskRegistry::builtin();
    let available: HashSet<_> = ["list_dir".into()].into_iter().collect();
    let mut plan = PlanArtifact {
        summary: "research".into(),
        skip_execution: false,
        subtasks: vec![Subtask {
            id: 1,
            task: Some("web_research".into()),
            params: serde_json::json!({}),
            goal: String::new(),
            done_when: String::new(),
            depends_on: vec![],
        }],
        knowledge_sufficient: None,
        user_reply: None,
    };
    reg.resolve_plan_with_tools(&mut plan, "user", None, Some(&available));
    let st = &plan.subtasks[0];
    assert!(st.task.is_none(), "expected demotion, got {:?}", st.task);
    assert!(st.goal.contains("unavailable tools"));
    assert!(st.goal.contains("web_search"));
}

#[test]
fn tasks_missing_tools_reports_gaps() {
    let reg = TaskRegistry::builtin();
    let available: HashSet<_> = ["list_dir".into()].into_iter().collect();
    let gaps = reg.tasks_missing_tools(&available);
    assert!(
        gaps.iter().any(|(id, missing)| {
            id == "web_research" && missing.iter().any(|m| m == "web_search")
        }),
        "gaps={gaps:?}"
    );
}

#[test]
fn resolve_plan_injects_reference_id_into_task_params() {
    let mut reg = TaskRegistry::default();
    let def: TaskDefinition = serde_json::from_str(
        r#"{
            "id": "fetch_item",
            "summary": "fetch by uid",
            "default_params": { "uid": 0 },
            "steps": [
                { "order": 1, "method": "read_file", "args": { "path": "{uid}" }, "required": true }
            ]
        }"#,
    )
    .unwrap();
    reg.register(def).unwrap();
    let mut plan = PlanArtifact {
        summary: "fetch".into(),
        skip_execution: false,
        subtasks: vec![Subtask {
            id: 1,
            task: Some("fetch_item".into()),
            params: serde_json::json!({}),
            goal: String::new(),
            done_when: String::new(),
            depends_on: vec![],
        }],
        knowledge_sufficient: None,
        user_reply: None,
    };
    let contract = crate::plan::PlanDataContract::new("read: item", "write: chat", "fetch_item")
        .with_reference_id(Some(42));
    reg.resolve_plan(&mut plan, "user", Some(&contract));
    assert_eq!(plan.subtasks[0].params["uid"], 42);
}

#[test]
fn resolve_plan_host_enforce_runs_before_param_merge() {
    let mut reg = TaskRegistry::default();
    let def: TaskDefinition = serde_json::from_str(
        r#"{
            "id": "save_item",
            "summary": "save",
            "default_params": {},
            "steps": [
                { "order": 1, "method": "write_file", "args": { "path": "x" }, "required": true }
            ]
        }"#,
    )
    .unwrap();
    reg.register(def).unwrap();
    let mut plan = PlanArtifact {
        summary: "x".into(),
        skip_execution: false,
        subtasks: vec![
            Subtask {
                id: 1,
                task: Some("fetch_item".into()),
                params: serde_json::json!({}),
                goal: "load".into(),
                done_when: "loaded".into(),
                depends_on: vec![],
            },
            Subtask {
                id: 2,
                task: Some("save_item".into()),
                params: serde_json::json!({}),
                goal: "persist changes".into(),
                done_when: "saved".into(),
                depends_on: vec![],
            },
        ],
        knowledge_sufficient: None,
        user_reply: None,
    };
    let contract =
        crate::plan::PlanDataContract::new("in", "out", "save_item").with_enforce(|plan| {
            let goals: Vec<String> = plan
                .subtasks
                .iter()
                .filter(|st| st.task.as_deref() != Some("fetch_item"))
                .map(|st| st.goal.clone())
                .collect();
            plan.subtasks = vec![Subtask {
                id: 1,
                task: Some("save_item".into()),
                params: serde_json::json!({ "id": 9 }),
                goal: goals.join(" → "),
                done_when: "saved".into(),
                depends_on: vec![],
            }];
        });
    reg.resolve_plan(&mut plan, "user", Some(&contract));
    assert_eq!(plan.subtasks.len(), 1);
    assert_eq!(plan.subtasks[0].task.as_deref(), Some("save_item"));
    assert_eq!(plan.subtasks[0].params["id"], 9);
    assert!(plan.subtasks[0].goal.contains("persist"));
}

#[test]
fn resolve_plan_blocks_reference_fetch_skips_injection() {
    let mut reg = TaskRegistry::default();
    let def: TaskDefinition = serde_json::from_str(
        r#"{
            "id": "fetch_item",
            "summary": "fetch by uid",
            "default_params": { "uid": 0 },
            "steps": []
        }"#,
    )
    .unwrap();
    reg.register(def).unwrap();
    let mut plan = PlanArtifact {
        summary: "fetch".into(),
        skip_execution: false,
        subtasks: vec![Subtask {
            id: 1,
            task: Some("fetch_item".into()),
            params: serde_json::json!({}),
            goal: String::new(),
            done_when: String::new(),
            depends_on: vec![],
        }],
        knowledge_sufficient: None,
        user_reply: None,
    };
    let contract = crate::plan::PlanDataContract::new("in", "out", "fetch_item")
        .with_reference_id(Some(42))
        .with_blocks_reference_fetch(true);
    reg.resolve_plan(&mut plan, "user", Some(&contract));
    assert_eq!(plan.subtasks[0].params["uid"], 0);
}
