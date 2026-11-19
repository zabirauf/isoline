//! The AI layer as the engine sees it: inputs built from the index, cached output served to
//! file views, and module names and colors written back after a cluster pass.
//!
//! No code leaves the machine unless the user turned AI on for the repo; `estimate` and `run`
//! refuse otherwise. Serving cached output never calls a model.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Result, anyhow, bail};
use iso_ai::{AiEngine, FileInput, FullReport, ModuleInput, Progress};
use iso_core::ipc::{
    AiEstimate, AiScope, ArchFlow, ArchLayer, Architecture, ChangeKind, ChangeReview, Comparison,
    ModuleEdge, ModuleNote, ReviewKind, ReviewNote, ReviewState,
};
use iso_core::{FileFacts, FileId, FlowUnit, Tier};
use iso_index::FileRow;

use crate::Engine;
use crate::view::{AiHook, CachedAi};

/// Serves cached model output to file views.
pub struct AiLayer {
    pub ai: Arc<AiEngine>,
}

impl AiHook for AiLayer {
    fn cached(&self, row: &FileRow, facts: &FileFacts, flows: &[FlowUnit]) -> CachedAi {
        let input = FileInput {
            path: row.path.clone(),
            lang: row.lang,
            content_hash: row.content_hash.clone(),
            source: String::new(),
            facts: facts.clone(),
            flows: flows.to_vec(),
        };
        let c = self.ai.cached_file(&input);
        CachedAi {
            summaries: c.summaries,
            types: c.types,
            overview: c.overview,
        }
    }
}

/// Everything a run or an estimate needs.
pub struct AiInputs {
    pub files: Vec<FileInput>,
    pub modules: Vec<ModuleInput>,
    pub edges: Vec<ModuleEdge>,
    pub colors: HashMap<String, String>,
}

impl Engine {
    /// Build AI inputs for a scope: one file, the file's module, or the whole repo.
    pub fn ai_inputs(self: &Arc<Self>, scope: AiScope, path: Option<&str>) -> Result<AiInputs> {
        let all = self.index.read(iso_index::files_all)?;
        let fm: HashMap<FileId, String> = self
            .index
            .read(iso_index::file_modules)?
            .into_iter()
            .collect();
        let mods = self.index.read(iso_index::modules_all)?;
        let by_path: HashMap<&str, &FileRow> = all.iter().map(|f| (f.path.as_str(), f)).collect();
        let selected: Vec<&FileRow> = match scope {
            AiScope::File => {
                let p = path.ok_or_else(|| anyhow!("a file scope needs a path"))?;
                vec![*by_path.get(p).ok_or_else(|| anyhow!("not indexed: {p}"))?]
            }
            AiScope::Module => {
                let p = path.ok_or_else(|| anyhow!("a module scope needs a file path"))?;
                let row = by_path.get(p).ok_or_else(|| anyhow!("not indexed: {p}"))?;
                let m = fm.get(&row.id).cloned();
                all.iter().filter(|f| fm.get(&f.id) == m.as_ref()).collect()
            }
            AiScope::Repo | AiScope::Architecture => all.iter().collect(),
        };
        // The architecture run reads only cached summaries and symbol names: no source, no
        // call flows, which keeps it fast on large repos.
        let light = scope == AiScope::Architecture;
        let mut files = Vec::new();
        for row in selected.into_iter().filter(|f| f.tier != Tier::Text) {
            let facts = self.facts_for(row)?;
            if facts.symbols.is_empty() {
                continue;
            }
            let (source, flows) = if light {
                (String::new(), Vec::new())
            } else {
                let source = std::fs::read_to_string(self.root.join(&row.path)).unwrap_or_default();
                let flows = self.flow_units_for(row, &source).unwrap_or_default();
                (source, flows)
            };
            files.push(FileInput {
                path: row.path.clone(),
                lang: row.lang,
                content_hash: row.content_hash.clone(),
                source,
                facts,
                flows,
            });
        }
        let path_of: HashMap<FileId, &str> = all.iter().map(|f| (f.id, f.path.as_str())).collect();
        let wanted: Option<std::collections::HashSet<String>> = match scope {
            AiScope::Repo | AiScope::Architecture => None,
            _ => Some(
                files
                    .iter()
                    .filter_map(|f| fm.get(&FileId::of_path(&f.path)).cloned())
                    .collect(),
            ),
        };
        let modules: Vec<ModuleInput> = mods
            .iter()
            .filter(|m| wanted.as_ref().is_none_or(|w| w.contains(&m.id)))
            .map(|m| ModuleInput {
                id: m.id.clone(),
                dir: m.dir.clone(),
                files: fm
                    .iter()
                    .filter(|(_, mid)| *mid == &m.id)
                    .filter_map(|(f, _)| path_of.get(f).map(|p| p.to_string()))
                    .collect(),
            })
            .collect();
        let colors = mods
            .iter()
            .filter(|m| m.source != "cluster")
            .map(|m| (m.id.clone(), m.color.clone()))
            .collect();
        let edges = self.module_map()?.edges;
        Ok(AiInputs {
            files,
            modules,
            edges,
            colors,
        })
    }

    fn require_ai(&self) -> Result<()> {
        if !self.ai_enabled() {
            bail!(
                "AI analysis is off for this repo. Turn it on first; no code leaves the machine until then."
            );
        }
        Ok(())
    }

    pub fn ai_estimate(
        self: &Arc<Self>,
        ai: &AiEngine,
        scope: AiScope,
        path: Option<&str>,
    ) -> Result<AiEstimate> {
        self.require_ai()?;
        let i = self.ai_inputs(scope, path)?;
        Ok(ai.estimate(&i.files, &i.modules, &i.edges, scope).to_ipc())
    }

    /// Run the passes for a scope within a budget, then store module names, roles and colors.
    /// A module the user renamed keeps the user's name and color.
    pub fn ai_run(
        self: &Arc<Self>,
        ai: &AiEngine,
        scope: AiScope,
        path: Option<&str>,
        budget_usd: f64,
        on_progress: &mut dyn FnMut(&Progress),
    ) -> Result<FullReport> {
        self.require_ai()?;
        let i = self.ai_inputs(scope, path)?;
        let report = ai
            .run(
                &i.files,
                &i.modules,
                &i.edges,
                &i.colors,
                scope,
                budget_usd,
                on_progress,
            )
            .map_err(|e| anyhow!("{e}"))?;
        if let Some(c) = &report.cluster {
            let existing: HashMap<String, String> = self
                .index
                .read(iso_index::modules_all)?
                .into_iter()
                .map(|m| (m.id, m.source))
                .collect();
            self.index.write(|tx| {
                for m in c.modules.iter().filter(|m| m.ai) {
                    if existing.get(&m.id).map(String::as_str) == Some("user") {
                        continue;
                    }
                    iso_index::update_module(
                        tx,
                        &m.id,
                        Some(&m.name),
                        Some(&m.role),
                        Some(&m.color),
                        "ai",
                    )?;
                }
                Ok(())
            })?;
        }
        if let Some(o) = report.repo.as_ref().and_then(|r| r.overview.as_ref()) {
            let a = Architecture {
                overview: o.overview.clone(),
                layers: o
                    .layers
                    .iter()
                    .map(|l| ArchLayer {
                        name: l.name.clone(),
                        modules: l.modules.clone(),
                    })
                    .collect(),
                flows: o
                    .flows
                    .iter()
                    .map(|f| ArchFlow {
                        name: f.name.clone(),
                        text: f.text.clone(),
                        modules: f.modules.clone(),
                    })
                    .collect(),
                model: o.model.clone(),
                modules: o.legend.len() as u32,
                written_at: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs()),
            };
            let json = serde_json::to_string(&a)?;
            self.index
                .write(|tx| iso_index::meta_set(tx, ARCHITECTURE_KEY, &json))?;
        }
        Ok(report)
    }

    /// The last architecture overview written for this repo, if any. Reading it never calls
    /// a model.
    pub fn architecture(&self) -> Result<Option<Architecture>> {
        let raw = self
            .index
            .read(|c| iso_index::meta_get(c, ARCHITECTURE_KEY))?;
        Ok(raw.and_then(|s| serde_json::from_str(&s).ok()))
    }
}

/// Index meta key of the stored architecture overview (JSON).
const ARCHITECTURE_KEY: &str = "ai_architecture";

// ------------------------------------------------------------------ change review

/// Upper bound on the changed files whose diffs are read for a review.
const REVIEW_MAX_FILES: usize = 300;
/// Lines of an untracked file sent as its diff.
const REVIEW_NEW_FILE_LINES: usize = 400;

impl Engine {
    /// What the review of a comparison is written from: the comparison's facts, module roles
    /// from the index, and the diff of each changed file from git.
    pub fn review_input(&self, cmp: &Comparison) -> Result<iso_ai::ReviewInput> {
        use iso_ai::{ReviewFile, ReviewInput, ReviewModule};
        let roles: HashMap<String, Option<String>> = self
            .index
            .read(iso_index::modules_all)
            .unwrap_or_default()
            .into_iter()
            .map(|m| (m.id, m.role))
            .collect();
        let name_of: HashMap<&str, &str> = cmp
            .modules
            .iter()
            .map(|m| (m.id.as_str(), m.name.as_str()))
            .collect();
        let title = match &cmp.pr {
            Some(p) => format!("#{} {}", p.number, p.title),
            None => {
                let mut t = format!("{} ← {}", cmp.base.label, cmp.head.label);
                if let Some(s) = &cmp.head.subject {
                    t.push_str(&format!(" ({s})"));
                }
                t
            }
        };
        let base = cmp.merge_base.as_deref().unwrap_or(&cmp.base.sha);
        let head = (cmp.head.sha != iso_git::WORKTREE).then_some(cmp.head.sha.as_str());
        // Lockfiles change in bulk and say little: their diffs are not sent. Paths go to git in
        // batches, so the command line stays short (Windows caps it at 32k characters).
        let paths: Vec<String> = cmp
            .files
            .iter()
            .filter(|f| !f.binary && !is_lockfile(&f.path))
            .take(REVIEW_MAX_FILES)
            .flat_map(|f| std::iter::once(f.path.clone()).chain(f.old_path.clone()))
            .collect();
        let mut patches: HashMap<String, String> = HashMap::new();
        if let Ok(g) = self.git_or_err() {
            for chunk in paths.chunks(100) {
                if let Ok(p) = g.patches(base, head, chunk, 2) {
                    patches.extend(p);
                }
            }
        }
        let sig = |s: &Option<String>| s.as_deref().unwrap_or("").to_string();
        let files = cmp
            .files
            .iter()
            .map(|f| {
                let mut patch = patches.remove(&f.path).unwrap_or_default();
                // Untracked files are not in git's diff: their first lines are the diff.
                if patch.is_empty()
                    && head.is_none()
                    && !is_lockfile(&f.path)
                    && f.change == ChangeKind::Added
                    && !f.binary
                    && let Ok(text) = std::fs::read_to_string(self.root.join(&f.path))
                {
                    patch = text
                        .lines()
                        .take(REVIEW_NEW_FILE_LINES)
                        .map(|l| format!("+{l}\n"))
                        .collect();
                }
                let change = match (f.change, &f.old_path) {
                    (ChangeKind::Renamed, Some(o)) => format!("renamed from {o}"),
                    (ChangeKind::Added, _) => "added".into(),
                    (ChangeKind::Deleted, _) => "deleted".into(),
                    (ChangeKind::Renamed, None) => "renamed".into(),
                    (ChangeKind::Modified, _) => "modified".into(),
                };
                let symbols = f
                    .symbols
                    .iter()
                    .map(|s| {
                        let kind = format!("{:?}", s.kind).to_lowercase();
                        let exp = if s.exported { "exported " } else { "" };
                        match s.change {
                            ChangeKind::Added => {
                                format!("+ {exp}{kind} {}: {}", s.name, sig(&s.signature_after))
                            }
                            ChangeKind::Deleted => {
                                format!("- {exp}{kind} {}: {}", s.name, sig(&s.signature_before))
                            }
                            _ if s.signature_before != s.signature_after => format!(
                                "~ {exp}{kind} {}: signature {} → {}{}",
                                s.name,
                                sig(&s.signature_before),
                                sig(&s.signature_after),
                                if s.callers > 0 {
                                    format!(" ({} callers in other files)", s.callers)
                                } else {
                                    String::new()
                                }
                            ),
                            _ => format!("~ {exp}{kind} {}: body changed", s.name),
                        }
                    })
                    .collect();
                let imports = f
                    .imports
                    .iter()
                    .map(|i| {
                        let sign = if i.change == ChangeKind::Added {
                            "+"
                        } else {
                            "-"
                        };
                        match (&i.file, &i.external) {
                            (Some(p), _) => format!("{sign} {} ({p})", i.specifier),
                            (None, Some(e)) => format!("{sign} {} (package {e})", i.specifier),
                            _ => format!("{sign} {}", i.specifier),
                        }
                    })
                    .collect();
                ReviewFile {
                    path: f.path.clone(),
                    change,
                    module: f.module.as_ref().map(|m| m.id.clone()),
                    additions: f.additions,
                    deletions: f.deletions,
                    symbols,
                    imports,
                    patch,
                }
            })
            .collect();
        let links = cmp
            .module_edges
            .iter()
            .map(|e| {
                let n = |id: &str| {
                    name_of
                        .get(id)
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| self.module_name(id))
                };
                let what = if e.before == 0 {
                    "new link"
                } else if e.after == 0 {
                    "link removed"
                } else {
                    "link changed"
                };
                format!(
                    "{} → {}: {what}, {} → {} file imports",
                    n(&e.from),
                    n(&e.to),
                    e.before,
                    e.after
                )
            })
            .collect();
        let packages = cmp
            .packages
            .iter()
            .map(|p| {
                let files = p.files.len();
                match (p.change, p.elsewhere) {
                    (ChangeKind::Added, 0) => format!("{}: new to the repo, imported by {files} changed files", p.name),
                    (ChangeKind::Added, n) => format!("{}: now also imported by {files} changed files ({n} other files import it)", p.name),
                    (_, 0) => format!("{}: no longer imported anywhere", p.name),
                    (_, n) => format!("{}: no longer imported by {files} changed files ({n} other files still import it)", p.name),
                }
            })
            .collect();
        let impact = cmp
            .impact
            .iter()
            .map(|i| {
                let what = if i.change == ChangeKind::Deleted {
                    "deleted"
                } else {
                    "signature changed"
                };
                format!(
                    "{} in {} ({what}): {} unchanged files use it",
                    i.symbol, i.path, i.caller_files
                )
            })
            .collect();
        Ok(ReviewInput {
            title,
            commits: cmp.commits.iter().map(|c| c.subject.clone()).collect(),
            modules: cmp
                .modules
                .iter()
                .map(|m| ReviewModule {
                    id: m.id.clone(),
                    name: m.name.clone(),
                    dir: m.dir.clone(),
                    role: roles.get(&m.id).cloned().flatten(),
                    files: m.files,
                    additions: m.additions,
                    deletions: m.deletions,
                })
                .collect(),
            links,
            packages,
            impact,
            files,
        })
    }

    fn module_name(&self, id: &str) -> String {
        self.index
            .read(iso_index::modules_all)
            .ok()
            .and_then(|ms| ms.into_iter().find(|m| m.id == id).map(|m| m.name))
            .unwrap_or_else(|| id.to_string())
    }

    /// The cached review of a comparison, else what writing one costs.
    pub fn ai_review_state(&self, ai: &AiEngine, cmp: &Comparison) -> Result<ReviewState> {
        self.require_ai()?;
        let input = self.review_input(cmp)?;
        if let Some(r) = ai.cached_review(&input) {
            return Ok(ReviewState {
                files_without_patch: r.files_without_patch,
                model: r.model.clone(),
                review: Some(review_ipc(r)),
                cost_usd: 0.0,
                tokens_in: 0,
            });
        }
        let e = ai.estimate_review(&input);
        Ok(ReviewState {
            review: None,
            cost_usd: e.pass.cost_usd,
            tokens_in: e.pass.tokens_in,
            model: e.pass.model,
            files_without_patch: e.files_without_patch,
        })
    }

    /// Writes the review of a comparison within a budget (or serves it from the cache).
    pub fn ai_review_run(
        &self,
        ai: &AiEngine,
        cmp: &Comparison,
        budget_usd: f64,
    ) -> Result<ChangeReview> {
        self.require_ai()?;
        let input = self.review_input(cmp)?;
        let (r, _) = ai.review(&input, budget_usd).map_err(|e| anyhow!("{e}"))?;
        Ok(review_ipc(r))
    }
}

fn is_lockfile(path: &str) -> bool {
    matches!(
        path.rsplit('/').next().unwrap_or(path),
        "package-lock.json"
            | "npm-shrinkwrap.json"
            | "yarn.lock"
            | "pnpm-lock.yaml"
            | "bun.lockb"
            | "bun.lock"
            | "Cargo.lock"
            | "go.sum"
            | "poetry.lock"
            | "uv.lock"
            | "Pipfile.lock"
            | "composer.lock"
            | "Gemfile.lock"
            | "flake.lock"
    )
}

fn review_ipc(r: iso_ai::ChangeReview) -> ChangeReview {
    ChangeReview {
        summary: r.summary,
        notes: r
            .notes
            .into_iter()
            .map(|n| ReviewNote {
                kind: match n.kind {
                    iso_ai::ReviewNoteKind::Risk => ReviewKind::Risk,
                    iso_ai::ReviewNoteKind::Check => ReviewKind::Check,
                    iso_ai::ReviewNoteKind::Info => ReviewKind::Info,
                },
                text: n.text,
                path: n.path,
                symbol: n.symbol,
            })
            .collect(),
        modules: r
            .modules
            .into_iter()
            .map(|(id, text)| ModuleNote { id, text })
            .collect(),
        model: r.model,
        files_without_patch: r.files_without_patch,
    }
}
