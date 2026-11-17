//! Golden tests for the Python extractor: `fixtures/python/<case>/expected/facts.json` maps
//! case-relative paths to the expected `FileFacts` (written by hand). `body_hash` is not
//! compared; fields with serde defaults may be omitted.

use std::path::{Path, PathBuf};

use iso_core::{FileFacts, Language};
use pretty_assertions::assert_eq;

fn check_goldens(lang_dir: &str) -> usize {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(lang_dir);
    let mut cases: Vec<PathBuf> = std::fs::read_dir(&root)
        .expect("fixture dir")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.join("expected/facts.json").exists())
        .collect();
    cases.sort();
    let mut checked = 0;
    for case in cases {
        let golden = std::fs::read_to_string(case.join("expected/facts.json")).expect("golden");
        let golden: serde_json::Value = serde_json::from_str(&golden).expect("golden json");
        for (rel, facts) in golden.as_object().expect("object of files") {
            let mut facts = facts.clone();
            if let Some(syms) = facts.get_mut("symbols").and_then(|s| s.as_array_mut()) {
                for s in syms {
                    s["body_hash"] = 0.into();
                }
            }
            let expected: FileFacts = serde_json::from_value(facts)
                .unwrap_or_else(|e| panic!("{}: bad golden for {rel}: {e}", case.display()));
            let path = case.join(rel);
            let src = std::fs::read_to_string(&path).expect("fixture source");
            let lang = Language::detect(rel, src.lines().next());
            let mut actual = iso_parse::extract_file(&path, lang, &src);
            for s in &mut actual.symbols {
                assert_ne!(s.body_hash, 0, "{}: body_hash set", s.qualified_name);
                s.body_hash = 0;
            }
            assert_eq!(actual, expected, "{}", path.display());
            checked += 1;
        }
    }
    checked
}

#[test]
fn python_goldens() {
    assert!(check_goldens("python") >= 2);
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
