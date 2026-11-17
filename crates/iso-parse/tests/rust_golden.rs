//! Golden tests for the Rust extractor (`fixtures/rust/*/expected/facts.json`).

use std::path::Path;

use iso_core::Language;

#[path = "generic_golden_support.rs"]
mod support;

#[test]
fn rust_goldens() {
    assert!(support::check_goldens("rust") >= 2);
}

#[test]
fn rust_broken_code_still_extracts() {
    let src = "fn ok() {}\nfn broken( {\nstruct S;\nimpl S { fn m(&self) {} }\n";
    let f = iso_parse::extract_file(Path::new("b.rs"), Language::Rust, src);
    assert!(f.has_errors);
    assert!(f.symbols.iter().any(|s| s.qualified_name == "ok"));
}

#[test]
fn rust_deep_nesting_does_not_overflow() {
    // A long method chain nests one level per call.
    let mut src = String::from("fn f() { x");
    for _ in 0..20_000 {
        src.push_str(".a()");
    }
    src.push_str("; }\n");
    let f = iso_parse::extract_file(Path::new("deep.rs"), Language::Rust, &src);
    assert_eq!(f.calls.len(), 20_000);
}
