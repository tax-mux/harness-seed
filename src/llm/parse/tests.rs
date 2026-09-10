use super::*;

#[test]
fn parses_thought_json() {
    let step = parse_agent_step(r#"{"step":"thought","content":"考え中"}"#, 1).unwrap();
    assert!(matches!(step, AgentStep::Thought(_)));
}

#[test]
fn parses_action_json() {
    let raw = r#"{"step":"action","tool":"echo","args":{"message":"hi"}}"#;
    let step = parse_agent_step(raw, 7).unwrap();
    assert!(matches!(step, AgentStep::Action(a) if a.invoke_id == 7 && a.tool == "echo"));
}

#[test]
fn coerces_tool_name_used_as_step_to_action() {
    let raw = r#"{"step":"list_dir","args":{"path":"."}}"#;
    let step = parse_agent_step(raw, 3).unwrap();
    assert!(matches!(
        step,
        AgentStep::Action(a) if a.invoke_id == 3 && a.tool == "list_dir"
            && a.args.get("path").and_then(|p| p.as_str()) == Some(".")
    ));

    let raw = r#"{"step":"read_file","args":{"path":"README.md"}}"#;
    let step = parse_agent_step(raw, 1).unwrap();
    assert!(matches!(step, AgentStep::Action(a) if a.tool == "read_file"));
}

#[test]
fn picks_action_from_multiline_response() {
    let raw = r#"{"step":"thought","content":"plan"}
{"step":"action","tool":"write_file","args":{"path":"a.rs","content":"x"}}"#;
    let step = parse_agent_step(raw, 1).unwrap();
    assert!(matches!(step, AgentStep::Action(a) if a.tool == "write_file"));
}

#[test]
fn strips_markdown_fence() {
    let raw = "```json\n{\"step\":\"answer\",\"content\":\"ok\"}\n```";
    let step = parse_agent_step(raw, 1).unwrap();
    assert!(matches!(step, AgentStep::Answer(a) if a == "ok"));
}

#[test]
fn salvages_answer_with_unescaped_newlines_in_content() {
    let raw = r#"{"step":"answer","content":"参照メール（UID: 302699）の要点です。

【概要】
マネックス証券の高配当米国ETF案内です。

手続き案内: 配信解除の案内があります。"}"#;
    let step = parse_agent_step(raw, 1).unwrap();
    assert!(matches!(
        step,
        AgentStep::Answer(a) if a.contains("手続き案内") && a.contains("302699")
    ));
}

#[test]
fn salvage_extracts_long_markdown_answer() {
    let raw = r#"{"step":"answer","content":"**【概要】**\n本メールは証券会社からの案内です。\n\n**注意**\n投資は自己責任です。"}"#;
    let body = salvage_answer_step_content(raw).expect("salvaged");
    assert!(body.contains("証券会社"));
    assert!(body.contains("自己責任"));
}

#[test]
fn salvages_answer_with_unescaped_quotes_in_content() {
    let raw = r#"{"step":"thought","content":"The user asked for a self-introduction in Japanese ("自己紹介して"). Since I am an AI agent, I should provide a polite introduction."}"#;
    let step = parse_agent_step(raw, 1).unwrap();
    assert!(
        matches!(step, AgentStep::Thought(a) if a.contains("自己紹介して") && a.contains("AI agent"))
    );
}

#[test]
fn picks_answer_from_multiline_objects_with_embedded_newlines() {
    let raw = r#"{"step":"thought","content":"planning"}
{"step":"answer","content":"{
  \"summary\": \"do work\",
  \"skip_execution\": false,
  \"subtasks\": [{\"id\": 1, \"goal\": \"g\", \"done_when\": \"d\"}]
}"}"#;
    let step = parse_agent_step(raw, 1).unwrap();
    assert!(matches!(step, AgentStep::Answer(a) if a.contains("summary")));
}

#[test]
fn salvages_answer_with_unquoted_japanese_content() {
    let raw = r#"{
  "step": "answer",
  "content": 「ファルモ」は主に以下の2つの意味があります：

1. **ファルモ・ジャパン** - 医療機器やヘルスケア関連の企業
2. **ファルモサ** - 健康食品ブランド
}"#;
    let step = parse_agent_step(raw, 1).unwrap();
    assert!(matches!(
        step,
        AgentStep::Answer(a) if a.contains("ファルモ・ジャパン") && a.contains("ファルモサ")
    ));
}
