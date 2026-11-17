//! Tier-2 (outline) goldens for Go, Java and C#, and the text tier.

use std::path::Path;

use iso_core::{Language, Tier};

#[path = "generic_golden_support.rs"]
mod support;

#[test]
fn generic_queries_compile() {
    assert_eq!(
        iso_parse::lang::generic::compile_errors(),
        Vec::<String>::new()
    );
    for spec in iso_parse::lang::generic::SPECS {
        assert_eq!(spec.lang.tier(), Tier::Outline);
        assert!(iso_parse::extractor(spec.lang).is_some());
    }
}

#[test]
fn generic_go_outline() {
    assert_eq!(support::check_goldens("go"), 1);
}

#[test]
fn generic_java_outline() {
    assert_eq!(support::check_goldens("java"), 1);
}

#[test]
fn generic_csharp_outline() {
    assert_eq!(support::check_goldens("csharp"), 1);
}

#[test]
fn generic_text_tier_is_empty() {
    assert_eq!(support::check_goldens("text"), 1);
    let lang = Language::detect("notes/notes.zzz", Some("These are plain notes"));
    assert_eq!(lang.tier(), Tier::Text);
    assert!(iso_parse::extractor(lang).is_none());
    let f = iso_parse::extract_file(Path::new("x.zzz"), lang, "anything at all {{{");
    assert_eq!(f, iso_core::FileFacts::default());
}

#[test]
fn generic_broken_go_still_outlines() {
    let src = "package x\n\nfunc A() {}\n\nfunc B( {\n\ntype T struct{}\n";
    let f = iso_parse::extract_file(Path::new("x.go"), Language::Go, src);
    assert!(f.has_errors);
    assert!(f.symbols.iter().any(|s| s.qualified_name == "A"));
}
