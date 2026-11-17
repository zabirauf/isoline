//! Facts goldens for TS, TSX and JS. `expected/facts.json` is hand-written; it leaves out
//! `body_hash`, which is checked separately against the contract (hash64 of the symbol's
//! source text).
//!
//! `FACTS_DUMP=1` prints what the extractor produced, for writing a new golden by hand.

use std::path::{Path, PathBuf};

use iso_core::{FileFacts, Language, hash64};
use serde_json::Value;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

fn facts(path: &Path) -> (String, FileFacts) {
    let src = std::fs::read_to_string(path).unwrap();
    let rel = path.to_string_lossy().replace('\\', "/");
    let lang = Language::detect(&rel, None);
    let facts = iso_parse::extract_file(path, lang, &src);
    (src, facts)
}

/// Facts as JSON without `body_hash`, the shape of the golden files.
fn comparable(f: &FileFacts) -> Value {
    let mut v = serde_json::to_value(f).unwrap();
    for s in v["symbols"].as_array_mut().unwrap() {
        s.as_object_mut().unwrap().remove("body_hash");
    }
    v
}

fn check(case: &str, file: &str) {
    let dir = fixtures().join(case);
    let (_, got) = facts(&dir.join(file));
    let have = comparable(&got);
    if std::env::var_os("FACTS_DUMP").is_some() {
        println!("{}", serde_json::to_string_pretty(&have).unwrap());
    }
    let want: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("expected/facts.json")).unwrap())
            .unwrap();
    let mut problems = Vec::new();
    for section in ["symbols", "imports", "calls", "refs"] {
        let w = want[section].as_array().unwrap();
        let h = have[section].as_array().unwrap();
        for x in w.iter().filter(|x| !h.contains(x)) {
            problems.push(format!("{section}: missing {x}"));
        }
        for x in h.iter().filter(|x| !w.contains(x)) {
            problems.push(format!("{section}: unexpected {x}"));
        }
        if problems.is_empty() && w != h {
            problems.push(format!("{section}: same entries, different order"));
        }
    }
    if want["has_errors"] != have["has_errors"] {
        problems.push(format!("has_errors: want {}", want["has_errors"]));
    }
    assert!(
        problems.is_empty(),
        "{case}: {} problem(s)\n{}",
        problems.len(),
        problems.join("\n")
    );
}

#[test]
fn ts_basic_matches_golden() {
    check("ts/facts-basic", "basic.ts");
}

#[test]
fn tsx_matches_golden() {
    check("ts/facts-tsx", "panel.tsx");
}

#[test]
fn js_cjs_matches_golden() {
    check("js/facts-cjs", "cjs.js");
}

/// Every symbol's `body_hash` is hash64 of its declaration's source text, from the first
/// line of the declaration (including `export`) through its last line.
#[test]
fn body_hash_is_hash_of_declaration_text() {
    let (src, f) = facts(&fixtures().join("ts/facts-basic/basic.ts"));
    let find = |qn: &str| f.symbols.iter().find(|s| s.qualified_name == qn).unwrap();
    let load = find("load");
    let start = src.find("export async function load").unwrap();
    let end = start + src[start..].find("\n}\n").unwrap() + 2;
    assert_eq!(load.body_hash, hash64(&src.as_bytes()[start..end]));
    let run = find("Loader.run");
    let start = src.find("async run(").unwrap();
    let end = start + src[start..].find("\n  }\n").unwrap() + 4;
    assert_eq!(run.body_hash, hash64(&src.as_bytes()[start..end]));
}

/// Editing one function's body changes its hash and nothing else's; moving code around
/// (shifting lines) leaves every hash alone.
#[test]
fn body_hash_tracks_body_edits_only() {
    let path = fixtures().join("ts/facts-basic/basic.ts");
    let src = std::fs::read_to_string(&path).unwrap();
    let base = iso_parse::extract_file(&path, Language::TypeScript, &src);
    let hashes = |f: &FileFacts| -> Vec<(String, u64)> {
        f.symbols
            .iter()
            .map(|s| (s.qualified_name.clone(), s.body_hash))
            .collect()
    };

    let edited = src.replace("return text.trim();", "return text.trimEnd();");
    let after = iso_parse::extract_file(&path, Language::TypeScript, &edited);
    for ((qn, a), (_, b)) in hashes(&base).iter().zip(hashes(&after).iter()) {
        if qn == "load" {
            assert_ne!(a, b, "load body changed");
        } else {
            assert_eq!(a, b, "{qn} untouched");
        }
    }

    let shifted = format!("// header\n\n{src}");
    let moved = iso_parse::extract_file(&path, Language::TypeScript, &shifted);
    assert_eq!(hashes(&base), hashes(&moved));
    assert_eq!(base.symbols[0].start_line + 2, moved.symbols[0].start_line);
}
