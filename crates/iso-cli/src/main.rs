//! iso-cli: every Isoline feature, headless. Agents and CI test with this binary and golden
//! files, not by looking at a window.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use iso_engine::{Engine, Event};

mod bench;
mod eval;

#[derive(Parser)]
#[command(name = "iso-cli", version, about = "Isoline, headless")]
struct Cli {
    /// Repository root (defaults to the current directory).
    #[arg(short, long, global = true)]
    repo: Option<PathBuf>,
    /// Data folder for the index (defaults to ISOLINE_DATA_DIR or the app data folder).
    #[arg(long, global = true)]
    data: Option<PathBuf>,
    /// Pretty-print JSON.
    #[arg(long, global = true)]
    pretty: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Index the repo (incremental after the first run) and print the report.
    Index {
        /// Print progress events to stderr.
        #[arg(long)]
        progress: bool,
    },
    /// Print the file view (cards, layers, imports) for one file.
    File { path: String },
    /// Print call flow: one unit, or every unit of the file.
    Flow { path: String, unit: Option<String> },
    /// Extract facts for a file without touching any index.
    Facts { file: PathBuf },
    /// Compute flow units for a file without touching any index.
    Flows { file: PathBuf },
    /// Print source lines.
    Source {
        path: String,
        start: Option<u32>,
        end: Option<u32>,
    },
    /// Print the directory tree.
    Tree,
    /// Search symbols and files.
    Search {
        query: String,
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Dependents and dependencies of a symbol (hex id, or path:qualified_name).
    Impact {
        symbol: String,
        #[arg(long)]
        transitive: bool,
    },
    /// Modules and the edges between them.
    Modules,
    /// Repo summary.
    Summary,
    /// Run installed precise indexers (or import a SCIP file) and upgrade edges.
    Precise {
        /// Import this SCIP file instead of running indexers.
        #[arg(long)]
        scip: Option<PathBuf>,
    },
    /// Benchmark the performance targets on this repo.
    Bench(bench::BenchArgs),
    /// Run the validation loop over eval/repos.toml and write eval/results/<round>.json.
    Eval(eval::EvalArgs),
}

fn print<T: serde::Serialize>(v: &T, pretty: bool) -> Result<()> {
    let s = if pretty {
        serde_json::to_string_pretty(v)?
    } else {
        serde_json::to_string(v)?
    };
    println!("{s}");
    Ok(())
}

pub(crate) fn open(repo: &Path, data: Option<&Path>) -> Result<Arc<Engine>> {
    match data {
        Some(d) => Engine::open_with_data(repo, d),
        None => Engine::open(repo),
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let repo = cli.repo.clone().unwrap_or_else(|| PathBuf::from("."));
    let data = cli.data.as_deref();
    match cli.cmd {
        Cmd::Facts { file } => {
            let src = std::fs::read_to_string(&file)
                .with_context(|| format!("read {}", file.display()))?;
            let lang = iso_core::Language::detect(&file.to_string_lossy(), src.lines().next());
            print(&iso_parse::extract_file(&file, lang, &src), cli.pretty)
        }
        Cmd::Flows { file } => {
            let src = std::fs::read_to_string(&file)
                .with_context(|| format!("read {}", file.display()))?;
            let lang = iso_core::Language::detect(&file.to_string_lossy(), src.lines().next());
            let units = iso_parse::flow_units(&file, lang, &src);
            // Same shape as the prototype's analyze.js: an object keyed by unit id.
            let map: serde_json::Map<String, serde_json::Value> = units
                .into_iter()
                .map(|u| (u.id.clone(), serde_json::to_value(u).unwrap()))
                .collect();
            print(&map, cli.pretty)
        }
        Cmd::Bench(args) => bench::run(&repo, data, args),
        Cmd::Eval(args) => eval::run(data, args),
        cmd => {
            let eng = open(&repo, data)?;
            match cmd {
                Cmd::Index { progress } => {
                    if progress {
                        eng.set_sink(Some(Arc::new(|e| {
                            if let Event::IndexProgress(p) = e {
                                eprintln!(
                                    "{:?} {}/{} {}ms {}",
                                    p.phase,
                                    p.done,
                                    p.total,
                                    p.elapsed_ms,
                                    p.current.unwrap_or_default()
                                );
                            }
                        })));
                    }
                    let t = Instant::now();
                    let rep = eng.index()?;
                    eprintln!(
                        "indexed {} files in {} ms",
                        rep.files,
                        t.elapsed().as_millis()
                    );
                    print(&rep, cli.pretty)
                }
                Cmd::File { path } => print(&eng.file_view(&path)?, cli.pretty),
                Cmd::Flow { path, unit } => match unit {
                    Some(u) => print(&eng.flow(&path, &u)?, cli.pretty),
                    None => {
                        let map: serde_json::Map<String, serde_json::Value> = eng
                            .flows(&path)?
                            .into_iter()
                            .map(|u| (u.id.clone(), serde_json::to_value(u).unwrap()))
                            .collect();
                        print(&map, cli.pretty)
                    }
                },
                Cmd::Source { path, start, end } => {
                    print(&eng.source(&path, start, end)?, cli.pretty)
                }
                Cmd::Tree => print(&eng.tree()?, cli.pretty),
                Cmd::Search { query, limit } => print(&eng.search(&query, limit)?, cli.pretty),
                Cmd::Impact { symbol, transitive } => {
                    let id = match iso_core::SymbolId::from_hex(&symbol) {
                        Some(id) if symbol.len() == 16 => id,
                        _ => {
                            let (path, qn) = symbol
                                .split_once(':')
                                .context("use a hex id or path:qualified_name")?;
                            let view = eng.file_view(path)?;
                            view.functions
                                .iter()
                                .find(|f| f.qualified_name == qn)
                                .map(|f| f.id)
                                .context("no such function")?
                        }
                    };
                    print(&eng.impact(id, transitive)?, cli.pretty)
                }
                Cmd::Modules => print(&eng.module_map()?, cli.pretty),
                Cmd::Summary => print(&eng.summary()?, cli.pretty),
                Cmd::Precise { scip } => match scip {
                    Some(p) => print(&eng.import_scip(&std::fs::read(p)?)?, cli.pretty),
                    None => print(&eng.run_precise()?, cli.pretty),
                },
                Cmd::Facts { .. } | Cmd::Flows { .. } | Cmd::Bench(_) | Cmd::Eval(_) => {
                    unreachable!()
                }
            }
        }
    }
}
