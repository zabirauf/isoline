//! Golden tests for the Python extractor (`fixtures/python/*/expected/facts.json`).

use std::path::Path;

use iso_core::Language;

#[path = "generic_golden_support.rs"]
mod support;

#[test]
fn python_goldens() {
    assert!(support::check_goldens("python") >= 2);
}

#[test]
fn python_body_hash_tracks_body_only() {
    let a = "def f(x):\n    return x\n\ndef g():\n    pass\n";
    let b = "def f(x):\n    return x + 1\n\ndef g():\n    pass\n";
    let fa = iso_parse::extract_file(Path::new("a.py"), Language::Python, a);
    let fb = iso_parse::extract_file(Path::new("a.py"), Language::Python, b);
    assert_ne!(fa.symbols[0].body_hash, fb.symbols[0].body_hash);
    assert_eq!(fa.symbols[1].body_hash, fb.symbols[1].body_hash);
}

#[test]
fn python_broken_code_still_extracts() {
    let src =
        "def ok():\n    return 1\n\ndef broken(:\n    pass\n\nclass C:\n    def m(self): ...\n";
    let f = iso_parse::extract_file(Path::new("b.py"), Language::Python, src);
    assert!(f.has_errors);
    assert!(f.symbols.iter().any(|s| s.qualified_name == "ok"));
    assert!(f.symbols.iter().any(|s| s.qualified_name == "C.m"));
}

#[test]
fn python_signature_stops_at_colon() {
    let src = "def f(\n    a,\n    b,\n):  # trailing: comment\n    return a\n";
    let f = iso_parse::extract_file(Path::new("s.py"), Language::Python, src);
    assert_eq!(f.symbols[0].signature, "def f(a, b)");
}

#[test]
fn clip_matches_short() {
    use iso_parse::util::{clip, short};
    for s in [
        "",
        "  a  b ",
        "abcd efg",
        "abc defg",
        "abcde",
        "a\n\tb  c d e f g h",
    ] {
        for max in [1, 2, 4, 5, 6, 8, 50] {
            assert_eq!(clip(s, max), short(s, max), "{s:?} {max}");
        }
    }
}

#[test]
fn python_deep_nesting_does_not_overflow() {
    let mut src = String::from("x = 1");
    for _ in 0..20_000 {
        src.push_str(" + f()");
    }
    src.push('\n');
    let f = iso_parse::extract_file(Path::new("deep.py"), Language::Python, &src);
    assert_eq!(f.calls.len(), 20_000);
}
