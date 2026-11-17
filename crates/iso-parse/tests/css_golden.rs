//! Golden tests for the CSS extractor (`fixtures/css/*/expected/facts.json`).

#[path = "generic_golden_support.rs"]
mod support;

#[test]
fn css_goldens() {
    assert!(support::check_goldens("css") >= 1);
}
