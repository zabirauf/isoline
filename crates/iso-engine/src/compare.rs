//! Compare two revisions in the map's terms: which functions and types were added, deleted or
//! modified, which imports and module dependencies appeared or vanished, and which deleted or
//! re-signatured functions unchanged files still use.
//!
//! Both sides are read from git (the head side may be the working tree) and parsed with the
//! same extractors as the index, so nothing is checked out. Imports resolve with the repo's
//! resolvers, then against each side's own file list for relative paths, so a file that exists
//! on one side only still resolves there. Module dependencies count the changed files' imports
//! on each side on top of the unchanged files' imports from the index.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use iso_core::ipc::{
    ChangeKind, CompareRev, CompareTotals, Comparison, FileChange, FileDiffText, FileRef,
    ImpactItem, ImportChange, LineRange, ModuleChange, ModuleEdgeChange, ModuleRef, PackageChange,
    SymbolChange,
};
use iso_core::{FileFacts, FileId, ImportFact, Language, SymbolFact, SymbolId, SymbolKind, Tier};
use iso_git::{RawChange, WORKTREE};
use iso_index::ModuleRow;
use iso_resolve::{Resolution, Resolvers};
use rayon::prelude::*;

use crate::Engine;

/// Files whose symbols are compared; the rest are listed with line counts only.
const MAX_ANALYZED: usize = 3000;
const MAX_TEXT_BYTES: usize = 4_000_000;
const MAX_CALLERS: usize = 50;
const MAX_IMPACT: usize = 200;

/// Symbols a reader cares about in a comparison. Fields, impl blocks and plain variables are
/// noise next to the functions and types they belong to.
fn tracked(s: &SymbolFact) -> bool {
    use SymbolKind::*;
    match s.kind {
        Function | Method | Class | Interface | Type | Enum | Struct | Trait => true,
        Constant | Variable => s.exported,
        _ => false,
    }
}

fn is_container(k: SymbolKind) -> bool {
    matches!(
        k,
        SymbolKind::Class
            | SymbolKind::Struct
            | SymbolKind::Trait
            | SymbolKind::Interface
            | SymbolKind::Enum
            | SymbolKind::Impl
    )
}

fn range(s: &SymbolFact) -> LineRange {
    LineRange {
        start: s.start_line,
        end: s.end_line,
    }
}

/// Callees per enclosing function: as written when short (`api.getTree`), else the called
/// name alone (a chained call's whole text says less than its last name).
fn calls_by_caller(f: &FileFacts) -> HashMap<&str, BTreeSet<&str>> {
    let mut m: HashMap<&str, BTreeSet<&str>> = HashMap::new();
    for c in &f.calls {
        if let Some(caller) = &c.caller {
            let plain =
                c.callee.len() <= 40 && !c.callee.contains(|ch: char| "([`{ \"'".contains(ch));
            let label = if plain {
                c.callee.as_str()
            } else {
                c.name.as_str()
            };
            m.entry(caller.as_str()).or_default().insert(label);
        }
    }
    m
}

/// A changed declaration head that can break callers: a value's head carries the value itself.
fn api_kind(k: SymbolKind) -> bool {
    !matches!(k, SymbolKind::Constant | SymbolKind::Variable)
}

/// Added, deleted and modified symbols between two versions of one file, in file order.
pub fn diff_symbols(before: &FileFacts, after: &FileFacts) -> Vec<SymbolChange> {
    let key = |s: &SymbolFact| (s.qualified_name.clone(), s.kind);
    let b: HashMap<_, &SymbolFact> = before
        .symbols
        .iter()
        .filter(|s| tracked(s))
        .map(|s| (key(s), s))
        .collect();
    let a: HashMap<_, &SymbolFact> = after
        .symbols
        .iter()
        .filter(|s| tracked(s))
        .map(|s| (key(s), s))
        .collect();
    let calls_b = calls_by_caller(before);
    let calls_a = calls_by_caller(after);
    let empty = BTreeSet::new();
    let mut out: Vec<SymbolChange> = Vec::new();
    for (k, s) in &a {
        match b.get(k) {
            None => out.push(SymbolChange {
                name: s.qualified_name.clone(),
                kind: s.kind,
                change: ChangeKind::Added,
                exported: s.exported,
                signature_before: None,
                signature_after: Some(s.signature.clone()),
                before: None,
                after: Some(range(s)),
                calls_added: Vec::new(),
                calls_deleted: Vec::new(),
                callers: 0,
            }),
            Some(old) if old.body_hash != s.body_hash || old.signature != s.signature => {
                let cb = calls_b.get(old.qualified_name.as_str()).unwrap_or(&empty);
                let ca = calls_a.get(s.qualified_name.as_str()).unwrap_or(&empty);
                out.push(SymbolChange {
                    name: s.qualified_name.clone(),
                    kind: s.kind,
                    change: ChangeKind::Modified,
                    exported: s.exported || old.exported,
                    signature_before: Some(old.signature.clone()),
                    signature_after: Some(s.signature.clone()),
                    before: Some(range(old)),
                    after: Some(range(s)),
                    calls_added: ca.difference(cb).map(|c| c.to_string()).collect(),
                    calls_deleted: cb.difference(ca).map(|c| c.to_string()).collect(),
                    callers: 0,
                });
            }
            Some(_) => {}
        }
    }
    for (k, s) in &b {
        if !a.contains_key(k) {
            out.push(SymbolChange {
                name: s.qualified_name.clone(),
                kind: s.kind,
                change: ChangeKind::Deleted,
                exported: s.exported,
                signature_before: Some(s.signature.clone()),
                signature_after: None,
                before: Some(range(s)),
                after: None,
                calls_added: Vec::new(),
                calls_deleted: Vec::new(),
                callers: 0,
            });
        }
    }
    // A class whose only change is inside its methods is not a change of its own.
    let changed_children: HashSet<String> = out
        .iter()
        .filter_map(|c| c.name.rsplit_once('.').map(|(p, _)| p.to_string()))
        .collect();
    out.retain(|c| {
        !(c.change == ChangeKind::Modified
            && is_container(c.kind)
            && c.signature_before == c.signature_after
            && changed_children.contains(&c.name))
    });
    out.sort_by_key(|c| {
        let r = c.after.as_ref().or(c.before.as_ref());
        (r.map(|r| r.start).unwrap_or(0), c.name.clone())
    });
    out
}

/// A relative import against one side's file list (JS and TS conventions), for files that
/// exist on that side only.
fn relative_on_side(from: &str, spec: &str, files: &HashSet<String>) -> Option<String> {
    if !spec.starts_with('.') {
        return None;
    }
    let base = iso_resolve::join_rel(iso_resolve::parent_dir(from), spec);
    let stem = base
        .strip_suffix(".js")
        .or_else(|| base.strip_suffix(".jsx"))
        .or_else(|| base.strip_suffix(".mjs"))
        .unwrap_or(&base)
        .to_string();
    let mut cands = vec![base.clone()];
    for ext in [
        "ts", "tsx", "d.ts", "js", "jsx", "mjs", "cjs", "mts", "cts", "css", "scss",
    ] {
        cands.push(format!("{stem}.{ext}"));
    }
    for ext in ["ts", "tsx", "js", "jsx"] {
        cands.push(format!("{base}/index.{ext}"));
    }
    cands.into_iter().find(|c| files.contains(c))
}

fn resolve_on_side(
    res: &Resolvers,
    files: &HashSet<String>,
    lang: Language,
    from: &str,
    imp: &ImportFact,
) -> Resolution {
    match res.resolve(lang, from, imp) {
        Resolution::File(f) if files.contains(&f) => Resolution::File(f),
        Resolution::External(e) => Resolution::External(e),
        _ => match relative_on_side(from, &imp.specifier, files) {
            Some(f) => Resolution::File(f),
            None => Resolution::Unresolved,
        },
    }
}

/// One side of a file: its path there and its facts.
struct Side {
    path: String,
    facts: FileFacts,
    targets: HashSet<String>,
    resolved: HashMap<String, Resolution>,
}

fn text_of(bytes: &[u8]) -> Option<String> {
    if bytes.len() > MAX_TEXT_BYTES || bytes.iter().take(8000).any(|&b| b == 0) {
        return None;
    }
    Some(String::from_utf8_lossy(bytes).into_owned())
}

fn change_kind(c: char) -> ChangeKind {
    match c {
        'A' => ChangeKind::Added,
        'D' => ChangeKind::Deleted,
        'R' => ChangeKind::Renamed,
        _ => ChangeKind::Modified,
    }
}

struct Modules {
    rows: Vec<ModuleRow>,
    of_file: HashMap<FileId, String>,
}

impl Modules {
    fn of(&self, path: &str) -> Option<&ModuleRow> {
        let id = self
            .of_file
            .get(&FileId::of_path(path))
            .cloned()
            .or_else(|| {
                self.rows
                    .iter()
                    .filter(|m| m.dir.is_empty() || path.starts_with(&format!("{}/", m.dir)))
                    .max_by_key(|m| m.dir.len())
                    .map(|m| m.id.clone())
            })?;
        self.rows.iter().find(|m| m.id == id)
    }
}

/// How a revision reads in the summary: a full commit id is shortened, names stay as typed.
fn rev_label(spec: &str) -> String {
    if spec.len() >= 40 && spec.bytes().all(|b| b.is_ascii_hexdigit()) {
        spec[..7].to_string()
    } else {
        spec.to_string()
    }
}

impl Engine {
    /// Compare `base` with `head` (`worktree` for uncommitted changes), or a pull request.
    /// With `merge_base`, the base side is where the two histories split, as on GitHub.
    pub fn compare(
        self: &Arc<Self>,
        base: &str,
        head: &str,
        merge_base: bool,
        pr: Option<u32>,
    ) -> Result<Comparison> {
        let t0 = Instant::now();
        let git = self.git_or_err()?;
        let (pr_info, base_sha, head_sha, base_rev, head_rev) = match pr {
            Some(n) => {
                let (p, b, h) = git.pull_request(n)?;
                let base_rev = CompareRev {
                    spec: format!("pr:{n}"),
                    sha: b.clone(),
                    label: p.base_ref.clone(),
                    subject: git.subject(&b),
                };
                let head_rev = CompareRev {
                    spec: format!("pr:{n}"),
                    sha: h.clone(),
                    label: format!("#{n} · {}", p.head_ref),
                    subject: Some(p.title.clone()),
                };
                (Some(p), b, Some(h), base_rev, head_rev)
            }
            None => {
                let b = git.rev_parse(base)?;
                let h = if head == WORKTREE {
                    None
                } else {
                    Some(git.rev_parse(head)?)
                };
                let base_rev = CompareRev {
                    spec: base.to_string(),
                    sha: b.clone(),
                    label: rev_label(base),
                    subject: git.subject(&b),
                };
                let head_rev = CompareRev {
                    spec: head.to_string(),
                    sha: h.clone().unwrap_or_else(|| WORKTREE.to_string()),
                    label: if h.is_none() {
                        "Working tree".to_string()
                    } else {
                        rev_label(head)
                    },
                    subject: h.as_deref().and_then(|h| git.subject(h)),
                };
                (None, b, h, base_rev, head_rev)
            }
        };
        let head_commit = match &head_sha {
            Some(h) => h.clone(),
            None => git.rev_parse("HEAD")?,
        };
        let (from, mb) = if merge_base {
            let m = git.merge_base(&base_sha, &head_commit)?;
            (m.clone(), Some(m))
        } else {
            (base_sha.clone(), None)
        };
        let raw = git.diff(&from, head_sha.as_deref())?;
        let commits = git
            .log(&["-n", "200", &format!("{from}..{head_commit}")])
            .unwrap_or_default();

        let mut cmp = self.analyze(&from, head_sha.as_deref(), &raw)?;
        cmp.base = base_rev;
        cmp.head = head_rev;
        cmp.merge_base = mb;
        cmp.pr = pr_info;
        cmp.commits = commits;
        cmp.elapsed_ms = t0.elapsed().as_millis() as u64;
        Ok(cmp)
    }

    fn analyze(&self, from: &str, head: Option<&str>, raw: &[RawChange]) -> Result<Comparison> {
        let git = self.git_or_err()?;
        let analyzed: Vec<&RawChange> = raw
            .iter()
            .filter(|c| !c.binary && Language::detect(&c.path, None).tier() != Tier::Text)
            .take(MAX_ANALYZED)
            .collect();
        let truncated = raw
            .iter()
            .filter(|c| !c.binary && Language::detect(&c.path, None).tier() != Tier::Text)
            .count()
            > MAX_ANALYZED;

        // ---- read both sides
        let base_paths: Vec<String> = analyzed
            .iter()
            .filter(|c| c.status != 'A')
            .map(|c| c.old_path.clone().unwrap_or_else(|| c.path.clone()))
            .collect();
        let head_paths: Vec<String> = analyzed
            .iter()
            .filter(|c| c.status != 'D')
            .map(|c| c.path.clone())
            .collect();
        let base_blobs = git.read_blobs(from, &base_paths)?;
        let head_blobs = match head {
            Some(h) => git.read_blobs(h, &head_paths)?,
            None => head_paths
                .iter()
                .filter_map(|p| {
                    std::fs::read(self.root.join(p))
                        .ok()
                        .map(|b| (p.clone(), b))
                })
                .collect(),
        };
        let base_files: HashSet<String> = git.files_at(Some(from))?.into_iter().collect();
        let head_files: HashSet<String> = git.files_at(head)?.into_iter().collect();

        // ---- resolvers: the engine's own, built from the index on first use
        let resolvers = {
            let cached = self
                .state
                .lock()
                .resolvers
                .as_ref()
                .map(|(_, r)| Arc::clone(r));
            match cached {
                Some(r) => r,
                None => {
                    let rows = self.index.read(iso_index::files_all)?;
                    self.resolvers(&rows, &[], &[])
                }
            }
        };

        let side = |path: &str, bytes: Option<&Vec<u8>>, files: &HashSet<String>| -> Option<Side> {
            let text = text_of(bytes?)?;
            let lang = Language::detect(path, text.lines().next());
            let facts = iso_parse::extract_file(Path::new(path), lang, &text);
            let mut targets = HashSet::new();
            let mut resolved = HashMap::new();
            for imp in &facts.imports {
                let r = resolve_on_side(&resolvers, files, lang, path, imp);
                if let Resolution::File(f) = &r
                    && f != path
                {
                    targets.insert(f.clone());
                }
                resolved.insert(imp.specifier.clone(), r);
            }
            Some(Side {
                path: path.to_string(),
                facts,
                targets,
                resolved,
            })
        };

        let sides: Vec<(Option<Side>, Option<Side>)> = analyzed
            .par_iter()
            .map(|c| {
                let bp = c.old_path.clone().unwrap_or_else(|| c.path.clone());
                let b = (c.status != 'A')
                    .then(|| side(&bp, base_blobs.get(&bp), &base_files))
                    .flatten();
                let a = (c.status != 'D')
                    .then(|| side(&c.path, head_blobs.get(&c.path), &head_files))
                    .flatten();
                (b, a)
            })
            .collect();
        let by_path: HashMap<&str, &(Option<Side>, Option<Side>)> = analyzed
            .iter()
            .zip(&sides)
            .map(|(c, s)| (c.path.as_str(), s))
            .collect();

        // ---- modules
        let modules = Modules {
            rows: self.index.read(iso_index::modules_all)?,
            of_file: self
                .index
                .read(iso_index::file_modules)?
                .into_iter()
                .collect(),
        };
        let mod_ref = |path: &str| {
            modules.of(path).map(|m| ModuleRef {
                id: m.id.clone(),
                name: m.name.clone(),
                color: m.color.clone(),
            })
        };

        // ---- per file
        let mut totals = CompareTotals {
            files: raw.len() as u32,
            added: 0,
            modified: 0,
            deleted: 0,
            renamed: 0,
            additions: 0,
            deletions: 0,
            symbols_added: 0,
            symbols_deleted: 0,
            symbols_modified: 0,
            signatures_changed: 0,
            imports_added: 0,
            imports_deleted: 0,
        };
        let mut files = Vec::with_capacity(raw.len());
        for c in raw {
            let kind = change_kind(c.status);
            match kind {
                ChangeKind::Added => totals.added += 1,
                ChangeKind::Deleted => totals.deleted += 1,
                ChangeKind::Renamed => totals.renamed += 1,
                ChangeKind::Modified => totals.modified += 1,
            }
            totals.additions += c.additions;
            totals.deletions += c.deletions;
            let lang = Language::detect(&c.path, None);
            let empty = FileFacts::default();
            let (mut symbols, imports) = match by_path.get(c.path.as_str()) {
                Some((b, a)) => {
                    let bf = b.as_ref().map(|s| &s.facts).unwrap_or(&empty);
                    let af = a.as_ref().map(|s| &s.facts).unwrap_or(&empty);
                    let syms = diff_symbols(bf, af);
                    let bs: BTreeSet<&str> =
                        bf.imports.iter().map(|i| i.specifier.as_str()).collect();
                    let as_: BTreeSet<&str> =
                        af.imports.iter().map(|i| i.specifier.as_str()).collect();
                    let mut imps = Vec::new();
                    let res = |s: Option<&Side>, spec: &str| {
                        s.and_then(|s| s.resolved.get(spec).cloned())
                            .unwrap_or(Resolution::Unresolved)
                    };
                    let mk = |change: ChangeKind, spec: &str, r: Resolution| {
                        let (file, external) = match r {
                            Resolution::File(f) => (Some(f), None),
                            Resolution::External(e) => (None, Some(e)),
                            Resolution::Unresolved => (None, None),
                        };
                        ImportChange {
                            change,
                            specifier: spec.to_string(),
                            file,
                            external,
                        }
                    };
                    for s in as_.difference(&bs) {
                        imps.push(mk(ChangeKind::Added, s, res(a.as_ref(), s)));
                    }
                    for s in bs.difference(&as_) {
                        imps.push(mk(ChangeKind::Deleted, s, res(b.as_ref(), s)));
                    }
                    (syms, imps)
                }
                None => (Vec::new(), Vec::new()),
            };
            // Call sites in other files, from the index (the symbol's file as indexed now).
            let _ = self.index.read(|conn| {
                for s in &mut symbols {
                    let p = if s.change == ChangeKind::Deleted {
                        c.old_path.as_deref().unwrap_or(&c.path)
                    } else {
                        &c.path
                    };
                    let id = SymbolId::new(p, &s.name, s.kind);
                    s.callers = iso_index::external_caller_count(conn, id, FileId::of_path(p))?;
                }
                Ok(())
            });
            for s in &symbols {
                match s.change {
                    ChangeKind::Added => totals.symbols_added += 1,
                    ChangeKind::Deleted => totals.symbols_deleted += 1,
                    _ => {
                        totals.symbols_modified += 1;
                        if api_kind(s.kind) && s.signature_before != s.signature_after {
                            totals.signatures_changed += 1;
                        }
                    }
                }
            }
            for i in &imports {
                if i.change == ChangeKind::Added {
                    totals.imports_added += 1;
                } else {
                    totals.imports_deleted += 1;
                }
            }
            files.push(FileChange {
                path: c.path.clone(),
                old_path: c.old_path.clone(),
                change: kind,
                lang,
                tier: lang.tier(),
                additions: c.additions,
                deletions: c.deletions,
                binary: c.binary,
                module: mod_ref(&c.path).or_else(|| c.old_path.as_deref().and_then(mod_ref)),
                symbols,
                imports,
            });
        }

        // ---- modules touched
        let mut mods: HashMap<String, ModuleChange> = HashMap::new();
        for f in &files {
            let Some(m) = &f.module else { continue };
            let row = modules.rows.iter().find(|r| r.id == m.id);
            let e = mods.entry(m.id.clone()).or_insert_with(|| ModuleChange {
                id: m.id.clone(),
                name: m.name.clone(),
                color: m.color.clone(),
                dir: row.map(|r| r.dir.clone()).unwrap_or_default(),
                files: 0,
                additions: 0,
                deletions: 0,
                symbols: 0,
            });
            e.files += 1;
            e.additions += f.additions;
            e.deletions += f.deletions;
            e.symbols += f.symbols.len() as u32;
        }
        let mut modules_out: Vec<ModuleChange> = mods.into_values().collect();
        modules_out.sort_by(|a, b| {
            (b.additions + b.deletions)
                .cmp(&(a.additions + a.deletions))
                .then_with(|| a.name.cmp(&b.name))
        });

        // ---- module dependencies: unchanged files from the index, changed files per side
        let changed_ids: HashSet<FileId> = raw
            .iter()
            .flat_map(|c| std::iter::once(&c.path).chain(c.old_path.as_ref()))
            .map(|p| FileId::of_path(p))
            .collect();
        let edges = self.index.read(iso_index::file_import_edges)?;
        let paths_by_id: HashMap<FileId, String> = self
            .index
            .read(iso_index::files_all)?
            .into_iter()
            .map(|r| (r.id, r.path))
            .collect();
        let mid = |p: &str| modules.of(p).map(|m| m.id.clone());
        let mut unchanged: HashMap<(String, String), u32> = HashMap::new();
        let mut touched_pairs: HashSet<(String, String)> = HashSet::new();
        let mut before: HashMap<(String, String), u32> = HashMap::new();
        let mut after: HashMap<(String, String), u32> = HashMap::new();
        for (b, a) in &sides {
            for (s, acc) in [(b, &mut before), (a, &mut after)] {
                let Some(s) = s else { continue };
                let Some(fm) = mid(&s.path) else { continue };
                for t in &s.targets {
                    if let Some(tm) = mid(t)
                        && tm != fm
                    {
                        *acc.entry((fm.clone(), tm.clone())).or_default() += 1;
                        touched_pairs.insert((fm.clone(), tm));
                    }
                }
            }
        }
        for (f, t) in edges {
            if changed_ids.contains(&f) {
                continue;
            }
            let (Some(fp), Some(tp)) = (paths_by_id.get(&f), paths_by_id.get(&t)) else {
                continue;
            };
            if let (Some(fm), Some(tm)) = (mid(fp), mid(tp))
                && fm != tm
            {
                let k = (fm, tm);
                if touched_pairs.contains(&k) {
                    *unchanged.entry(k).or_default() += 1;
                }
            }
        }
        let mut module_edges: Vec<ModuleEdgeChange> = touched_pairs
            .into_iter()
            .filter_map(|k| {
                let u = unchanged.get(&k).copied().unwrap_or(0);
                let b = u + before.get(&k).copied().unwrap_or(0);
                let a = u + after.get(&k).copied().unwrap_or(0);
                (a != b).then_some(ModuleEdgeChange {
                    from: k.0,
                    to: k.1,
                    before: b,
                    after: a,
                })
            })
            .collect();
        module_edges.sort_by(|x, y| (&x.from, &x.to).cmp(&(&y.from, &y.to)));

        // ---- impact: deleted or re-signatured exported symbols unchanged files still use
        let impact = self.impact_of(&files, &changed_ids)?;
        let packages = self.package_changes(&files);

        Ok(Comparison {
            base: CompareRev {
                spec: String::new(),
                sha: from.to_string(),
                label: String::new(),
                subject: None,
            },
            head: CompareRev {
                spec: String::new(),
                sha: head.unwrap_or(WORKTREE).to_string(),
                label: String::new(),
                subject: None,
            },
            merge_base: None,
            pr: None,
            files,
            totals,
            modules: modules_out,
            module_edges,
            impact,
            packages,
            commits: Vec::new(),
            elapsed_ms: 0,
            truncated,
        })
    }

    fn impact_of(
        &self,
        files: &[FileChange],
        changed: &HashSet<FileId>,
    ) -> Result<Vec<ImpactItem>> {
        let mut out = Vec::new();
        self.index.read(|conn| {
            for f in files {
                let base_path = f.old_path.as_deref().unwrap_or(&f.path);
                for s in &f.symbols {
                    let breaking = s.exported
                        && match s.change {
                            ChangeKind::Deleted => true,
                            ChangeKind::Modified => {
                                api_kind(s.kind) && s.signature_before != s.signature_after
                            }
                            _ => false,
                        };
                    if !breaking || out.len() >= MAX_IMPACT {
                        continue;
                    }
                    let mut callers: Vec<FileRef> = Vec::new();
                    let mut seen = HashSet::new();
                    // The symbol as indexed: its call edges.
                    let id = SymbolId::new(base_path, &s.name, s.kind);
                    if iso_index::symbol_by_id(conn, id)?.is_some() {
                        for e in iso_index::edges_to(conn, id)? {
                            if changed.contains(&e.from_file) || !seen.insert(e.from_file) {
                                continue;
                            }
                            if let Some(r) = iso_index::file_by_id(conn, e.from_file)? {
                                callers.push(FileRef {
                                    path: r.path,
                                    line: Some(e.line),
                                });
                            }
                        }
                    }
                    // Gone from the index (the head side is checked out): files that import
                    // the name from its file.
                    let by_import = callers.is_empty();
                    if by_import {
                        let top = s.name.split('.').next().unwrap_or(&s.name);
                        for (imp_file, _) in
                            iso_index::importers_of(conn, FileId::of_path(base_path))?
                        {
                            if changed.contains(&imp_file) || !seen.insert(imp_file) {
                                continue;
                            }
                            let rows = iso_index::imports_for_file(conn, imp_file)?;
                            let hit = rows.iter().find(|i| {
                                i.resolved == Some(FileId::of_path(base_path))
                                    && i.names.iter().any(|n| n.name == top || n.name == "*")
                            });
                            if let (Some(i), Some(r)) =
                                (hit, iso_index::file_by_id(conn, imp_file)?)
                            {
                                callers.push(FileRef {
                                    path: r.path,
                                    line: Some(i.line),
                                });
                            }
                        }
                    }
                    if callers.is_empty() {
                        continue;
                    }
                    callers.sort_by(|a, b| a.path.cmp(&b.path));
                    let caller_files = callers.len() as u32;
                    callers.truncate(MAX_CALLERS);
                    out.push(ImpactItem {
                        symbol: s.name.clone(),
                        path: base_path.to_string(),
                        change: s.change,
                        signature_before: s.signature_before.clone(),
                        signature_after: s.signature_after.clone(),
                        callers,
                        caller_files,
                        by_import,
                    });
                }
            }
            Ok(())
        })?;
        out.sort_by(|a, b| {
            b.caller_files
                .cmp(&a.caller_files)
                .then_with(|| a.symbol.cmp(&b.symbol))
        });
        Ok(out)
    }

    /// Packages that changed files start or stop importing, with how many unchanged files
    /// import each (from the index).
    fn package_changes(&self, files: &[FileChange]) -> Vec<PackageChange> {
        let mut by: BTreeMap<(String, bool), BTreeSet<String>> = BTreeMap::new();
        for f in files {
            for i in &f.imports {
                if let Some(p) = &i.external {
                    let added = i.change == ChangeKind::Added;
                    by.entry((p.clone(), added))
                        .or_default()
                        .insert(f.path.clone());
                }
            }
        }
        // A package added in one changed file and deleted in another moved: not a change.
        let names: Vec<String> = by
            .keys()
            .filter(|(n, added)| !by.contains_key(&(n.clone(), !added)))
            .map(|(n, _)| n.clone())
            .collect();
        let changed: HashSet<&str> = files
            .iter()
            .flat_map(|f| std::iter::once(f.path.as_str()).chain(f.old_path.as_deref()))
            .collect();
        let users = self
            .index
            .read(|conn| iso_index::files_importing_packages(conn, &names))
            .unwrap_or_default();
        let mut out: Vec<PackageChange> = by
            .into_iter()
            .filter(|((n, _), _)| names.contains(n))
            .map(|((name, added), fs)| {
                let elsewhere = users.get(&name).map_or(0, |v| {
                    v.iter().filter(|p| !changed.contains(p.as_str())).count()
                }) as u32;
                PackageChange {
                    name,
                    change: if added {
                        ChangeKind::Added
                    } else {
                        ChangeKind::Deleted
                    },
                    files: fs.into_iter().collect(),
                    elsewhere,
                }
            })
            .collect();
        // New to the repo first, then by how many files.
        out.sort_by(|a, b| {
            (a.elsewhere > 0)
                .cmp(&(b.elsewhere > 0))
                .then(b.files.len().cmp(&a.files.len()))
                .then(a.name.cmp(&b.name))
        });
        out
    }

    /// Both versions of one file for the diff view. `head` may be `worktree`.
    pub fn compare_file(
        &self,
        base: &str,
        head: &str,
        path: &str,
        old_path: Option<&str>,
    ) -> Result<FileDiffText> {
        let git = self.git_or_err()?;
        let bp = old_path.unwrap_or(path).to_string();
        let before = git
            .read_blobs(base, std::slice::from_ref(&bp))?
            .remove(&bp)
            .and_then(|b| text_of(&b));
        let after = if head == WORKTREE {
            std::fs::read(self.root.join(path))
                .ok()
                .and_then(|b| text_of(&b))
        } else {
            git.read_blobs(head, &[path.to_string()])?
                .remove(path)
                .and_then(|b| text_of(&b))
        };
        let first = after
            .as_deref()
            .or(before.as_deref())
            .and_then(|t| t.lines().next());
        let monaco_lang = Language::detect(path, first).monaco_id().to_string();
        Ok(FileDiffText {
            path: path.to_string(),
            old_path: old_path.map(str::to_string),
            before,
            after,
            monaco_lang,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(src: &str) -> FileFacts {
        iso_parse::extract_file(Path::new("a.ts"), Language::TypeScript, src)
    }

    #[test]
    fn full_commit_ids_are_shortened_in_labels() {
        assert_eq!(
            rev_label("a17feb9195400d3d37c92e9ad8805e7196daf90c"),
            "a17feb9"
        );
        assert_eq!(rev_label("origin/main"), "origin/main");
        assert_eq!(rev_label("HEAD~3"), "HEAD~3");
        assert_eq!(rev_label("deadbeef"), "deadbeef");
    }

    #[test]
    fn symbol_changes() {
        let before = facts(
            "export function keep() { return 1; }\n\
             export function gone() { return 2; }\n\
             export function edit(a: number) { return helper(a); }\n\
             function helper(x: number) { return x; }\n\
             export class C {\n  m() { return 1; }\n  n() { return 2; }\n}\n",
        );
        let after = facts(
            "export function keep() { return 1; }\n\
             export function edit(a: number, b: string) { return other(a, b); }\n\
             function helper(x: number) { return x; }\n\
             function other(x: number, y: string) { return x; }\n\
             export class C {\n  m() { return 10; }\n  n() { return 2; }\n}\n",
        );
        let ch = diff_symbols(&before, &after);
        let names: Vec<(&str, ChangeKind)> =
            ch.iter().map(|c| (c.name.as_str(), c.change)).collect();
        assert!(names.contains(&("gone", ChangeKind::Deleted)), "{names:?}");
        assert!(names.contains(&("other", ChangeKind::Added)), "{names:?}");
        assert!(names.contains(&("edit", ChangeKind::Modified)), "{names:?}");
        assert!(names.contains(&("C.m", ChangeKind::Modified)), "{names:?}");
        // Unchanged symbols are absent, and the class is not listed for its method's change.
        assert!(
            !names
                .iter()
                .any(|(n, _)| *n == "keep" || *n == "helper" || *n == "C.n" || *n == "C"),
            "{names:?}"
        );
        let edit = ch.iter().find(|c| c.name == "edit").unwrap();
        assert_ne!(edit.signature_before, edit.signature_after);
        assert_eq!(edit.calls_added, vec!["other".to_string()]);
        assert_eq!(edit.calls_deleted, vec!["helper".to_string()]);
    }

    #[test]
    fn relative_imports_resolve_against_one_side() {
        let files: HashSet<String> = ["src/a.ts", "src/lib/index.ts", "src/b.tsx"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            relative_on_side("src/x.ts", "./a.js", &files).as_deref(),
            Some("src/a.ts")
        );
        assert_eq!(
            relative_on_side("src/x.ts", "./lib", &files).as_deref(),
            Some("src/lib/index.ts")
        );
        assert_eq!(
            relative_on_side("src/sub/x.ts", "../b", &files).as_deref(),
            Some("src/b.tsx")
        );
        assert_eq!(relative_on_side("src/x.ts", "react", &files), None);
    }
}
