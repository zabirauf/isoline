//! Call-flow goldens for Python and Rust. `expected/flows.json` is hand-written and lists
//! every unit of the file; the walker must produce exactly those units. The mapping is in
//! `fixtures/python/FLOW.md` and `fixtures/rust/FLOW.md`.

use std::path::{Path, PathBuf};

use iso_core::Language;
use serde_json::Value;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

fn check(case: &str, file: &str) {
    let dir = fixtures().join(case);
    let path = dir.join(file);
    let src = std::fs::read_to_string(&path).unwrap();
    let lang = Language::detect(&path.to_string_lossy(), None);
    let got: serde_json::Map<String, Value> = iso_parse::flow_units(&path, lang, &src)
        .into_iter()
        .map(|u| (u.id.clone(), serde_json::to_value(&u).unwrap()))
        .collect();
    let want: serde_json::Map<String, Value> =
        serde_json::from_str(&std::fs::read_to_string(dir.join("expected/flows.json")).unwrap())
            .unwrap();
    let mut problems = Vec::new();
    for (id, w) in &want {
        match got.get(id) {
            None => problems.push(format!("missing unit {id}")),
            Some(h) if h != w => problems.push(format!(
                "unit {id} differs\n--- want\n{}\n--- have\n{}",
                serde_json::to_string_pretty(w).unwrap(),
                serde_json::to_string_pretty(h).unwrap()
            )),
            _ => {}
        }
    }
    for id in got.keys().filter(|id| !want.contains_key(*id)) {
        problems.push(format!(
            "unexpected unit {id}:\n{}",
            serde_json::to_string_pretty(&got[id]).unwrap()
        ));
    }
    assert!(
        problems.is_empty(),
        "{case}: {} problem(s)\n{}",
        problems.len(),
        problems.join("\n\n")
    );
}

#[test]
fn python_control() {
    check("python/flow-control", "control.py");
}

#[test]
fn python_calls() {
    check("python/flow-calls", "calls.py");
}

#[test]
fn python_nested() {
    check("python/flow-nested", "nested.py");
}

#[test]
fn rust_control() {
    check("rust/flow-control", "control.rs");
}

#[test]
fn rust_calls() {
    check("rust/flow-calls", "calls.rs");
}

#[test]
fn rust_closures() {
    check("rust/flow-closures", "closures.rs");
}
