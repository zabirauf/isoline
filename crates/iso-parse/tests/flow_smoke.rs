//! Python and Rust call flow over real trees (not part of the normal run): no panics,
//! timings, and every unit id must be a callable's qualified name (cards link by it).
//!
//! ```sh
//! FLOW_SMOKE_DIRS=~/src/a:~/src/b cargo test -p iso-parse --release --test flow_smoke -- --ignored --nocapture
//! ```
//!
//! Without `FLOW_SMOKE_DIRS` it walks this repo's `crates/`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use iso_core::Language;

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.')
            || matches!(
                name.as_ref(),
                "node_modules" | "target" | "dist" | "build" | "venv" | "__pycache__"
            )
        {
            continue;
        }
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() {
            walk(&p, out);
        } else if ft.is_file() && (name.ends_with(".py") || name.ends_with(".rs")) {
            out.push(p);
        }
    }
}

#[derive(Default)]
struct Tally {
    files: usize,
    lines: usize,
    units: usize,
    orphans: usize,
    panics: usize,
    time: Duration,
    slowest: (Duration, PathBuf),
    examples: Vec<String>,
}

#[test]
#[ignore]
fn smoke() {
    let dirs: Vec<PathBuf> = match std::env::var("FLOW_SMOKE_DIRS") {
        Ok(v) => std::env::split_paths(&v).collect(),
        Err(_) => vec![Path::new(env!("CARGO_MANIFEST_DIR")).join("..")],
    };
    let mut files = Vec::new();
    for d in &dirs {
        walk(d, &mut files);
    }
    let mut py = Tally::default();
    let mut rs = Tally::default();
    for p in &files {
        let Ok(src) = std::fs::read_to_string(p) else {
            continue;
        };
        let lang = Language::detect(&p.to_string_lossy(), None);
        let t = if lang == Language::Python {
            &mut py
        } else {
            &mut rs
        };
        t.files += 1;
        t.lines += src.lines().count();
        let start = Instant::now();
        let res = std::panic::catch_unwind(|| iso_parse::flow_units(p, lang, &src));
        let took = start.elapsed();
        t.time += took;
        if took > t.slowest.0 {
            t.slowest = (took, p.clone());
        }
        let Ok(units) = res else {
            t.panics += 1;
            t.examples.push(format!("PANIC {}", p.display()));
            continue;
        };
        t.units += units.len();
        let facts = iso_parse::extract_file(p, lang, &src);
        let callables: HashSet<&str> = facts
            .symbols
            .iter()
            .filter(|s| s.kind.is_callable())
            .map(|s| s.qualified_name.as_str())
            .collect();
        for u in &units {
            if !callables.contains(u.id.as_str()) {
                t.orphans += 1;
                if t.examples.len() < 12 {
                    t.examples
                        .push(format!("orphan unit {} in {}", u.id, p.display()));
                }
            }
        }
    }
    for (name, t) in [("python", &py), ("rust", &rs)] {
        let ms = t.time.as_secs_f64() * 1e3;
        println!(
            "{name}: {} files, {} lines, {} units, {} orphan units, {} panics; flow {:.1} ms ({:.0} klines/s), slowest {:.1} ms {}",
            t.files,
            t.lines,
            t.units,
            t.orphans,
            t.panics,
            ms,
            t.lines as f64 / 1e3 / t.time.as_secs_f64().max(1e-9),
            t.slowest.0.as_secs_f64() * 1e3,
            t.slowest.1.display()
        );
        for e in &t.examples {
            println!("  {e}");
        }
    }
    assert_eq!(py.panics + rs.panics, 0);
}
