use myharness::VERSION;

#[test]
fn version_is_set() {
    assert!(!VERSION.is_empty());
}
