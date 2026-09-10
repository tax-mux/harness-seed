//! JSON Lines ワイヤプロトコルの統合テスト。

use harness_seed::{ReActLoop, SimpleRuleBrain, WIRE_VERSION};
use serde_json::Value;

#[test]
fn wire_turn_and_session_clear() {
    let mut react = ReActLoop::with_defaults(SimpleRuleBrain::new());

    let turn_out = react
        .handle_wire_json(r#"{"type":"turn","user_input":"help"}"#)
        .unwrap();
    let turn: Value = serde_json::from_str(&turn_out).unwrap();
    assert_eq!(turn["version"], WIRE_VERSION);
    assert_eq!(turn["type"], "turn");
    assert_eq!(turn["ok"], true);
    assert_eq!(turn["session_turns"], 1);

    let clear_out = react
        .handle_wire_json(r#"{"type":"session_clear"}"#)
        .unwrap();
    let clear: Value = serde_json::from_str(&clear_out).unwrap();
    assert_eq!(clear["type"], "session_clear");
    assert_eq!(clear["session_turns"], 0);
}
