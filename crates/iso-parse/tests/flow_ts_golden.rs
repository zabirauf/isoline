//! Call-flow goldens for TS/JS. `expected/flows.json` comes from `tools/analyze/analyze.js`;
//! every unit in it must match exactly. Extra units Isoline emits (class methods, top-level
//! arrow consts) are listed in `expected/flows.extra.json` when a fixture has them.

use std::path::{Path, PathBuf};

use iso_core::Language;
use serde_json::Value;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

fn ours(path: &Path) -> serde_json::Map<String, Value> {
    let src = std::fs::read_to_string(path).unwrap();
    let rel = path.to_string_lossy().replace('\\', "/");
    let lang = Language::detect(&rel, None);
    let units = iso_parse::flow_units(path, lang, &src);
    let mut map = serde_json::Map::new();
    for u in units {
        map.insert(u.id.clone(), serde_json::to_value(&u).unwrap());
    }
    map
}

fn check(case: &str, file: &str) {
    let dir = fixtures().join(case);
    let got = ours(&dir.join(file));
    let golden: serde_json::Map<String, Value> =
        serde_json::from_str(&std::fs::read_to_string(dir.join("expected/flows.json")).unwrap())
            .unwrap();
    let mut failures = Vec::new();
    for (id, want) in &golden {
        match got.get(id) {
            None => failures.push(format!("missing unit {id}")),
            Some(have) if have != want => {
                failures.push(format!(
                    "unit {id} differs\n--- want\n{}\n--- have\n{}",
                    serde_json::to_string_pretty(want).unwrap(),
                    serde_json::to_string_pretty(have).unwrap()
                ));
            }
            _ => {}
        }
    }
    let extra_path = dir.join("expected/flows.extra.json");
    let extra: serde_json::Map<String, Value> = if extra_path.exists() {
        serde_json::from_str(&std::fs::read_to_string(&extra_path).unwrap()).unwrap()
    } else {
        serde_json::Map::new()
    };
    for (id, have) in &got {
        if golden.contains_key(id) {
            continue;
        }
        match extra.get(id) {
            None => failures.push(format!(
                "unexpected extra unit {id}:\n{}",
                serde_json::to_string_pretty(have).unwrap()
            )),
            Some(want) if want != have => failures.push(format!(
                "extra unit {id} differs\n--- want\n{}\n--- have\n{}",
                serde_json::to_string_pretty(want).unwrap(),
                serde_json::to_string_pretty(have).unwrap()
            )),
            _ => {}
        }
    }
    for id in extra.keys() {
        if !got.contains_key(id) {
            failures.push(format!("missing extra unit {id}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{case}: {} problem(s)\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

#[test]
fn lifecycle_matches_analyze_js() {
    check("ts/lifecycle", "lifecycle.ts");
}

#[test]
fn lifecycle_has_all_17_units() {
    let got = ours(&fixtures().join("ts/lifecycle/lifecycle.ts"));
    assert_eq!(got.len(), 17);
}

#[test]
fn control_matches_analyze_js() {
    check("ts/control", "control.ts");
}

#[test]
fn callbacks_matches_analyze_js() {
    check("ts/callbacks", "callbacks.ts");
}

#[test]
fn classes_extra_units() {
    check("ts/classes", "classes.ts");
}

#[test]
fn plain_js_matches_analyze_js() {
    check("js/plain", "plain.js");
}

#[test]
fn real_argv_matches_analyze_js() {
    check("ts/real-argv", "argv.ts");
}

#[test]
fn real_tui_formatters_matches_analyze_js() {
    check("ts/real-tui-formatters", "tui-formatters.ts");
}

#[test]
fn real_user_profile_events_matches_analyze_js() {
    check("ts/real-user-profile-events", "user-profile-events.ts");
}
