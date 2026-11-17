//! `iso-cli history`: replay a repo's commit history through incremental indexing and check
//! that the Merkle tree and the early-cutoff pipeline do exactly the work the changes need,
//! and that the incremental index equals a cold index of the same checkout.
//!
//! Works on a scratch clone (`git clone --shared --no-checkout`); the repo's own working tree
//! is never touched.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use iso_engine::{Engine, IndexReport};
use serde::Serialize;

#[derive(clap::Args, Clone, Debug)]
pub struct HistoryArgs {
    /// How many first-parent commits to replay, oldest first, ending at HEAD.
    #[arg(long, default_value_t = 30)]
    pub commits: usize,
    /// Verify against a cold index every N steps (and always at the last step).
    #[arg(long, default_value_t = 10)]
    pub verify_every: usize,
    /// Scratch folder for the clone and the databases.
    #[arg(long)]
    pub scratch: Option<PathBuf>,
    /// Write the JSON report here as well as stdout.
    #[arg(long)]
    pub out: Option<PathBuf>,
}

#[derive(Serialize, Debug, Default)]
pub struct Step {
    pub label: String,
    pub commit: String,
    /// Files git reports changed since the previous step, limited to files the walker indexes.
    pub git_changed: u32,
    pub hashed: u32,
    pub parsed: u32,
    pub facts_cache_hits: u32,
    pub added: u32,
    pub deleted: u32,
    pub exports_changed: u32,
    pub dependents_recomputed: u32,
    pub edge_files: u32,
    pub tree_changed_files: u32,
    pub tree_dirs_visited: u32,
    pub tree_dirs_total: u32,
    pub ms: u64,
    /// Stage timings: walk, parse, resolve, edges, tree.
    pub stages_ms: [u64; 5],
    /// `hashed` and the Merkle diff both equal the git change set.
    pub exact: bool,
    /// Paths in one set but not the other (up to 10), when not exact.
    pub mismatch: Vec<String>,
    pub verified: Option<Verify>,
}

#[derive(Serialize, Debug, Default, Clone)]
pub struct Verify {
    pub equal: bool,
    pub files: (usize, usize),
    pub symbols: (usize, usize),
    pub imports: (usize, usize),
    pub edges: (usize, usize),
    pub diffs: Vec<String>,
}

#[derive(Serialize, Debug, Default)]
pub struct HistoryReport {
    pub repo: String,
    pub commits: usize,
    pub steps: Vec<Step>,
    pub exact_steps: usize,
    pub verified_equal: usize,
    pub verified_total: usize,
    pub total_git_changed: u64,
    pub total_parsed: u64,
    pub total_hashed: u64,
    pub median_step_ms: u64,
}

fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .context("run git")?;
    if !out.status.success() {
        bail!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn run(repo: &Path, args: HistoryArgs) -> Result<()> {
    let rep = history(repo, &args)?;
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

pub fn history(repo: &Path, args: &HistoryArgs) -> Result<HistoryReport> {
    let repo = repo.canonicalize()?;
    let scratch = match &args.scratch {
        Some(s) => s.clone(),
        None => std::env::temp_dir().join(format!("iso-history-{}", std::process::id())),
    };
    std::fs::create_dir_all(&scratch)?;
    let wt = scratch.join("checkout");
    anyhow::ensure!(
        !wt.exists(),
        "scratch checkout {} already exists; pass an empty --scratch",
        wt.display()
    );
    let out = Command::new("git")
        .args(["clone", "--quiet", "--shared", "--no-checkout"])
        .arg(&repo)
        .arg(&wt)
        .output()?;
    anyhow::ensure!(
        out.status.success(),
        "git clone failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // A partial clone keeps old blobs on its promisor remote; let the scratch clone fetch
    // them the same way instead of failing on missing objects.
    if git(&repo, &["config", "--get", "remote.origin.promisor"]).is_ok_and(|v| v.trim() == "true")
    {
        let url = git(&repo, &["config", "--get", "remote.origin.url"])?;
        let filter = git(
            &repo,
            &["config", "--get", "remote.origin.partialclonefilter"],
        )
        .unwrap_or_else(|_| "blob:none".into());
        git(&wt, &["remote", "add", "upstream", url.trim()])?;
        git(&wt, &["config", "remote.upstream.promisor", "true"])?;
        git(
            &wt,
            &[
                "config",
                "remote.upstream.partialclonefilter",
                filter.trim(),
            ],
        )?;
        git(&wt, &["config", "extensions.partialclone", "upstream"])?;
    }
    let mut commits: Vec<String> = git(
        &repo,
        &[
            "rev-list",
            "--first-parent",
            "-n",
            &args.commits.to_string(),
            "HEAD",
        ],
    )?
    .lines()
    .map(str::to_string)
    .collect();
    commits.reverse();
    anyhow::ensure!(!commits.is_empty(), "no commits");
    // Forward through history, then a long jump back and a long jump forward.
    let mut plan: Vec<(String, String)> = commits
        .iter()
        .enumerate()
        .map(|(i, c)| (format!("forward {}", i + 1), c.clone()))
        .collect();
    if commits.len() > 2 {
        plan.push(("jump back to oldest".into(), commits[0].clone()));
        plan.push((
            "jump forward to newest".into(),
            commits[commits.len() - 1].clone(),
        ));
    }
    let data = scratch.join("data");
    let mut report = HistoryReport {
        repo: repo.to_string_lossy().into(),
        commits: commits.len(),
        ..Default::default()
    };
    let mut prev: Option<String> = None;
    let mut prev_walk: BTreeSet<String> = BTreeSet::new();
    let mut times = Vec::new();
    // One engine for the whole replay, as in the running app watching a repo while the user
    // switches branches. The checkout path never changes, so its index is reused.
    let mut eng = None;
    for (i, (label, commit)) in plan.iter().enumerate() {
        git(&wt, &["checkout", "--quiet", "--force", commit])?;
        let walk: BTreeSet<String> = iso_walk::walk(&wt).into_iter().map(|f| f.rel).collect();
        if eng.is_none() {
            eng = Some(Engine::open_with_data(&wt, &data)?);
        }
        let t = Instant::now();
        let r: IndexReport = eng.as_ref().expect("opened").index()?;
        let ms = t.elapsed().as_millis() as u64;
        let mut step = Step {
            label: label.clone(),
            commit: commit[..12.min(commit.len())].to_string(),
            hashed: r.hashed,
            parsed: r.parsed,
            facts_cache_hits: r.facts_cache_hits,
            added: r.added,
            deleted: r.deleted,
            exports_changed: r.exports_changed,
            dependents_recomputed: r.dependents_recomputed,
            edge_files: r.edge_files,
            tree_changed_files: r.tree_changed_files,
            tree_dirs_visited: r.tree_dirs_visited,
            tree_dirs_total: r.tree_dirs_total,
            ms,
            stages_ms: [r.ms_walk, r.ms_parse, r.ms_resolve, r.ms_edges, r.ms_tree],
            ..Default::default()
        };
        if let Some(p) = &prev {
            // Files git changed that the walker indexes (now or before), excluding no-ops.
            let changed: BTreeSet<String> =
                git(&wt, &["diff", "--name-only", "--no-renames", p, commit])?
                    .lines()
                    .filter(|l| walk.contains(*l) || prev_walk.contains(*l))
                    .map(str::to_string)
                    .collect();
            step.git_changed = changed.len() as u32;
            report.total_git_changed += changed.len() as u64;
            let hashed_ok =
                r.hashed as usize == changed.iter().filter(|p| walk.contains(*p)).count();
            let tree: BTreeSet<String> = r.tree_changed_sample.iter().cloned().collect();
            let tree_ok = r.tree_changed_files as usize == changed.len()
                && (changed.len() > 50 || tree == changed);
            step.exact = hashed_ok && tree_ok;
            if !step.exact {
                step.mismatch = changed
                    .symmetric_difference(&tree)
                    .take(10)
                    .cloned()
                    .collect();
            }
            report.total_hashed += r.hashed as u64;
            report.total_parsed += r.parsed as u64;
            times.push(ms);
        } else {
            step.exact = true;
        }
        if step.exact {
            report.exact_steps += 1;
        }
        let verify =
            i + 1 == plan.len() || (args.verify_every > 0 && (i + 1) % args.verify_every == 0);
        if verify {
            let v = verify_against_cold(&wt, &data, &scratch.join(format!("cold-{i}")))?;
            report.verified_total += 1;
            if v.equal {
                report.verified_equal += 1;
            }
            step.verified = Some(v);
        }
        eprintln!(
            "{:<24} {} {:?} git {:>5} hashed {:>5} parsed {:>5} cache {:>5} deps {:>4} tree {:>5} dirs {:>5}/{:<5} {:>6} ms {}{}",
            step.label,
            step.commit,
            step.stages_ms,
            step.git_changed,
            step.hashed,
            step.parsed,
            step.facts_cache_hits,
            step.dependents_recomputed,
            step.tree_changed_files,
            step.tree_dirs_visited,
            step.tree_dirs_total,
            step.ms,
            if step.exact { "exact" } else { "MISMATCH" },
            step.verified
                .as_ref()
                .map(|v| if v.equal {
                    " · equals cold index"
                } else {
                    " · DIFFERS from cold index"
                })
                .unwrap_or("")
        );
        report.steps.push(step);
        prev = Some(commit.clone());
        prev_walk = walk;
    }
    times.sort_unstable();
    report.median_step_ms = times.get(times.len() / 2).copied().unwrap_or(0);
    let _ = std::fs::remove_dir_all(&scratch);
    Ok(report)
}

/// Index the same checkout cold into a fresh database and compare the two indexes.
fn verify_against_cold(wt: &Path, data: &Path, cold_data: &Path) -> Result<Verify> {
    let cold = Engine::open_with_data(wt, cold_data)?;
    cold.index()?;
    let warm = Engine::open_with_data(wt, data)?;
    let a = digest(&warm)?;
    let b = digest(&cold)?;
    drop(cold);
    let _ = std::fs::remove_dir_all(cold_data);
    let mut v = Verify {
        files: (a.0.len(), b.0.len()),
        symbols: (a.1.len(), b.1.len()),
        imports: (a.2.len(), b.2.len()),
        edges: (a.3.len(), b.3.len()),
        ..Default::default()
    };
    for (name, x, y) in [
        ("file", &a.0, &b.0),
        ("symbol", &a.1, &b.1),
        ("import", &a.2, &b.2),
        ("edge", &a.3, &b.3),
    ] {
        for d in x.symmetric_difference(y).take(5) {
            let side = if x.contains(d) {
                "incremental only"
            } else {
                "cold only"
            };
            v.diffs.push(format!("{name} {side}: {d}"));
        }
    }
    v.equal = v.diffs.is_empty();
    Ok(v)
}

type Digest = (
    BTreeSet<String>,
    BTreeSet<String>,
    BTreeSet<String>,
    BTreeSet<String>,
);

fn digest(e: &Engine) -> Result<Digest> {
    e.index.read(|c| {
        let q = |sql: &str| -> Result<BTreeSet<String>> {
            let mut st = c.prepare(sql)?;
            let rows = st.query_map([], |r| r.get::<_, String>(0))?.collect::<iso_index::rusqlite::Result<BTreeSet<_>>>()?;
            Ok(rows)
        };
        Ok((
            q("SELECT path || ' ' || content_hash || ' ' || lang || ' ' || tier || ' ' || status FROM files")?,
            q("SELECT f.path || ' ' || s.qualified_name || ' ' || s.kind || ' ' || s.start_line || '-' || s.end_line || ' ' || s.exported FROM symbols s JOIN files f ON f.id = s.file_id")?,
            q("SELECT f.path || ' ' || i.specifier || ' ' || i.line || ' -> ' || COALESCE(t.path, 'ext:' || i.external, CASE WHEN i.unresolved = 1 THEN 'unresolved' ELSE 'pending' END) FROM imports i JOIN files f ON f.id = i.file_id LEFT JOIN files t ON t.id = i.resolved_file_id")?,
            q("SELECT f.path || ' ' || printf('%x', e.from_symbol) || ' ' || e.kind || ' ' || e.confidence || ' ' || e.line || ' -> ' || COALESCE(printf('%x', e.to_symbol), e.to_external) FROM edges e JOIN files f ON f.id = e.from_file")?,
        ))
    })
}
