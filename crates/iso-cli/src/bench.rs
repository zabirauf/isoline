//! `iso-cli bench`: measures the plan's performance targets on one repo.
//!
//! Edits are never made to the repo itself: with `--edits`, the walked files are copied to a
//! scratch folder first and every edit happens there.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use iso_core::Tier;
use iso_engine::{Engine, IndexReport};
use serde::Serialize;

#[derive(clap::Args, Clone, Debug)]
pub struct BenchArgs {
    /// Number of random file opens.
    #[arg(long, default_value_t = 200)]
    pub opens: usize,
    /// Number of simulated edits (needs a scratch copy; 0 to skip).
    #[arg(long, default_value_t = 50)]
    pub edits: usize,
    /// Scratch folder for the copy used by edits.
    #[arg(long)]
    pub scratch: Option<PathBuf>,
    /// Write the JSON report here as well as stdout.
    #[arg(long)]
    pub out: Option<PathBuf>,
    #[arg(long, default_value_t = 7)]
    pub seed: u64,
}

#[derive(Serialize, Default, Debug, Clone)]
pub struct Percentiles {
    pub n: usize,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub max_ms: f64,
}

pub fn pct(mut v: Vec<f64>) -> Percentiles {
    if v.is_empty() {
        return Percentiles::default();
    }
    v.sort_by(|a, b| a.total_cmp(b));
    let at = |p: f64| v[((v.len() as f64 - 1.0) * p).round() as usize];
    Percentiles {
        n: v.len(),
        p50_ms: at(0.5),
        p95_ms: at(0.95),
        max_ms: *v.last().unwrap(),
    }
}

#[derive(Serialize, Default, Debug, Clone)]
pub struct BenchReport {
    pub repo: String,
    pub files: u32,
    pub lines: u64,
    pub cores: usize,
    pub cold: IndexReport,
    pub cold_lines_per_min: f64,
    pub reopen: IndexReport,
    pub reopen_ms: u64,
    pub reopen_hit_rate: f64,
    pub open_file: Percentiles,
    pub open_file_failures: usize,
    pub flow_trees: usize,
    pub flow_failures: usize,
    pub empty_flow_files: usize,
    pub edit: Percentiles,
    /// Files reparsed and dependents recomputed after a one-file body edit (max over edits).
    pub edit_reparsed_max: u32,
    pub edit_dependents_max: u32,
    pub branch_switch_files_reported: Option<u32>,
    pub unresolved_imports_pct: f64,
    pub unresolved_by_lang: Vec<(String, u64, u64)>,
    pub tiers: (u32, u32, u32),
    pub peak_rss_mb: f64,
    pub edges: u64,
    pub panics: usize,
}

/// Tiny deterministic PRNG so runs are repeatable.
pub struct Lcg(pub u64);
impl Lcg {
    pub fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    pub fn pick(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

pub fn peak_rss_mb() -> f64 {
    // getrusage reports bytes on macOS and kilobytes on Linux.
    #[cfg(unix)]
    {
        let out = std::process::Command::new("ps")
            .args(["-o", "rss=", "-p", &std::process::id().to_string()])
            .output();
        if let Ok(o) = out {
            return String::from_utf8_lossy(&o.stdout)
                .trim()
                .parse::<f64>()
                .unwrap_or(0.0)
                / 1024.0;
        }
    }
    0.0
}

pub fn run(repo: &Path, _data: Option<&Path>, args: BenchArgs) -> Result<()> {
    let rep = bench(repo, &args)?;
    let s = serde_json::to_string_pretty(&rep)?;
    if let Some(o) = &args.out {
        if let Some(d) = o.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::write(o, &s)?;
    }
    println!("{s}");
    Ok(())
}

pub fn bench(repo: &Path, args: &BenchArgs) -> Result<BenchReport> {
    let repo = repo.canonicalize()?;
    let data = tempdir("iso-bench-data")?;
    let mut r = BenchReport {
        repo: repo.to_string_lossy().into(),
        cores: std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1),
        ..Default::default()
    };

    // Cold index.
    let eng = Engine::open_with_data(&repo, &data)?;
    let t = Instant::now();
    r.cold = eng.index()?;
    let cold_ms = t.elapsed().as_secs_f64() * 1000.0;
    r.files = r.cold.files;
    r.lines = r.cold.lines;
    r.cold_lines_per_min = r.lines as f64 / (cold_ms / 60000.0);
    eprintln!(
        "cold: {} files, {} lines in {:.0} ms",
        r.files, r.lines, cold_ms
    );
    drop(eng);

    // Reopen: a fresh engine on the same data folder.
    let t = Instant::now();
    let eng = Engine::open_with_data(&repo, &data)?;
    r.reopen = eng.index()?;
    r.reopen_ms = t.elapsed().as_millis() as u64;
    let considered = r.reopen.unchanged + r.reopen.hashed;
    r.reopen_hit_rate = if considered == 0 {
        1.0
    } else {
        r.reopen.unchanged as f64 / considered as f64
    };
    eprintln!(
        "reopen: {} ms, hit rate {:.4}",
        r.reopen_ms, r.reopen_hit_rate
    );

    // Random file opens (full tier first, like a user browsing code).
    let files = eng.index.read(iso_index::files_all)?;
    r.tiers = (
        files.iter().filter(|f| f.tier == Tier::Full).count() as u32,
        files.iter().filter(|f| f.tier == Tier::Outline).count() as u32,
        files.iter().filter(|f| f.tier == Tier::Text).count() as u32,
    );
    let full: Vec<&iso_index::FileRow> = files.iter().filter(|f| f.tier != Tier::Text).collect();
    let mut rng = Lcg(args.seed);
    let mut times = Vec::new();
    for _ in 0..args.opens.min(full.len().max(1) * 4) {
        if full.is_empty() {
            break;
        }
        let f = full[rng.pick(full.len())];
        let t = Instant::now();
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| eng.file_view(&f.path)));
        match res {
            Ok(Ok(v)) => {
                times.push(t.elapsed().as_secs_f64() * 1000.0);
                if f.tier == Tier::Full && !v.functions.is_empty() {
                    r.flow_trees += v.flow_units.len();
                    if v.flow_units.is_empty() {
                        r.empty_flow_files += 1;
                    }
                }
            }
            Ok(Err(e)) => {
                eprintln!("open {} failed: {e:#}", f.path);
                r.open_file_failures += 1;
            }
            Err(_) => r.panics += 1,
        }
    }
    r.open_file = pct(times);
    eprintln!(
        "open file: p50 {:.1} ms, p95 {:.1} ms",
        r.open_file.p50_ms, r.open_file.p95_ms
    );

    // Unresolved imports by language (tier 1 only).
    let rows: Vec<(String, i64, i64)> = eng.index.read(|c| {
        let mut st = c.prepare(
            "SELECT f.lang, COUNT(*), SUM(i.unresolved) FROM imports i JOIN files f ON f.id = i.file_id
             WHERE f.tier = 'full' AND i.kind != 'dynamic' GROUP BY f.lang ORDER BY f.lang",
        )?;
        let v = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<iso_index::rusqlite::Result<Vec<_>>>()?;
        Ok(v)
    })?;
    let (tot, unres) = rows
        .iter()
        .fold((0i64, 0i64), |a, r| (a.0 + r.1, a.1 + r.2));
    r.unresolved_imports_pct = if tot == 0 {
        0.0
    } else {
        unres as f64 * 100.0 / tot as f64
    };
    r.unresolved_by_lang = rows
        .into_iter()
        .map(|(l, t, u)| (l, t as u64, u as u64))
        .collect();
    r.edges = eng.index.read(iso_index::edge_count)?;

    // Edits on a scratch copy.
    if args.edits > 0 && !full.is_empty() {
        let scratch = match &args.scratch {
            Some(s) => s.clone(),
            None => tempdir("iso-bench-copy")?,
        };
        copy_walked(&repo, &scratch)?;
        let edata = tempdir("iso-bench-edit-data")?;
        let e2 = Engine::open_with_data(&scratch, &edata)?;
        e2.index()?;
        let mut times = Vec::new();
        let candidates: Vec<String> = full
            .iter()
            .filter(|f| f.tier == Tier::Full)
            .map(|f| f.path.clone())
            .collect();
        for _ in 0..args.edits {
            if candidates.is_empty() {
                break;
            }
            let p = &candidates[rng.pick(candidates.len())];
            let abs = scratch.join(p);
            let Ok(text) = std::fs::read_to_string(&abs) else {
                continue;
            };
            // A body-only edit: an extra blank line at the top shifts every line but changes no
            // exported signature.
            std::fs::write(&abs, format!("\n{text}"))?;
            let t = Instant::now();
            let rep = e2.index_paths(std::slice::from_ref(p))?;
            let _ = e2.file_view(p)?;
            times.push(t.elapsed().as_secs_f64() * 1000.0);
            r.edit_reparsed_max = r.edit_reparsed_max.max(rep.parsed);
            r.edit_dependents_max = r.edit_dependents_max.max(rep.dependents_recomputed);
        }
        r.edit = pct(times);
        eprintln!(
            "edit: p50 {:.1} ms, p95 {:.1} ms",
            r.edit.p50_ms, r.edit.p95_ms
        );

        // Branch switch touching exactly three files.
        if candidates.len() >= 3 {
            for p in candidates.iter().take(3) {
                let abs = scratch.join(p);
                let text = std::fs::read_to_string(&abs)?;
                std::fs::write(&abs, format!("{text}\n// switched\n"))?;
            }
            let rep = e2.index()?;
            r.branch_switch_files_reported = Some(rep.hashed);
        }
        let _ = std::fs::remove_dir_all(&scratch);
        let _ = std::fs::remove_dir_all(&edata);
    }
    r.peak_rss_mb = peak_rss_mb();
    let _ = std::fs::remove_dir_all(&data);
    Ok(r)
}

fn tempdir(prefix: &str) -> Result<PathBuf> {
    let base = std::env::var_os("ISOLINE_SCRATCH")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let p = base.join(format!(
        "{prefix}-{}-{}",
        std::process::id(),
        Instant::now().elapsed().as_nanos() ^ rand_seed()
    ));
    std::fs::create_dir_all(&p)?;
    Ok(p)
}

fn rand_seed() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

/// Copy every walked (non-ignored) file, plus .gitignore files, to `dst`.
pub fn copy_walked(src: &Path, dst: &Path) -> Result<()> {
    for f in iso_walk::walk(src) {
        let to = dst.join(&f.rel);
        if let Some(d) = to.parent() {
            std::fs::create_dir_all(d)?;
        }
        // A file that vanished or is unreadable mid-copy is skipped, not fatal.
        if let Err(e) = std::fs::copy(src.join(&f.rel), &to) {
            eprintln!("copy skipped {}: {e}", f.rel);
        }
    }
    if src.join(".gitignore").exists() {
        let _ = std::fs::copy(src.join(".gitignore"), dst.join(".gitignore"));
    }
    Ok(())
}

pub fn _unused(_: Arc<Engine>) {}
