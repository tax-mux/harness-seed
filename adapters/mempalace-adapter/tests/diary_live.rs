use mempalace_adapter::{MempalaceClient, MempalaceConfig};

#[test]
#[ignore]
fn diary_write_live_like_harness() {
    let mut cfg = MempalaceConfig::default();
    cfg.wing_from_cwd = false;
    cfg.wing = Some("wing_harness-seed".into());
    cfg.agent_name = "harness-seed".into();
    cfg.init_wing_if_missing = false;
    let client = MempalaceClient::connect(cfg).expect("connect");
    let entry = format!(
        "id:live-utf8-test|SESSION:2026-07-04|summary:test|user:ファルモってなんじゃ|answer:日本語が壊れないこと"
    );
    client
        .diary_write(&entry, Some("harness-seed"))
        .expect("diary_write");
    let recent = client.diary_read(5).expect("diary_read");
    eprintln!("recent entries: {}", recent.len());
    let joined: String = recent.iter().map(|e| e.body.clone()).collect();
    assert!(
        joined.contains("ファルモ") || joined.contains("日本語"),
        "Japanese must survive MCP stdio (got: {joined})"
    );
    for e in &recent {
        eprintln!("- {} : {}", e.title, e.body.chars().take(80).collect::<String>());
    }
}
