//! Indexing: the full run and the incremental run share one pipeline that stops as early as
//! it can.
//!
//! 1. Detect: walk (or the watcher's paths), then a size + mtime check against the index.
//! 2. Reparse only files whose content hash changed; facts come from the derived cache when the
//!    same content was seen before (branch switches back and forth cost nothing).
//! 3. Cut off early: a file whose exports hash is unchanged leaves its importers alone.
//! 4. Re-resolve imports only for changed files, and for files whose imports pointed at
//!    added or deleted files.
//! 5. Rebuild cross-file edges only for changed files and importers of files whose exports
//!    changed.
//! 6. Store the Merkle tree and recluster modules when the import graph changed.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Instant;

use anyhow::Result;
use iso_cache::{exports_hash, facts_hash};
use iso_core::cache::{DerivedKind, EXTRACTOR_VERSION};
use iso_core::ipc::{IndexPhase, IndexProgress, RepoSummary};
use iso_core::jobs::{JobKind, Priority};
use iso_core::{FileFacts, FileId, Language, Tier};
use iso_index::{FileRow, ResolvedTo};
use iso_resolve::{RepoFiles, Resolution, Resolvers};
use iso_walk::WalkedFile;
use rayon::prelude::*;
use serde::Serialize;

use crate::{Engine, Event};

/// What one index run did. The eval loop records these.
#[derive(Clone, Debug, Default, Serialize)]
pub struct IndexReport {
    pub files: u32,
    pub lines: u64,
    /// Files read from disk and hashed.
    pub hashed: u32,
    /// Files run through an extractor.
    pub parsed: u32,
    /// Files whose facts came from the derived cache instead of the extractor.
    pub facts_cache_hits: u32,
    /// Files skipped by the size + mtime check or by an equal content hash.
    pub unchanged: u32,
    pub added: u32,
    pub deleted: u32,
    /// Files whose facts changed.
    pub facts_changed: u32,
    /// Files whose exported surface changed.
    pub exports_changed: u32,
    /// Files whose imports were (re)resolved.
    pub resolved_files: u32,
    pub imports_resolved: u32,
    pub imports_unresolved: u32,
    /// Files whose outgoing edges were rebuilt.
    pub edge_files: u32,
    /// Edge rebuilds for files that did not change themselves (importers of changed exports).
    pub dependents_recomputed: u32,
    pub reclustered: bool,
    pub root_hash: Option<String>,
    pub ms_walk: u64,
    pub ms_parse: u64,
    pub ms_resolve: u64,
    pub ms_edges: u64,
    pub ms_tree: u64,
    pub ms_total: u64,
}

/// Result of processing one file on a worker.
struct Processed {
    row: FileRow,
    /// New facts to store (None when the content did not change or the file has no extractor).
    facts: Option<Arc<FileFacts>>,
    /// Bytes to put in the derived cache (only when freshly extracted).
    derived: Option<(u64, Vec<u8>)>,
    parsed: bool,
    cache_hit: bool,
    hashed: bool,
    content_same: bool,
}

impl Engine {
    /// Full index run: walk the repo and process every file that changed since the last run.
    pub fn index(self: &Arc<Self>) -> Result<IndexReport> {
        let _g = self.index_lock.lock();
        self.indexing.store(true, Ordering::Relaxed);
        let t0 = Instant::now();
        let walked = iso_walk::walk(&self.root);
        let ms_walk = t0.elapsed().as_millis() as u64;
        self.emit(Event::IndexProgress(IndexProgress {
            phase: IndexPhase::Walk,
            done: walked.len() as u32,
            total: walked.len() as u32,
            current: None,
            elapsed_ms: ms_walk,
        }));
        let r = self.run(walked, true, t0);
        self.indexing.store(false, Ordering::Relaxed);
        let mut r = r?;
        r.ms_walk = ms_walk;
        r.ms_total = t0.elapsed().as_millis() as u64;
        if let Ok(s) = self.summary() {
            self.emit(Event::IndexDone(s));
        }
        Ok(r)
    }

    /// Incremental run for specific paths (from the watcher, or after a save). Paths that no
    /// longer exist are deleted from the index.
    pub fn index_paths(self: &Arc<Self>, paths: &[String]) -> Result<IndexReport> {
        let _g = self.index_lock.lock();
        let t0 = Instant::now();
        let mut walked = Vec::new();
        for p in paths {
            let abs = self.root.join(p);
            if let Ok(md) = std::fs::metadata(&abs)
                && md.is_file()
            {
                let mtime = md
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_nanos() as i64)
                    .unwrap_or(0);
                walked.push(WalkedFile {
                    rel: p.clone(),
                    size: md.len(),
                    mtime,
                });
            }
        }
        let mut r = self.run_subset(walked, paths, t0)?;
        r.ms_total = t0.elapsed().as_millis() as u64;
        Ok(r)
    }

    fn run_subset(
        self: &Arc<Self>,
        walked: Vec<WalkedFile>,
        paths: &[String],
        t0: Instant,
    ) -> Result<IndexReport> {
        // Deletions: requested paths that were not found on disk.
        let present: HashSet<&str> = walked.iter().map(|w| w.rel.as_str()).collect();
        let gone: Vec<String> = paths
            .iter()
            .filter(|p| !present.contains(p.as_str()))
            .cloned()
            .collect();
        self.pipeline(walked, Some(gone), t0)
    }

    fn run(
        self: &Arc<Self>,
        walked: Vec<WalkedFile>,
        _full: bool,
        t0: Instant,
    ) -> Result<IndexReport> {
        self.pipeline(walked, None, t0)
    }

    /// `deleted_hint = None` means `walked` is the whole repo (deletions are computed);
    /// `Some(list)` means an incremental run over `walked` plus explicit deletions.
    fn pipeline(
        self: &Arc<Self>,
        walked: Vec<WalkedFile>,
        deleted_hint: Option<Vec<String>>,
        t0: Instant,
    ) -> Result<IndexReport> {
        let mut rep = IndexReport::default();
        let full = deleted_hint.is_none();
        let existing: HashMap<String, FileRow> = if full {
            self.index
                .read(iso_index::files_all)?
                .into_iter()
                .map(|r| (r.path.clone(), r))
                .collect()
        } else {
            let mut m = HashMap::new();
            self.index.read(|c| {
                for w in &walked {
                    if let Some(r) = iso_index::file_by_path(c, &w.rel)? {
                        m.insert(w.rel.clone(), r);
                    }
                }
                Ok(())
            })?;
            m
        };

        // ---- 1. detect
        let mut todo: Vec<(WalkedFile, Option<FileRow>)> = Vec::new();
        for w in &walked {
            match existing.get(&w.rel) {
                Some(r)
                    if r.size == w.size as i64
                        && r.mtime == w.mtime
                        && r.status != "pending"
                        && r.status != "failed" =>
                {
                    rep.unchanged += 1;
                }
                other => todo.push((w.clone(), other.cloned())),
            }
        }
        let deleted: Vec<FileRow> = match &deleted_hint {
            None => {
                let seen: HashSet<&str> = walked.iter().map(|w| w.rel.as_str()).collect();
                existing
                    .values()
                    .filter(|r| !seen.contains(r.path.as_str()))
                    .cloned()
                    .collect()
            }
            Some(paths) => {
                let mut v = Vec::new();
                self.index.read(|c| {
                    for p in paths {
                        if let Some(r) = iso_index::file_by_path(c, p)? {
                            v.push(r);
                        }
                    }
                    Ok(())
                })?;
                v
            }
        };
        rep.deleted = deleted.len() as u32;
        rep.added = todo.iter().filter(|(_, r)| r.is_none()).count() as u32;

        // Show pending files at once so the tree can paint before parsing finishes.
        if full && !todo.is_empty() {
            self.index.write(|tx| {
                for (w, old) in &todo {
                    if old.is_none() {
                        let lang = Language::detect(&w.rel, None);
                        iso_index::upsert_file(
                            tx,
                            &FileRow {
                                id: FileId::of_path(&w.rel),
                                path: w.rel.clone(),
                                lang,
                                tier: lang.tier(),
                                size: w.size as i64,
                                mtime: 0,
                                content_hash: String::new(),
                                lines: 0,
                                parsed_at: None,
                                facts_hash: None,
                                exports_hash: None,
                                has_errors: false,
                                status: "pending".into(),
                            },
                        )?;
                    }
                }
                Ok(())
            })?;
        }

        // ---- 2. parse through the priority queue
        let t_parse = Instant::now();
        let total = todo.len() as u32;
        let todo_map: HashMap<String, (WalkedFile, Option<FileRow>)> = todo
            .into_iter()
            .map(|(w, r)| (w.rel.clone(), (w, r)))
            .collect();
        for p in todo_map.keys() {
            self.queue.push(JobKind::Parse, p.clone(), Priority::Repo);
        }
        let (tx, rx) = crossbeam_channel::bounded::<Processed>(1024);
        let workers = iso_jobs::default_workers();
        let todo_map = Arc::new(todo_map);
        let mut handles = Vec::new();
        for _ in 0..workers {
            let eng = Arc::clone(self);
            let tx = tx.clone();
            let todo_map = Arc::clone(&todo_map);
            handles.push(std::thread::spawn(move || {
                while let Some((key, _prio)) = eng.queue.try_pop() {
                    if key.kind != JobKind::Parse {
                        eng.queue.finish(true);
                        continue;
                    }
                    let Some((w, old)) = todo_map.get(&key.target) else {
                        eng.queue.finish(true);
                        continue;
                    };
                    let res = eng.process_file(w, old.as_ref());
                    eng.queue.finish(res.is_ok());
                    if let Ok(p) = res
                        && tx.send(p).is_err()
                    {
                        break;
                    }
                }
            }));
        }
        drop(tx);

        let mut facts_changed: HashSet<FileId> = HashSet::new();
        let mut exports_changed: HashSet<FileId> = HashSet::new();
        let mut batch: Vec<Processed> = Vec::with_capacity(256);
        let mut done = 0u32;
        let mut last_emit = Instant::now();
        let flush = |batch: &mut Vec<Processed>,
                     facts_changed: &mut HashSet<FileId>,
                     exports_changed: &mut HashSet<FileId>,
                     rep: &mut IndexReport|
         -> Result<()> {
            if batch.is_empty() {
                return Ok(());
            }
            self.index.write(|tx| {
                for p in batch.iter() {
                    let old = existing.get(&p.row.path);
                    iso_index::upsert_file(tx, &p.row)?;
                    if let Some((key, bytes)) = &p.derived {
                        iso_index::derived_put(
                            tx,
                            DerivedKind::Facts.as_str(),
                            *key,
                            EXTRACTOR_VERSION,
                            bytes,
                        )?;
                    }
                    if let Some(f) = &p.facts {
                        let same_facts = old.and_then(|o| o.facts_hash.as_deref())
                            == p.row.facts_hash.as_deref()
                            && old.is_some_and(|o| o.parsed_at.is_some());
                        if !same_facts {
                            iso_index::replace_facts(tx, p.row.id, &p.row.path, f)?;
                            facts_changed.insert(p.row.id);
                        }
                        if old.and_then(|o| o.exports_hash.as_deref())
                            != p.row.exports_hash.as_deref()
                        {
                            exports_changed.insert(p.row.id);
                        }
                    } else if !p.content_same {
                        // Text tier or failed: make sure no stale facts remain.
                        if old.is_some_and(|o| o.parsed_at.is_some() && o.facts_hash.is_some()) {
                            iso_index::replace_facts(
                                tx,
                                p.row.id,
                                &p.row.path,
                                &FileFacts::default(),
                            )?;
                            facts_changed.insert(p.row.id);
                            exports_changed.insert(p.row.id);
                        }
                    }
                    rep.parsed += p.parsed as u32;
                    rep.facts_cache_hits += p.cache_hit as u32;
                    rep.hashed += p.hashed as u32;
                    if p.content_same {
                        rep.unchanged += 1;
                    }
                }
                Ok(())
            })?;
            batch.clear();
            Ok(())
        };
        for p in rx.iter() {
            done += 1;
            batch.push(p);
            if batch.len() >= 256 || last_emit.elapsed().as_millis() > 150 {
                let current = batch.last().map(|p| p.row.path.clone());
                flush(
                    &mut batch,
                    &mut facts_changed,
                    &mut exports_changed,
                    &mut rep,
                )?;
                if last_emit.elapsed().as_millis() > 100 {
                    last_emit = Instant::now();
                    self.emit(Event::IndexProgress(IndexProgress {
                        phase: IndexPhase::Parse,
                        done,
                        total,
                        current,
                        elapsed_ms: t0.elapsed().as_millis() as u64,
                    }));
                }
            }
        }
        flush(
            &mut batch,
            &mut facts_changed,
            &mut exports_changed,
            &mut rep,
        )?;
        for h in handles {
            let _ = h.join();
        }
        if !deleted.is_empty() {
            self.index.write(|tx| {
                for d in &deleted {
                    iso_index::delete_file(tx, d.id)?;
                }
                Ok(())
            })?;
        }
        rep.ms_parse = t_parse.elapsed().as_millis() as u64;
        rep.facts_changed = facts_changed.len() as u32;
        rep.exports_changed = exports_changed.len() as u32;

        // ---- 4. resolve
        let t_res = Instant::now();
        self.emit(Event::IndexProgress(IndexProgress {
            phase: IndexPhase::Resolve,
            done: 0,
            total: 0,
            current: None,
            elapsed_ms: t0.elapsed().as_millis() as u64,
        }));
        let all_files = self.index.read(iso_index::files_all)?;
        let file_set_changed = rep.added > 0 || rep.deleted > 0;
        let resolvers = self.resolvers(&all_files, file_set_changed);
        let deleted_ids: HashSet<FileId> = deleted.iter().map(|d| d.id).collect();
        let lang_of: HashMap<FileId, (Language, String)> = all_files
            .iter()
            .map(|f| (f.id, (f.lang, f.path.clone())))
            .collect();
        let to_resolve: Vec<iso_index::ImportRow> = self.index.read(|c| {
            if full && (existing.is_empty() || file_set_changed)
                || facts_changed.len() > all_files.len() / 2
            {
                // Cold run or big change: resolve everything that changed, plus anything that
                // was unresolved or pointed at a deleted file.
                Ok(iso_index::imports_all(c)?
                    .into_iter()
                    .filter(|i| {
                        facts_changed.contains(&i.file_id)
                            || i.unresolved
                            || (i.resolved.is_none() && i.external.is_none())
                            || i.resolved.is_some_and(|r| deleted_ids.contains(&r))
                    })
                    .collect())
            } else {
                let mut v = Vec::new();
                for f in &facts_changed {
                    v.extend(iso_index::imports_for_file(c, *f)?);
                }
                if file_set_changed {
                    v.extend(iso_index::imports_all(c)?.into_iter().filter(|i| {
                        !facts_changed.contains(&i.file_id)
                            && (i.unresolved
                                || i.resolved.is_some_and(|r| deleted_ids.contains(&r)))
                    }));
                }
                Ok(v)
            }
        })?;
        let resolved: Vec<(i64, FileId, ResolvedTo)> = to_resolve
            .par_iter()
            .filter_map(|imp| {
                let (lang, path) = lang_of.get(&imp.file_id)?;
                let r = resolvers.resolve(*lang, path, &imp.as_fact());
                let to = match r {
                    Resolution::File(p) => {
                        let id = FileId::of_path(&p);
                        if lang_of.contains_key(&id) {
                            ResolvedTo::File(id)
                        } else {
                            ResolvedTo::Unresolved
                        }
                    }
                    Resolution::External(e) => ResolvedTo::External(e),
                    Resolution::Unresolved => ResolvedTo::Unresolved,
                };
                Some((imp.rowid, imp.file_id, to))
            })
            .collect();
        let mut resolved_files: HashSet<FileId> = HashSet::new();
        let mut resolution_changed: HashSet<FileId> = HashSet::new();
        let before: HashMap<i64, (Option<FileId>, Option<String>, bool)> = to_resolve
            .iter()
            .map(|i| (i.rowid, (i.resolved, i.external.clone(), i.unresolved)))
            .collect();
        self.index.write(|tx| {
            for (rowid, file, to) in &resolved {
                iso_index::set_import_resolution(tx, *rowid, to)?;
                resolved_files.insert(*file);
                let now = match to {
                    ResolvedTo::File(f) => (Some(*f), None, false),
                    ResolvedTo::External(e) => (None, Some(e.clone()), false),
                    ResolvedTo::Unresolved => (None, None, true),
                };
                if before.get(rowid) != Some(&now) {
                    resolution_changed.insert(*file);
                }
            }
            Ok(())
        })?;
        rep.resolved_files = resolved_files.len() as u32;
        rep.imports_resolved = resolved
            .iter()
            .filter(|r| matches!(r.2, ResolvedTo::File(_)))
            .count() as u32;
        rep.imports_unresolved = resolved
            .iter()
            .filter(|r| matches!(r.2, ResolvedTo::Unresolved))
            .count() as u32;
        rep.ms_resolve = t_res.elapsed().as_millis() as u64;

        // ---- 5. edges
        let t_edges = Instant::now();
        let mut edge_targets: HashSet<FileId> = facts_changed
            .iter()
            .chain(resolution_changed.iter())
            .copied()
            .collect();
        edge_targets.retain(|f| !deleted_ids.contains(f));
        let mut dependents: HashSet<FileId> = HashSet::new();
        self.index.read(|c| {
            for f in exports_changed.iter().chain(deleted_ids.iter()) {
                for (imp, _) in iso_index::importers_of(c, *f)? {
                    if !edge_targets.contains(&imp) && !deleted_ids.contains(&imp) {
                        dependents.insert(imp);
                    }
                }
            }
            Ok(())
        })?;
        rep.dependents_recomputed = dependents.len() as u32;
        edge_targets.extend(dependents.iter().copied());
        rep.edge_files = edge_targets.len() as u32;
        if !edge_targets.is_empty() {
            self.emit(Event::IndexProgress(IndexProgress {
                phase: IndexPhase::Graph,
                done: 0,
                total: edge_targets.len() as u32,
                current: None,
                elapsed_ms: t0.elapsed().as_millis() as u64,
            }));
            crate::edges::rebuild(
                self,
                &edge_targets.into_iter().collect::<Vec<_>>(),
                &all_files,
            )?;
        }
        rep.ms_edges = t_edges.elapsed().as_millis() as u64;

        // ---- 6. Merkle tree and modules
        let t_tree = Instant::now();
        let changed_anything = rep.facts_changed > 0
            || rep.deleted > 0
            || rep.added > 0
            || !resolution_changed.is_empty();
        let all_files = if rep.deleted > 0 || rep.added > 0 || rep.hashed > 0 {
            self.index.read(iso_index::files_all)?
        } else {
            all_files
        };
        rep.files = all_files.len() as u32;
        rep.lines = all_files.iter().map(|f| f.lines as u64).sum();
        let stored_root = self.index.read(|c| iso_index::meta_get(c, "root_hash"))?;
        if rep.hashed > 0 || rep.deleted > 0 || stored_root.is_none() {
            let tree = iso_walk::MerkleTree::build(
                all_files
                    .iter()
                    .map(|f| (f.path.as_str(), f.content_hash.as_str())),
            );
            let root_hash = tree.root_hash().map(|s| s.to_string());
            if root_hash != stored_root {
                self.index.write(|tx| {
                    iso_index::replace_tree(
                        tx,
                        tree.nodes.iter().map(|(p, n)| {
                            (p.as_str(), n.is_dir, n.hash.as_str(), n.parent.as_deref())
                        }),
                    )?;
                    if let Some(h) = &root_hash {
                        iso_index::meta_set(tx, "root_hash", h)?;
                    }
                    Ok(())
                })?;
            }
            rep.root_hash = root_hash;
        } else {
            rep.root_hash = stored_root;
        }
        let has_modules = self
            .index
            .read(|c| Ok(!iso_index::modules_all(c)?.is_empty()))?;
        if (changed_anything && (file_set_changed || !resolution_changed.is_empty()))
            || !has_modules
        {
            crate::modules::recluster(self, &all_files)?;
            rep.reclustered = true;
        }
        rep.ms_tree = t_tree.elapsed().as_millis() as u64;
        self.emit(Event::IndexProgress(IndexProgress {
            phase: IndexPhase::Done,
            done: total,
            total,
            current: None,
            elapsed_ms: t0.elapsed().as_millis() as u64,
        }));
        Ok(rep)
    }

    /// Read, hash, classify and (if needed) extract one file.
    fn process_file(&self, w: &WalkedFile, old: Option<&FileRow>) -> Result<Processed> {
        let read = iso_walk::read_file(&self.root, &w.rel)?;
        let id = FileId::of_path(&w.rel);
        let content_same = old.is_some_and(|o| {
            o.content_hash == read.hash && o.parsed_at.is_some() && o.status != "pending"
        });
        if content_same {
            let mut row = old.unwrap().clone();
            row.size = w.size as i64;
            row.mtime = w.mtime;
            return Ok(Processed {
                row,
                facts: None,
                derived: None,
                parsed: false,
                cache_hit: false,
                hashed: true,
                content_same: true,
            });
        }
        let lang = read.lang;
        let mut tier = lang.tier();
        let mut status = "indexed";
        if read.binary {
            tier = Tier::Text;
            status = "skipped";
        } else if w.size > iso_walk::MAX_PARSE_BYTES {
            tier = Tier::Text;
        }
        let mut row = FileRow {
            id,
            path: w.rel.clone(),
            lang,
            tier,
            size: w.size as i64,
            mtime: w.mtime,
            content_hash: read.hash.clone(),
            lines: read.lines,
            parsed_at: Some(iso_index::now_ms()),
            facts_hash: None,
            exports_hash: None,
            has_errors: false,
            status: status.into(),
        };
        if tier == Tier::Text || iso_parse::extractor(lang).is_none() {
            return Ok(Processed {
                row,
                facts: None,
                derived: None,
                parsed: false,
                cache_hit: false,
                hashed: true,
                content_same: false,
            });
        }
        let cached: Option<FileFacts> = self
            .index
            .read(|c| {
                self.derived
                    .get(c, DerivedKind::Facts, &read.hash, EXTRACTOR_VERSION)
            })
            .ok()
            .flatten();
        let (facts, derived, parsed, cache_hit) = match cached {
            Some(f) => (f, None, false, true),
            None => {
                let path = std::path::Path::new(&w.rel);
                let f =
                    std::panic::catch_unwind(|| iso_parse::extract_file(path, lang, &read.text))
                        .unwrap_or_else(|_| FileFacts {
                            has_errors: true,
                            ..Default::default()
                        });
                let d =
                    self.derived
                        .remember(DerivedKind::Facts, &read.hash, EXTRACTOR_VERSION, &f)?;
                (f, Some(d), true, false)
            }
        };
        row.facts_hash = Some(facts_hash(&facts));
        row.exports_hash = Some(exports_hash(&facts));
        row.has_errors = facts.has_errors;
        Ok(Processed {
            row,
            facts: Some(Arc::new(facts)),
            derived,
            parsed,
            cache_hit,
            hashed: true,
            content_same: false,
        })
    }

    /// Resolvers are rebuilt only when the set of files changes.
    fn resolvers(&self, files: &[FileRow], file_set_changed: bool) -> Arc<Resolvers> {
        let key = files.len() as u64
            ^ iso_core::hash64(
                files
                    .iter()
                    .map(|f| f.path.as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
                    .as_bytes(),
            );
        let mut st = self.state.lock();
        if let Some((k, r)) = &st.resolvers
            && (*k == key || !file_set_changed)
        {
            return Arc::clone(r);
        }
        let repo = RepoFiles::new(self.root.clone(), files.iter().map(|f| f.path.clone()));
        let r = Arc::new(Resolvers::new(&repo));
        st.resolvers = Some((key, Arc::clone(&r)));
        r
    }

    /// Facts for a file row, from the derived cache or by re-extracting.
    pub fn facts_for(&self, row: &FileRow) -> Result<FileFacts> {
        if row.tier == Tier::Text || iso_parse::extractor(row.lang).is_none() {
            return Ok(FileFacts::default());
        }
        if let Some(f) = self.index.read(|c| {
            self.derived
                .get(c, DerivedKind::Facts, &row.content_hash, EXTRACTOR_VERSION)
        })? {
            return Ok(f);
        }
        let read = iso_walk::read_file(&self.root, &row.path)?;
        let f = iso_parse::extract_file(std::path::Path::new(&row.path), row.lang, &read.text);
        self.index.write(|tx| {
            self.derived
                .put(tx, DerivedKind::Facts, &read.hash, EXTRACTOR_VERSION, &f)
        })?;
        Ok(f)
    }

    /// Repo summary from the index.
    pub fn summary(&self) -> Result<RepoSummary> {
        let files = self.index.read(iso_index::files_all)?;
        let root_hash = self.index.read(|c| iso_index::meta_get(c, "root_hash"))?;
        let mut tiers = iso_core::ipc::TierCounts {
            full: 0,
            outline: 0,
            text: 0,
        };
        let mut langs: HashMap<Language, (u32, u64)> = HashMap::new();
        let mut indexed = 0;
        let mut lines = 0u64;
        for f in &files {
            match f.tier {
                Tier::Full => tiers.full += 1,
                Tier::Outline => tiers.outline += 1,
                Tier::Text => tiers.text += 1,
            }
            let e = langs.entry(f.lang).or_default();
            e.0 += 1;
            e.1 += f.lines as u64;
            lines += f.lines as u64;
            if f.status != "pending" {
                indexed += 1;
            }
        }
        let mut languages: Vec<iso_core::ipc::LangCount> = langs
            .into_iter()
            .map(|(lang, (files, lines))| iso_core::ipc::LangCount {
                lang,
                label: lang.label().into(),
                files,
                lines,
            })
            .collect();
        languages.sort_by(|a, b| b.lines.cmp(&a.lines).then(b.files.cmp(&a.files)));
        Ok(RepoSummary {
            root: self.root.to_string_lossy().into_owned(),
            name: self.name.clone(),
            files: files.len() as u32,
            lines,
            tiers,
            languages,
            indexed,
            root_hash,
            ai_enabled: self.ai_enabled(),
        })
    }
}
