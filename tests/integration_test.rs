use harness_seed::{ReActLoop, SimpleRuleBrain, VERSION};

#[test]
fn version_is_set() {
    assert!(!VERSION.is_empty());
}

#[test]
fn react_loop_responds_to_input() {
    let mut react = ReActLoop::with_defaults(SimpleRuleBrain::new());
    let result = react.run_turn("echo integration").unwrap();
    assert!(result.answer.contains("integration"));
}
