use harness_seed::{
    format_references_for_prompt, HarnessMailRefKind, HarnessReference, HarnessState,
};
use harness_seed::plan::PlanArtifact;

#[test]
fn harness_reference_formats_kind_labels() {
    let refs = vec![
        HarnessReference {
            id: Some("inbox:1".into()),
            kind: HarnessMailRefKind::Inbox,
            uid: 1,
            subject: "件名".into(),
            from: "a@b.c".into(),
            to: None,
            body: "本文".into(),
        },
        HarnessReference {
            id: Some("outgoing:2".into()),
            kind: HarnessMailRefKind::OutgoingPending,
            uid: 2,
            subject: "下書き".into(),
            from: "自分".into(),
            to: Some("to@x.y".into()),
            body: String::new(),
        },
    ];
    let text = format_references_for_prompt(&refs);
    assert!(text.contains("【参照情報】"));
    assert!(text.contains("受信メール"));
    assert!(text.contains("送信待ちメール"));
    assert!(text.contains("UID: 1"));
    assert!(text.contains("(本文なし)"));
}

#[test]
fn harness_state_serializes_references() {
    let mut hs = HarnessState::new("1. step", PlanArtifact::single_subtask("do"));
    hs.add_references(vec![HarnessReference {
        id: None,
        kind: HarnessMailRefKind::OutgoingSent,
        uid: 9,
        subject: "sent".into(),
        from: "me".into(),
        to: None,
        body: "ok".into(),
    }]);
    let json = hs.to_json_pretty();
    assert!(json.contains("\"references\""));
    assert!(json.contains("outgoing_sent"));
}
