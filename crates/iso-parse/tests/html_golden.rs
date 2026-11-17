//! Golden tests for the HTML extractor (`fixtures/html/*/expected/facts.json`).

#[path = "generic_golden_support.rs"]
mod support;

#[test]
fn html_goldens() {
    assert!(support::check_goldens("html") >= 1);
}
