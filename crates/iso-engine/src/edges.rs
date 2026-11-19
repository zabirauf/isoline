//! Cross-file edges from call and reference facts plus resolved imports.
//!
//! Same-file calls are `exact`. Calls matched through an import are `heuristic` until a precise
//! indexer upgrades them. Re-exports (barrel files) are followed up to four hops.

use std::collections::{HashMap, HashSet};

use anyhow::Result;
use iso_core::{Confidence, EdgeKind, FileFacts, FileId, ImportKind, SymbolId, SymbolKind};
use iso_index::{EdgeRow, FileRow, ImportRow};
use rayon::prelude::*;

use crate::Engine;

/// Exported symbols of one file, by bare name and by qualified name.
#[derive(Default)]
pub(crate) struct Exports {
    by_name: HashMap<String, (SymbolId, SymbolKind)>,
    by_qualified: HashMap<String, SymbolId>,
    /// Re-exports: (exported name or "*", imported name, target file).
    reexports: Vec<(String, String, FileId)>,
}

pub(crate) struct ExportTable<'a> {
    /// Engine for lazy per-file loads (incremental runs); `None` when everything is preloaded.
    eng: Option<&'a Engine>,
    files: parking_lot::RwLock<HashMap<FileId, std::sync::Arc<Exports>>>,
}

type ExportRow = (SymbolId, FileId, String, String, SymbolKind, Option<String>);

fn export_row(r: &iso_index::rusqlite::Row) -> iso_index::rusqlite::Result<ExportRow> {
    Ok((
        SymbolId::from_i64(r.get(0)?),
        FileId::from_i64(r.get(1)?),
        r.get::<_, String>(2)?,
        r.get::<_, String>(3)?,
        SymbolKind::parse(&r.get::<_, String>(4)?),
        r.get::<_, Option<String>>(5)?,
    ))
}

fn add_symbol(e: &mut Exports, row: ExportRow) {
    let (id, _file, name, qn, kind, parent) = row;
    if parent.is_none() {
        e.by_name.entry(name).or_insert((id, kind));
    }
    e.by_qualified.insert(qn, id);
}

fn add_reexports(e: &mut Exports, names: &str, target: FileId) {
    let names: Vec<iso_core::ImportedName> = serde_json::from_str(names).unwrap_or_default();
    if names.is_empty() {
        e.reexports.push(("*".into(), "*".into(), target));
    }
    for n in names {
        e.reexports
            .push((n.local().to_string(), n.name.clone(), target));
    }
}

const EXPORT_SQL: &str = "SELECT id, file_id, name, qualified_name, kind, parent FROM symbols WHERE (exported = 1 OR parent IS NOT NULL)";
const REEXPORT_SQL: &str = "SELECT file_id, names_json, resolved_file_id FROM imports WHERE kind = 'reexport' AND resolved_file_id IS NOT NULL";

impl<'a> ExportTable<'a> {
    /// Preload every file's exports in two queries (full index runs).
    pub(crate) fn preload(eng: &Engine) -> Result<ExportTable<'static>> {
        let mut map: HashMap<FileId, Exports> = HashMap::new();
        eng.index.read(|c| {
            let mut st = c.prepare_cached(EXPORT_SQL)?;
            for row in st.query_map([], export_row)? {
                let row = row?;
                add_symbol(map.entry(row.1).or_default(), row);
            }
            let mut st = c.prepare_cached(REEXPORT_SQL)?;
            let rows = st.query_map([], |r| {
                Ok((
                    FileId::from_i64(r.get(0)?),
                    r.get::<_, String>(1)?,
                    FileId::from_i64(r.get(2)?),
                ))
            })?;
            for row in rows {
                let (file, names, target) = row?;
                add_reexports(map.entry(file).or_default(), &names, target);
            }
            Ok(())
        })?;
        Ok(ExportTable {
            eng: None,
            files: parking_lot::RwLock::new(
                map.into_iter()
                    .map(|(k, v)| (k, std::sync::Arc::new(v)))
                    .collect(),
            ),
        })
    }

    /// Load exports on demand, one file at a time (incremental runs and file views).
    pub(crate) fn lazy(eng: &'a Engine) -> ExportTable<'a> {
        ExportTable {
            eng: Some(eng),
            files: parking_lot::RwLock::new(HashMap::new()),
        }
    }

    fn get(&self, file: FileId) -> Option<std::sync::Arc<Exports>> {
        if let Some(e) = self.files.read().get(&file) {
            return Some(std::sync::Arc::clone(e));
        }
        let eng = self.eng?;
        let mut e = Exports::default();
        eng.index
            .read(|c| {
                let mut st = c.prepare_cached(&format!("{EXPORT_SQL} AND file_id = ?1"))?;
                for row in st.query_map([file.as_i64()], export_row)? {
                    add_symbol(&mut e, row?);
                }
                let mut st = c.prepare_cached(&format!("{REEXPORT_SQL} AND file_id = ?1"))?;
                let rows = st.query_map([file.as_i64()], |r| {
                    Ok((r.get::<_, String>(1)?, FileId::from_i64(r.get(2)?)))
                })?;
                for row in rows {
                    let (names, target) = row?;
                    add_reexports(&mut e, &names, target);
                }
                Ok(())
            })
            .ok()?;
        let e = std::sync::Arc::new(e);
        self.files.write().insert(file, std::sync::Arc::clone(&e));
        Some(e)
    }

    /// Find an exported symbol `name` in `file`, following re-exports.
    pub(crate) fn find(
        &self,
        file: FileId,
        name: &str,
        depth: u32,
    ) -> Option<(SymbolId, SymbolKind)> {
        let e = self.get(file)?;
        if let Some(s) = e.by_name.get(name) {
            return Some(*s);
        }
        if depth >= 4 {
            return None;
        }
        for (exported, imported, target) in &e.reexports {
            if exported == name {
                let n = if imported == "*" || imported == "default" {
                    name
                } else {
                    imported.as_str()
                };
                if let Some(s) = self.find(*target, n, depth + 1) {
                    return Some(s);
                }
            }
        }
        for (exported, _, target) in &e.reexports {
            if exported == "*"
                && let Some(s) = self.find(*target, name, depth + 1)
            {
                return Some(s);
            }
        }
        None
    }

    pub(crate) fn find_qualified(&self, file: FileId, qn: &str) -> Option<SymbolId> {
        self.get(file)?.by_qualified.get(qn).copied()
    }

    pub(crate) fn has_type(&self, file: FileId, name: &str) -> bool {
        self.find(file, name, 0)
            .is_some_and(|(_, k)| k.is_type_like())
    }
}

/// Where a file-level local name points.
#[derive(Clone, Debug)]
pub(crate) enum Binding {
    /// A name imported from a repo file: (file, imported name). `*` for namespaces.
    File(FileId, String),
    /// A name imported from outside the repo: (module, imported name).
    External(String, String),
}

/// Local name → binding for every import of a file.
pub(crate) fn bindings(imports: &[ImportRow]) -> HashMap<String, Binding> {
    let mut m = HashMap::new();
    for i in imports {
        if i.kind == ImportKind::Reexport {
            continue;
        }
        let bind = |name: &str| -> Option<Binding> {
            if let Some(f) = i.resolved {
                Some(Binding::File(f, name.to_string()))
            } else {
                i.external
                    .as_ref()
                    .map(|e| Binding::External(e.clone(), name.to_string()))
            }
        };
        if i.names.is_empty() {
            // `import a.b` / `use a::b` / `mod b;`: bind the last path segment as a namespace.
            let last = i
                .specifier
                .rsplit(['.', ':', '/'])
                .find(|s| !s.is_empty())
                .unwrap_or(&i.specifier);
            if let Some(b) = bind("*") {
                m.entry(last.to_string()).or_insert(b);
            }
            continue;
        }
        for n in &i.names {
            if let Some(b) = bind(&n.name) {
                m.insert(n.local().to_string(), b);
            }
        }
    }
    m
}

/// Build and store outgoing edges for `targets`. `preload` loads every export up front (full
/// runs); otherwise exports load lazily for just the files the targets import.
pub(crate) fn rebuild(eng: &Engine, targets: &[FileId], preload: bool) -> Result<()> {
    let rows: Vec<FileRow> = eng.index.read(|c| {
        let mut v = Vec::with_capacity(targets.len());
        for t in targets {
            if let Some(r) = iso_index::file_by_id(c, *t)? {
                v.push(r);
            }
        }
        Ok(v)
    })?;
    let table = if preload {
        ExportTable::preload(eng)?
    } else {
        ExportTable::lazy(eng)
    };
    let results: Vec<(FileId, Vec<EdgeRow>)> = crate::indexer::index_pool().install(|| {
        rows.par_iter()
            .filter_map(|row| {
                let facts = eng.facts_for(row).ok()?;
                let imports = eng
                    .index
                    .read(|c| iso_index::imports_for_file(c, row.id))
                    .ok()?;
                Some((row.id, build_edges(row, &facts, &imports, &table)))
            })
            .collect()
    });
    for chunk in results.chunks(500) {
        eng.index.write(|tx| {
            for (fid, edges) in chunk {
                iso_index::replace_edges_from_file(tx, *fid, edges)?;
            }
            Ok(())
        })?;
    }
    Ok(())
}

pub(crate) fn build_edges(
    row: &FileRow,
    facts: &FileFacts,
    imports: &[ImportRow],
    table: &ExportTable,
) -> Vec<EdgeRow> {
    let path = row.path.as_str();
    let fid = row.id;
    let binds = bindings(imports);
    // Symbols of this file.
    let mut sym_by_qn: HashMap<&str, (SymbolId, SymbolKind, Option<&str>)> = HashMap::new();
    let mut top_callables: HashMap<&str, SymbolId> = HashMap::new();
    let mut types: HashSet<&str> = HashSet::new();
    for s in &facts.symbols {
        let id = SymbolId::new(path, &s.qualified_name, s.kind);
        sym_by_qn
            .entry(s.qualified_name.as_str())
            .or_insert((id, s.kind, s.parent.as_deref()));
        if s.parent.is_none() && s.kind.is_callable() {
            top_callables.entry(s.name.as_str()).or_insert(id);
        }
        if s.kind.is_type_like() {
            types.insert(s.name.as_str());
        }
    }
    let mut out: Vec<EdgeRow> = Vec::new();
    let mut seen: HashSet<(SymbolId, Option<SymbolId>, Option<String>, EdgeKind)> = HashSet::new();
    let mut push = |from: SymbolId,
                    to: Option<SymbolId>,
                    ext: Option<String>,
                    kind: EdgeKind,
                    conf: Confidence,
                    line: u32,
                    out: &mut Vec<EdgeRow>| {
        if to == Some(from) {
            return;
        }
        if seen.insert((from, to, ext.clone(), kind)) {
            out.push(EdgeRow {
                from_symbol: from,
                to_symbol: to,
                to_external: ext,
                kind,
                confidence: conf,
                line,
                from_file: fid,
            });
        }
    };
    let caller_of = |q: &Option<String>| -> Option<(SymbolId, Option<&str>)> {
        let q = q.as_deref()?;
        // Nested helpers are attributed to the nearest enclosing symbol the index knows.
        let mut cur = q;
        loop {
            if let Some((id, _, parent)) = sym_by_qn.get(cur) {
                return Some((*id, *parent));
            }
            cur = cur.rsplit_once('.')?.0;
        }
    };
    for c in &facts.calls {
        let Some((from, parent)) = caller_of(&c.caller) else {
            continue;
        };
        match c.receiver.as_deref() {
            None => {
                if let Some(&to) = top_callables.get(c.name.as_str()) {
                    push(
                        from,
                        Some(to),
                        None,
                        EdgeKind::Call,
                        Confidence::Exact,
                        c.line,
                        &mut out,
                    );
                } else if let Some(b) = binds.get(c.name.as_str()) {
                    match b {
                        Binding::File(t, name) => {
                            let n = if name == "default" || name == "*" {
                                c.name.as_str()
                            } else {
                                name.as_str()
                            };
                            if let Some((to, _)) = table.find(*t, n, 0) {
                                push(
                                    from,
                                    Some(to),
                                    None,
                                    EdgeKind::Call,
                                    Confidence::Heuristic,
                                    c.line,
                                    &mut out,
                                );
                            }
                        }
                        Binding::External(m, name) => {
                            push(
                                from,
                                None,
                                Some(format!("{m}\u{0}{name}")),
                                EdgeKind::Call,
                                Confidence::Heuristic,
                                c.line,
                                &mut out,
                            );
                        }
                    }
                }
            }
            Some(recv) => {
                let recv0 = recv.split(['.', ':']).next().unwrap_or(recv);
                if matches!(recv0, "this" | "self" | "Self" | "cls") {
                    if let Some(p) = parent
                        && let Some((to, _, _)) = sym_by_qn.get(format!("{p}.{}", c.name).as_str())
                    {
                        push(
                            from,
                            Some(*to),
                            None,
                            EdgeKind::Call,
                            Confidence::Exact,
                            c.line,
                            &mut out,
                        );
                    }
                    continue;
                }
                if types.contains(recv) {
                    if let Some((to, _, _)) = sym_by_qn.get(format!("{recv}.{}", c.name).as_str()) {
                        push(
                            from,
                            Some(*to),
                            None,
                            EdgeKind::Call,
                            Confidence::Exact,
                            c.line,
                            &mut out,
                        );
                    }
                    continue;
                }
                if let Some(b) = binds.get(recv0) {
                    match b {
                        Binding::File(t, name) => {
                            let to = if name != "*" && name != "default" && table.has_type(*t, name)
                            {
                                table.find_qualified(*t, &format!("{name}.{}", c.name))
                            } else {
                                // Namespace or module binding: the member is a top-level export.
                                table.find(*t, &c.name, 0).map(|x| x.0)
                            };
                            if let Some(to) = to {
                                push(
                                    from,
                                    Some(to),
                                    None,
                                    EdgeKind::Call,
                                    Confidence::Heuristic,
                                    c.line,
                                    &mut out,
                                );
                            }
                        }
                        Binding::External(m, name) => {
                            let n = if name == "*" {
                                c.name.clone()
                            } else {
                                format!("{name}.{}", c.name)
                            };
                            push(
                                from,
                                None,
                                Some(format!("{m}\u{0}{n}")),
                                EdgeKind::Call,
                                Confidence::Heuristic,
                                c.line,
                                &mut out,
                            );
                        }
                    }
                }
            }
        }
    }
    for r in &facts.refs {
        if !r.type_position {
            continue;
        }
        let Some((from, _)) = caller_of(&r.from) else {
            continue;
        };
        if let Some(Binding::File(t, name)) = binds.get(r.name.as_str()) {
            let n = if name == "*" || name == "default" {
                r.name.as_str()
            } else {
                name.as_str()
            };
            if let Some((to, _)) = table.find(*t, n, 0) {
                push(
                    from,
                    Some(to),
                    None,
                    EdgeKind::TypeUse,
                    Confidence::Heuristic,
                    r.line,
                    &mut out,
                );
            }
        }
    }
    out
}
