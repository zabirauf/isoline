//! `iso-cli eval`: one round of the validation loop over `eval/repos.toml`.
//!
//! Each repo entry has either a local `path` or a `url` + `commit` (cloned into `eval/work/`).
//! Remote repos are only cloned with `--fetch`, because the repo list needs a human's approval.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::bench::{BenchArgs, BenchReport, bench};

#[derive(clap::Args, Clone, Debug)]
pub struct EvalArgs {
    /// Repo list.
    #[arg(long, default_value = "eval/repos.toml")]
    pub repos: PathBuf,
    /// Round name; results go to eval/results/<round>.json.
    #[arg(long)]
    pub round: String,
    /// Only these repos (by name).
    #[arg(long)]
    pub only: Vec<String>,
    /// Clone remote repos that are missing from eval/work.
    #[arg(long)]
    pub fetch: bool,
    #[arg(long, default_value_t = 200)]
    pub opens: usize,
    #[arg(long, default_value_t = 50)]
    pub edits: usize,
}

#[derive(Deserialize, Debug, Clone)]
pub struct RepoList {
    pub repo: Vec<RepoEntry>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct RepoEntry {
    pub name: String,
    pub covers: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub commit: Option<String>,
    #[serde(default)]
    pub approved: bool,
}

#[derive(Serialize)]
struct RoundResult {
    round: String,
    machine_cores: usize,
    repos: Vec<RepoResult>,
}

#[derive(Serialize)]
struct RepoResult {
    entry: RepoEntry,
    ok: bool,
    error: Option<String>,
    report: Option<BenchReport>,
}

pub fn run(_data: Option<&Path>, args: EvalArgs) -> Result<()> {
    let text = std::fs::read_to_string(&args.repos)
        .with_context(|| format!("read {}", args.repos.display()))?;
    let list: RepoList = toml::from_str(&text)?;
    let base = args.repos.parent().unwrap_or(Path::new(".")).to_path_buf();
    let mut out = RoundResult {
        round: args.round.clone(),
        machine_cores: std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1),
        repos: vec![],
    };
    for e in list.repo {
        if !args.only.is_empty() && !args.only.contains(&e.name) {
            continue;
        }
        let root = match locate(&base, &e, args.fetch) {
            Ok(r) => r,
            Err(err) => {
                eprintln!("skip {}: {err:#}", e.name);
                out.repos.push(RepoResult {
                    entry: e,
                    ok: false,
                    error: Some(format!("{err:#}")),
                    report: None,
                });
                continue;
            }
        };
        eprintln!("== {} ({})", e.name, root.display());
        let res = bench(
            &root,
            &BenchArgs {
                opens: args.opens,
                edits: args.edits,
                scratch: None,
                out: None,
                seed: 7,
            },
        );
        match res {
            Ok(r) => out.repos.push(RepoResult {
                entry: e,
                ok: true,
                error: None,
                report: Some(r),
            }),
            Err(err) => out.repos.push(RepoResult {
                entry: e,
                ok: false,
                error: Some(format!("{err:#}")),
                report: None,
            }),
        }
    }
    let dir = base.join("results");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.json", args.round));
    std::fs::write(&path, serde_json::to_string_pretty(&out)?)?;
    eprintln!("wrote {}", path.display());
    print_table(&out);
    Ok(())
}

fn locate(base: &Path, e: &RepoEntry, fetch: bool) -> Result<PathBuf> {
    if let Some(p) = &e.path {
        let p = if p.starts_with('~') {
            PathBuf::from(std::env::var("HOME").unwrap_or_default())
                .join(p.trim_start_matches("~/"))
        } else {
            base.join(p)
        };
        anyhow::ensure!(p.is_dir(), "missing local repo {}", p.display());
        return Ok(p);
    }
    let url = e.url.as_ref().context("entry needs path or url")?;
    let dst = base.join("work").join(&e.name);
    if !dst.is_dir() {
        anyhow::ensure!(
            fetch,
            "not cloned; rerun with --fetch once the repo list is approved"
        );
        anyhow::ensure!(e.approved, "not approved in repos.toml");
        let st = std::process::Command::new("git")
            .args(["clone", "--filter=blob:none", "--quiet", url])
            .arg(&dst)
            .status()?;
        anyhow::ensure!(st.success(), "git clone failed");
        if let Some(c) = &e.commit {
            let st = std::process::Command::new("git")
                .arg("-C")
                .arg(&dst)
                .args(["checkout", "--quiet", c])
                .status()?;
            anyhow::ensure!(st.success(), "git checkout {c} failed");
        }
    }
    Ok(dst)
}

fn print_table(r: &RoundResult) {
    eprintln!(
        "\n{:<22} {:>7} {:>9} {:>9} {:>9} {:>8} {:>8} {:>8} {:>7} {:>6} {:>6}",
        "repo",
        "files",
        "lines",
        "Mlines/m",
        "reopen",
        "open95",
        "edit95",
        "unres%",
        "hit",
        "dep",
        "panic"
    );
    for x in &r.repos {
        match &x.report {
            Some(b) => eprintln!(
                "{:<22} {:>7} {:>9} {:>9.2} {:>7}ms {:>6.1}ms {:>6.1}ms {:>7.2}% {:>7.4} {:>6} {:>6}",
                x.entry.name,
                b.files,
                b.lines,
                b.cold_lines_per_min / 1e6,
                b.reopen_ms,
                b.open_file.p95_ms,
                b.edit.p95_ms,
                b.unresolved_imports_pct,
                b.reopen_hit_rate,
                b.edit_dependents_max,
                b.panics
            ),
            None => eprintln!(
                "{:<22} error: {}",
                x.entry.name,
                x.error.clone().unwrap_or_default()
            ),
        }
    }
}
