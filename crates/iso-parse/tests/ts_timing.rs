//! Throughput of the TS/JS extractor and flow walker on a real tree (not part of the normal
//! run). Single-threaded; numbers are per core.
//!
//! ```sh
//! TIMING_DIR=~/src/some-repo cargo test -p iso-parse --release --test ts_timing -- --ignored --nocapture
//! ```

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
        if name.starts_with('.') || name == "node_modules" || name == "dist" {
            continue;
        }
        if p.is_dir() {
            walk(&p, out);
        } else if [".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs"]
            .iter()
            .any(|x| name.ends_with(x))
            && !name.ends_with(".d.ts")
        {
            out.push(p);
        }
    }
}

#[test]
#[ignore]
fn timing() {
    let Ok(dir) = std::env::var("TIMING_DIR") else {
        eprintln!("TIMING_DIR not set");
        return;
    };
    let mut files = Vec::new();
    walk(Path::new(&dir), &mut files);
    let sources: Vec<(PathBuf, Language, String)> = files
        .into_iter()
        .filter_map(|p| {
            let src = std::fs::read_to_string(&p).ok()?;
            let lang = Language::detect(&p.to_string_lossy(), None);
            Some((p, lang, src))
        })
        .collect();
    let lines: usize = sources.iter().map(|(_, _, s)| s.lines().count()).sum();
    let bytes: usize = sources.iter().map(|(_, _, s)| s.len()).sum();

    let (mut parse, mut extract, mut flow) = (Duration::ZERO, Duration::ZERO, Duration::ZERO);
    let (mut symbols, mut calls, mut units, mut slowest) =
        (0, 0, 0, (Duration::ZERO, PathBuf::new()));
    let trace = std::env::var_os("TIMING_TRACE").is_some();
    for (p, lang, src) in &sources {
        if trace {
            eprintln!("{}", p.display());
        }
        let t = Instant::now();
        let tree = iso_parse::parse(*lang, src, None).unwrap();
        let tp = t.elapsed();
        let ex = iso_parse::extractor(*lang).unwrap();
        let t = Instant::now();
        let f = ex.extract(p, src, &tree);
        let te = t.elapsed();
        let t = Instant::now();
        let u = ex.flow_units(p, src, &tree);
        let tf = t.elapsed();
        parse += tp;
        extract += te;
        flow += tf;
        symbols += f.symbols.len();
        calls += f.calls.len();
        units += u.len();
        if tp + te + tf > slowest.0 {
            slowest = (tp + te + tf, p.clone());
        }
    }
    let ms = |d: Duration| d.as_secs_f64() * 1e3;
    let kloc_s = |d: Duration| lines as f64 / 1e3 / d.as_secs_f64();
    println!(
        "{} files, {lines} lines, {:.1} MB; {symbols} symbols, {calls} calls, {units} flow units",
        sources.len(),
        bytes as f64 / 1e6
    );
    println!(
        "parse    {:8.1} ms  {:7.0} klines/s",
        ms(parse),
        kloc_s(parse)
    );
    println!(
        "extract  {:8.1} ms  {:7.0} klines/s",
        ms(extract),
        kloc_s(extract)
    );
    println!(
        "flow     {:8.1} ms  {:7.0} klines/s",
        ms(flow),
        kloc_s(flow)
    );
    let all = parse + extract + flow;
    println!("total    {:8.1} ms  {:7.0} klines/s", ms(all), kloc_s(all));
    println!("slowest  {:8.1} ms  {}", ms(slowest.0), slowest.1.display());
}
