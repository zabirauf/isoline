//! Shared golden-file check, included by the per-language tests with
//! `#[path = "generic_golden_support.rs"] mod support;`. Cargo also builds it as its own
//! (empty) test target.
//!
//! `fixtures/<lang>/<case>/expected/facts.json` maps case-relative paths to the expected
//! `FileFacts`, written by hand. `body_hash` is not compared (it must be non-zero); fields
//! with serde defaults may be omitted.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use iso_core::{FileFacts, Language};
use pretty_assertions::assert_eq;

/// Checks every golden under `fixtures/<lang_dir>`; returns how many files were compared.
pub fn check_goldens(lang_dir: &str) -> usize {
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
