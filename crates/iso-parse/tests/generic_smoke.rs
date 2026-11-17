//! Smoke test over real code: every Python, Rust, HTML, CSS, Go, Java and C# file under the
//! given roots must extract without a panic. Ignored by default (it reads outside the repo).
//!
//! Run: `cargo test -p iso-parse --release --test generic_smoke -- --ignored --nocapture`
//! Roots: `ISOLINE_SMOKE_ROOTS` (`:`-separated), else `~/Documents/Projects/Opensource/
//! {crosspoint-reader,openclaw}` and this repo's `crates/`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use iso_core::Language;

const SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    ".venv",
    "venv",
    "__pycache__",
    ".next",
    ".cache",
];

fn roots() -> Vec<PathBuf> {
    if let Ok(v) = std::env::var("ISOLINE_SMOKE_ROOTS") {
        return v
            .split(':')
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .collect();
    }
    let home = std::env::var("HOME").unwrap_or_default();
    let os = Path::new(&home).join("Documents/Projects/Opensource");
    vec![
        os.join("crosspoint-reader"),
        os.join("openclaw"),
        Path::new(env!("CARGO_MANIFEST_DIR")).join(".."),
    ]
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_symlink() {
            continue;
        }
        if ft.is_dir() {
            let name = e.file_name();
            if !SKIP_DIRS.iter().any(|s| name == *s) {
                walk(&p, out);
            }
        } else {
            out.push(p);
        }
    }
}

#[derive(Default)]
struct Stats {
    files: usize,
    bytes: usize,
    symbols: usize,
    imports: usize,
    calls: usize,
    refs: usize,
    with_errors: usize,
    time: Duration,
    slowest: (Duration, String),
}

#[test]
#[ignore]
fn generic_smoke_real_repos() {
    let wanted = [
        Language::Python,
        Language::Rust,
        Language::Html,
        Language::Css,
        Language::Go,
        Language::Java,
        Language::CSharp,
    ];
    let mut stats: BTreeMap<&'static str, Stats> = BTreeMap::new();
    let mut panics = Vec::new();
    let total = Instant::now();
    for root in roots() {
        if !root.exists() {
            println!("skip missing root {}", root.display());
            continue;
        }
        let mut files = Vec::new();
        walk(&root, &mut files);
        for f in files {
            let rel = f.to_string_lossy().to_string();
            let lang = Language::detect(&rel, None);
            if !wanted.contains(&lang) {
                continue;
            }
            let Ok(src) = std::fs::read_to_string(&f) else {
                continue;
            };
            let t = Instant::now();
            let result = std::panic::catch_unwind(|| iso_parse::extract_file(&f, lang, &src));
            let dt = t.elapsed();
            let s = stats.entry(lang.as_str()).or_default();
            match result {
                Ok(facts) => {
                    s.files += 1;
                    s.bytes += src.len();
                    s.symbols += facts.symbols.len();
                    s.imports += facts.imports.len();
                    s.calls += facts.calls.len();
                    s.refs += facts.refs.len();
                    s.with_errors += facts.has_errors as usize;
                    s.time += dt;
                    if dt > s.slowest.0 {
                        s.slowest = (dt, rel);
                    }
                }
                Err(_) => panics.push(rel),
            }
        }
    }
    println!(
        "{:<8} {:>6} {:>9} {:>8} {:>8} {:>8} {:>8} {:>6} {:>9}  slowest",
        "lang", "files", "KB", "symbols", "imports", "calls", "refs", "errors", "ms"
    );
    for (lang, s) in &stats {
        println!(
            "{:<8} {:>6} {:>9} {:>8} {:>8} {:>8} {:>8} {:>6} {:>9.1}  {:.1} ms {}",
            lang,
            s.files,
            s.bytes / 1024,
            s.symbols,
            s.imports,
            s.calls,
            s.refs,
            s.with_errors,
            s.time.as_secs_f64() * 1000.0,
            s.slowest.0.as_secs_f64() * 1000.0,
            s.slowest.1
        );
    }
    println!("total wall time {:.2} s", total.elapsed().as_secs_f64());
    assert!(panics.is_empty(), "panicked on: {panics:#?}");
}
