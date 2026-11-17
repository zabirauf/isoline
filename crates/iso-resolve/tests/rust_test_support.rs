//! Shared helpers for the Python, Rust and HTML resolver tests, included with
//! `#[path = "rust_test_support.rs"] mod support;`. Cargo also builds it as its own (empty)
//! test target.
//!
//! `fixtures/<lang>/<case>/expected/resolutions.json` (written by hand) is a list of
//! `{from, specifier, kind?, names?, expect}` where `expect` is `{"file": …}`,
//! `{"external": …}` or `"unresolved"`.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use iso_core::{ImportFact, ImportKind, ImportedName};
use iso_resolve::{RepoFiles, Resolution, Resolver};

pub fn fixture(lang: &str, case: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(lang)
        .join(case)
}

/// Every file under `root` except `expected/`, repo-relative.
pub fn repo_at(root: &Path) -> RepoFiles {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<String>) {
        for e in std::fs::read_dir(dir).expect("dir") {
            let p = e.expect("entry").path();
            if p.is_dir() {
                if p.file_name().is_some_and(|n| n == "expected") {
                    continue;
                }
                walk(&p, root, out);
            } else {
                let rel = p.strip_prefix(root).expect("inside");
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    let mut files = Vec::new();
    walk(root, root, &mut files);
    RepoFiles::new(root, files)
}

pub fn import(spec: &str, names: &[&str], kind: ImportKind) -> ImportFact {
    ImportFact {
        specifier: spec.to_string(),
        kind,
        names: names
            .iter()
            .map(|n| ImportedName {
                name: n.to_string(),
                alias: None,
                type_only: false,
            })
            .collect(),
        type_only: false,
        line: 1,
    }
}

pub fn expected(v: &serde_json::Value) -> Resolution {
    if v == "unresolved" {
        return Resolution::Unresolved;
    }
    if let Some(f) = v.get("file").and_then(|f| f.as_str()) {
        return Resolution::File(f.to_string());
    }
    if let Some(m) = v.get("external").and_then(|f| f.as_str()) {
        return Resolution::External(m.to_string());
    }
    panic!("bad expectation {v}");
}

/// Runs every case in the fixture's `expected/resolutions.json`; returns the case count.
pub fn check_resolutions(root: &Path, resolver: &dyn Resolver) -> usize {
    let cases: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("expected/resolutions.json")).expect("golden"),
    )
    .expect("json");
    let mut failures = Vec::new();
    let cases = cases.as_array().expect("array");
    for c in cases {
        let from = c["from"].as_str().expect("from");
        let spec = c["specifier"].as_str().expect("specifier");
        let kind = match c.get("kind").and_then(|k| k.as_str()) {
            Some("mod") => ImportKind::Mod,
            Some("reexport") => ImportKind::Reexport,
            Some("link") => ImportKind::Link,
            _ => ImportKind::Static,
        };
        let names: Vec<&str> = c
            .get("names")
            .and_then(|n| n.as_array())
            .map(|a| a.iter().filter_map(|n| n.as_str()).collect())
            .unwrap_or_default();
        let got = resolver.resolve(from, &import(spec, &names, kind));
        let want = expected(&c["expect"]);
        if got != want {
            failures.push(format!(
                "{from}: {spec} {names:?}: got {got:?}, want {want:?}"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    cases.len()
}
