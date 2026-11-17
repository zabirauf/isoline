//! Corpus check against analyze.js output (not part of the normal run).
//!
//! `FLOW_CORPUS=<list>` names a file of `source<TAB>golden.json` lines, where each golden is
//! `node tools/analyze/analyze.js <source>`. Every golden unit must match.
//!
//! ```sh
//! FLOW_CORPUS=/tmp/list.txt cargo test -p iso-parse --release --test flow_corpus -- --ignored --nocapture
//! ```

use std::path::Path;

use iso_core::Language;
use serde_json::Value;

#[test]
#[ignore]
fn corpus_matches_analyze_js() {
    let Ok(list) = std::env::var("FLOW_CORPUS") else {
        eprintln!("FLOW_CORPUS not set");
        return;
    };
    let list = std::fs::read_to_string(list).unwrap();
    let (mut files, mut units, mut bad_units) = (0, 0, 0);
    let mut bad_files = Vec::new();
    for l in list.lines() {
        let Some((src_path, golden_path)) = l.split_once('\t') else {
            continue;
        };
        let src = std::fs::read_to_string(src_path).unwrap();
        let Ok(golden) = serde_json::from_str::<serde_json::Map<String, Value>>(
            &std::fs::read_to_string(golden_path).unwrap(),
        ) else {
            continue;
        };
        files += 1;
        let lang = Language::detect(src_path, None);
        let got: serde_json::Map<String, Value> =
            iso_parse::flow_units(Path::new(src_path), lang, &src)
                .into_iter()
                .map(|u| (u.id.clone(), serde_json::to_value(&u).unwrap()))
                .collect();
        let mut bad = Vec::new();
        for (id, want) in &golden {
            units += 1;
            if got.get(id) != Some(want) {
                bad_units += 1;
                bad.push(id.clone());
            }
        }
        if !bad.is_empty() {
            if let Ok(dir) = std::env::var("FLOW_DUMP") {
                let out = Path::new(&dir).join(format!("{}.ours.json", bad_files.len()));
                std::fs::write(&out, serde_json::to_string(&got).unwrap()).unwrap();
                println!("  wrote {} for {golden_path}", out.display());
            }
            bad_files.push(format!("{src_path}: {}", bad.join(", ")));
        }
    }
    println!("{files} files, {units} units, {bad_units} mismatched");
    for b in &bad_files {
        println!("  {b}");
    }
    assert_eq!(bad_units, 0);
}
