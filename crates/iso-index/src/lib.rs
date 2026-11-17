//! SQLite index: schema, migrations and typed queries.
//!
//! One database per repo, in WAL mode. [`Index`] holds a writer and a reader connection so the
//! UI can read while background indexing writes. All query helpers take a `&Connection`, so
//! they work on either connection and inside a transaction.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use iso_core::{
    Confidence, EdgeKind, FileFacts, FileId, ImportFact, ImportKind, ImportedName, Language,
    SymbolFact, SymbolId, SymbolKind, Tier,
};
use rusqlite::{Connection, OptionalExtension, params};
use std::sync::Mutex;

pub use rusqlite;

pub const SCHEMA_VERSION: i64 = 1;
const MIGRATION_1: &str = include_str!("../migrations/0001_init.sql");
pub const AI_CACHE_SQL: &str = include_str!("../migrations/ai_cache.sql");

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Writer + reader connections to one repo's index.
pub struct Index {
    path: PathBuf,
    writer: Mutex<Connection>,
    reader: Mutex<Connection>,
}

fn configure(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON; \
         PRAGMA temp_store=MEMORY; PRAGMA cache_size=-32000; PRAGMA busy_timeout=5000;",
    )?;
    Ok(())
}

pub fn migrate(conn: &Connection) -> Result<()> {
    let v: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if v < 1 {
        conn.execute_batch(MIGRATION_1).context("migration 1")?;
        conn.execute_batch(&format!("PRAGMA user_version={SCHEMA_VERSION}"))?;
    }
    Ok(())
}

impl Index {
    pub fn open(path: &Path) -> Result<Index> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let writer = Connection::open(path).with_context(|| format!("open {}", path.display()))?;
        configure(&writer)?;
        migrate(&writer)?;
        let reader = Connection::open(path)?;
        configure(&reader)?;
        Ok(Index {
            path: path.to_path_buf(),
            writer: Mutex::new(writer),
            reader: Mutex::new(reader),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Run `f` inside one write transaction.
    pub fn write<T>(&self, f: impl FnOnce(&rusqlite::Transaction) -> Result<T>) -> Result<T> {
        let mut conn = self.writer.lock().unwrap_or_else(|e| e.into_inner());
        let tx = conn.transaction()?;
        let out = f(&tx)?;
        tx.commit()?;
        Ok(out)
    }

    /// Run `f` on the reader connection.
    pub fn read<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let conn = self.reader.lock().unwrap_or_else(|e| e.into_inner());
        f(&conn)
    }
}

// ---------------------------------------------------------------- files

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileRow {
    pub id: FileId,
    pub path: String,
    pub lang: Language,
    pub tier: Tier,
    pub size: i64,
    pub mtime: i64,
    pub content_hash: String,
    pub lines: u32,
    pub parsed_at: Option<i64>,
    pub facts_hash: Option<String>,
    pub exports_hash: Option<String>,
    pub has_errors: bool,
    pub status: String,
}

const FILE_COLS: &str = "id, path, lang, tier, size, mtime, content_hash, lines, parsed_at, facts_hash, exports_hash, has_errors, status";

fn file_row(r: &rusqlite::Row) -> rusqlite::Result<FileRow> {
    Ok(FileRow {
        id: FileId::from_i64(r.get(0)?),
        path: r.get(1)?,
        lang: Language::parse(&r.get::<_, String>(2)?),
        tier: Tier::parse(&r.get::<_, String>(3)?),
        size: r.get(4)?,
        mtime: r.get(5)?,
        content_hash: r.get(6)?,
        lines: r.get(7)?,
        parsed_at: r.get(8)?,
        facts_hash: r.get(9)?,
        exports_hash: r.get(10)?,
        has_errors: r.get::<_, i64>(11)? != 0,
        status: r.get(12)?,
    })
}

pub fn files_all(conn: &Connection) -> Result<Vec<FileRow>> {
    let mut st = conn.prepare_cached(&format!("SELECT {FILE_COLS} FROM files ORDER BY path"))?;
    let rows = st
        .query_map([], file_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn file_by_path(conn: &Connection, path: &str) -> Result<Option<FileRow>> {
    let mut st = conn.prepare_cached(&format!("SELECT {FILE_COLS} FROM files WHERE path = ?1"))?;
    Ok(st.query_row([path], file_row).optional()?)
}

pub fn file_by_id(conn: &Connection, id: FileId) -> Result<Option<FileRow>> {
    let mut st = conn.prepare_cached(&format!("SELECT {FILE_COLS} FROM files WHERE id = ?1"))?;
    Ok(st.query_row([id.as_i64()], file_row).optional()?)
}

pub fn upsert_file(conn: &Connection, f: &FileRow) -> Result<()> {
    let mut st = conn.prepare_cached(
        "INSERT INTO files (id, path, lang, tier, size, mtime, content_hash, lines, parsed_at, facts_hash, exports_hash, has_errors, status)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
         ON CONFLICT(id) DO UPDATE SET path=excluded.path, lang=excluded.lang, tier=excluded.tier, size=excluded.size,
           mtime=excluded.mtime, content_hash=excluded.content_hash, lines=excluded.lines, parsed_at=excluded.parsed_at,
           facts_hash=excluded.facts_hash, exports_hash=excluded.exports_hash, has_errors=excluded.has_errors, status=excluded.status",
    )?;
    st.execute(params![
        f.id.as_i64(),
        f.path,
        f.lang.as_str(),
        f.tier.as_str(),
        f.size,
        f.mtime,
        f.content_hash,
        f.lines,
        f.parsed_at,
        f.facts_hash,
        f.exports_hash,
        f.has_errors as i64,
        f.status
    ])?;
    Ok(())
}

/// Update only size and mtime (content unchanged after a touch).
pub fn touch_file(conn: &Connection, id: FileId, size: i64, mtime: i64) -> Result<()> {
    conn.prepare_cached("UPDATE files SET size = ?2, mtime = ?3 WHERE id = ?1")?
        .execute(params![id.as_i64(), size, mtime])?;
    Ok(())
}

pub fn delete_file(conn: &Connection, id: FileId) -> Result<()> {
    conn.prepare_cached("DELETE FROM edges WHERE from_file = ?1")?
        .execute([id.as_i64()])?;
    conn.prepare_cached("DELETE FROM file_modules WHERE file_id = ?1")?
        .execute([id.as_i64()])?;
    conn.prepare_cached("DELETE FROM files WHERE id = ?1")?
        .execute([id.as_i64()])?;
    Ok(())
}

// ---------------------------------------------------------------- facts

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SymbolRow {
    pub id: SymbolId,
    pub file_id: FileId,
    pub name: String,
    pub qualified_name: String,
    pub kind: SymbolKind,
    pub start_line: u32,
    pub end_line: u32,
    pub signature: String,
    pub doc: Option<String>,
    pub exported: bool,
    pub is_async: bool,
    pub parent: Option<String>,
    pub body_hash: u64,
}

const SYM_COLS: &str = "id, file_id, name, qualified_name, kind, start_line, end_line, signature, doc, exported, is_async, parent, body_hash";

fn sym_row(r: &rusqlite::Row) -> rusqlite::Result<SymbolRow> {
    Ok(SymbolRow {
        id: SymbolId::from_i64(r.get(0)?),
        file_id: FileId::from_i64(r.get(1)?),
        name: r.get(2)?,
        qualified_name: r.get(3)?,
        kind: SymbolKind::parse(&r.get::<_, String>(4)?),
        start_line: r.get(5)?,
        end_line: r.get(6)?,
        signature: r.get(7)?,
        doc: r.get(8)?,
        exported: r.get::<_, i64>(9)? != 0,
        is_async: r.get::<_, i64>(10)? != 0,
        parent: r.get(11)?,
        body_hash: r.get::<_, i64>(12)? as u64,
    })
}

/// Replace a file's symbols and imports with freshly extracted facts. Import resolution
/// columns start empty; set them with [`set_import_resolutions`].
pub fn replace_facts(
    conn: &Connection,
    file_id: FileId,
    path: &str,
    facts: &FileFacts,
) -> Result<()> {
    conn.prepare_cached("DELETE FROM symbols WHERE file_id = ?1")?
        .execute([file_id.as_i64()])?;
    conn.prepare_cached("DELETE FROM imports WHERE file_id = ?1")?
        .execute([file_id.as_i64()])?;
    let mut ins = conn.prepare_cached(&format!(
        "INSERT OR REPLACE INTO symbols ({SYM_COLS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)"
    ))?;
    for s in &facts.symbols {
        let id = SymbolId::new(path, &s.qualified_name, s.kind);
        ins.execute(params![
            id.as_i64(),
            file_id.as_i64(),
            s.name,
            s.qualified_name,
            s.kind.as_str(),
            s.start_line,
            s.end_line,
            s.signature,
            s.doc,
            s.exported as i64,
            s.is_async as i64,
            s.parent,
            s.body_hash as i64
        ])?;
    }
    let mut imp = conn.prepare_cached(
        "INSERT INTO imports (file_id, specifier, kind, names_json, type_only, line) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?;
    for i in &facts.imports {
        imp.execute(params![
            file_id.as_i64(),
            i.specifier,
            import_kind_str(i.kind),
            serde_json::to_string(&i.names)?,
            i.type_only as i64,
            i.line
        ])?;
    }
    Ok(())
}

pub fn import_kind_str(k: ImportKind) -> &'static str {
    match k {
        ImportKind::Static => "static",
        ImportKind::Dynamic => "dynamic",
        ImportKind::Require => "require",
        ImportKind::Reexport => "reexport",
        ImportKind::Mod => "mod",
        ImportKind::Link => "link",
    }
}

pub fn import_kind_parse(s: &str) -> ImportKind {
    match s {
        "dynamic" => ImportKind::Dynamic,
        "require" => ImportKind::Require,
        "reexport" => ImportKind::Reexport,
        "mod" => ImportKind::Mod,
        "link" => ImportKind::Link,
        _ => ImportKind::Static,
    }
}

pub fn symbols_for_file(conn: &Connection, file_id: FileId) -> Result<Vec<SymbolRow>> {
    let mut st = conn.prepare_cached(&format!(
        "SELECT {SYM_COLS} FROM symbols WHERE file_id = ?1 ORDER BY start_line"
    ))?;
    Ok(st
        .query_map([file_id.as_i64()], sym_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn symbol_by_id(conn: &Connection, id: SymbolId) -> Result<Option<SymbolRow>> {
    let mut st = conn.prepare_cached(&format!("SELECT {SYM_COLS} FROM symbols WHERE id = ?1"))?;
    Ok(st.query_row([id.as_i64()], sym_row).optional()?)
}

/// Exported symbols of a file by name (used to resolve imported names to symbols).
pub fn exported_symbols(conn: &Connection, file_id: FileId) -> Result<Vec<SymbolRow>> {
    let mut st = conn.prepare_cached(&format!(
        "SELECT {SYM_COLS} FROM symbols WHERE file_id = ?1 AND exported = 1 ORDER BY start_line"
    ))?;
    Ok(st
        .query_map([file_id.as_i64()], sym_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Symbols whose name contains `q` (case-insensitive), best matches first.
pub fn search_symbols(
    conn: &Connection,
    q: &str,
    limit: usize,
) -> Result<Vec<(SymbolRow, String)>> {
    let like = format!("%{}%", q.replace('%', "\\%").replace('_', "\\_"));
    let mut st = conn.prepare_cached(
        "SELECT s.id, s.file_id, s.name, s.qualified_name, s.kind, s.start_line, s.end_line, s.signature, s.doc, s.exported,
                s.is_async, s.parent, s.body_hash, f.path
         FROM symbols s JOIN files f ON f.id = s.file_id
         WHERE s.name LIKE ?1 ESCAPE '\\' AND s.kind NOT IN ('selector', 'field', 'variable')
         ORDER BY (s.name = ?2 COLLATE NOCASE) DESC, (s.name LIKE ?3 ESCAPE '\\') DESC, s.exported DESC, length(s.name), s.name
         LIMIT ?4",
    )?;
    let prefix = format!("{}%", q.replace('%', "\\%").replace('_', "\\_"));
    let rows = st
        .query_map(params![like, q, prefix, limit as i64], |r| {
            Ok((sym_row(r)?, r.get::<_, String>(13)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportRow {
    pub rowid: i64,
    pub file_id: FileId,
    pub specifier: String,
    pub kind: ImportKind,
    pub resolved: Option<FileId>,
    pub external: Option<String>,
    pub unresolved: bool,
    pub names: Vec<ImportedName>,
    pub type_only: bool,
    pub line: u32,
}

const IMP_COLS: &str = "rowid, file_id, specifier, kind, resolved_file_id, external, unresolved, names_json, type_only, line";

fn imp_row(r: &rusqlite::Row) -> rusqlite::Result<ImportRow> {
    let names: String = r.get(7)?;
    Ok(ImportRow {
        rowid: r.get(0)?,
        file_id: FileId::from_i64(r.get(1)?),
        specifier: r.get(2)?,
        kind: import_kind_parse(&r.get::<_, String>(3)?),
        resolved: r.get::<_, Option<i64>>(4)?.map(FileId::from_i64),
        external: r.get(5)?,
        unresolved: r.get::<_, i64>(6)? != 0,
        names: serde_json::from_str(&names).unwrap_or_default(),
        type_only: r.get::<_, i64>(8)? != 0,
        line: r.get(9)?,
    })
}

impl ImportRow {
    pub fn as_fact(&self) -> ImportFact {
        ImportFact {
            specifier: self.specifier.clone(),
            kind: self.kind,
            names: self.names.clone(),
            type_only: self.type_only,
            line: self.line,
        }
    }
}

pub fn imports_for_file(conn: &Connection, file_id: FileId) -> Result<Vec<ImportRow>> {
    let mut st = conn.prepare_cached(&format!(
        "SELECT {IMP_COLS} FROM imports WHERE file_id = ?1 ORDER BY line"
    ))?;
    Ok(st
        .query_map([file_id.as_i64()], imp_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn imports_all(conn: &Connection) -> Result<Vec<ImportRow>> {
    let mut st = conn.prepare_cached(&format!("SELECT {IMP_COLS} FROM imports"))?;
    Ok(st
        .query_map([], imp_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Resolution outcome for one import row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResolvedTo {
    File(FileId),
    External(String),
    Unresolved,
}

pub fn set_import_resolution(conn: &Connection, rowid: i64, to: &ResolvedTo) -> Result<()> {
    let (file, ext, unres) = match to {
        ResolvedTo::File(f) => (Some(f.as_i64()), None, 0),
        ResolvedTo::External(e) => (None, Some(e.as_str()), 0),
        ResolvedTo::Unresolved => (None, None, 1),
    };
    conn.prepare_cached(
        "UPDATE imports SET resolved_file_id = ?2, external = ?3, unresolved = ?4 WHERE rowid = ?1",
    )?
    .execute(params![rowid, file, ext, unres])?;
    Ok(())
}

/// Files that import `file_id`, with the import line.
pub fn importers_of(conn: &Connection, file_id: FileId) -> Result<Vec<(FileId, u32)>> {
    let mut st = conn
        .prepare_cached("SELECT file_id, MIN(line) FROM imports WHERE resolved_file_id = ?1 GROUP BY file_id ORDER BY file_id")?;
    Ok(st
        .query_map([file_id.as_i64()], |r| {
            Ok((FileId::from_i64(r.get(0)?), r.get(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

/// File-level import graph: (importer, imported) pairs with a count.
pub fn file_import_edges(conn: &Connection) -> Result<Vec<(FileId, FileId)>> {
    let mut st = conn.prepare_cached(
        "SELECT DISTINCT file_id, resolved_file_id FROM imports WHERE resolved_file_id IS NOT NULL AND resolved_file_id != file_id",
    )?;
    Ok(st
        .query_map([], |r| {
            Ok((FileId::from_i64(r.get(0)?), FileId::from_i64(r.get(1)?)))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

// ---------------------------------------------------------------- edges

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EdgeRow {
    pub from_symbol: SymbolId,
    pub to_symbol: Option<SymbolId>,
    pub to_external: Option<String>,
    pub kind: EdgeKind,
    pub confidence: Confidence,
    pub line: u32,
    pub from_file: FileId,
}

fn edge_row(r: &rusqlite::Row) -> rusqlite::Result<EdgeRow> {
    Ok(EdgeRow {
        from_symbol: SymbolId::from_i64(r.get(0)?),
        to_symbol: r.get::<_, Option<i64>>(1)?.map(SymbolId::from_i64),
        to_external: r.get(2)?,
        kind: EdgeKind::parse(&r.get::<_, String>(3)?),
        confidence: Confidence::parse(&r.get::<_, String>(4)?),
        line: r.get(5)?,
        from_file: FileId::from_i64(r.get(6)?),
    })
}

const EDGE_COLS: &str = "from_symbol, to_symbol, to_external, kind, confidence, line, from_file";

pub fn replace_edges_from_file(
    conn: &Connection,
    file_id: FileId,
    edges: &[EdgeRow],
) -> Result<()> {
    conn.prepare_cached("DELETE FROM edges WHERE from_file = ?1")?
        .execute([file_id.as_i64()])?;
    let mut ins = conn.prepare_cached(&format!(
        "INSERT INTO edges ({EDGE_COLS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"
    ))?;
    for e in edges {
        ins.execute(params![
            e.from_symbol.as_i64(),
            e.to_symbol.map(|s| s.as_i64()),
            e.to_external,
            e.kind.as_str(),
            e.confidence.as_str(),
            e.line,
            e.from_file.as_i64()
        ])?;
    }
    Ok(())
}

pub fn edges_from(conn: &Connection, sym: SymbolId) -> Result<Vec<EdgeRow>> {
    let mut st = conn.prepare_cached(&format!(
        "SELECT {EDGE_COLS} FROM edges WHERE from_symbol = ?1"
    ))?;
    Ok(st
        .query_map([sym.as_i64()], edge_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn edges_to(conn: &Connection, sym: SymbolId) -> Result<Vec<EdgeRow>> {
    let mut st = conn.prepare_cached(&format!(
        "SELECT {EDGE_COLS} FROM edges WHERE to_symbol = ?1"
    ))?;
    Ok(st
        .query_map([sym.as_i64()], edge_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Upgrade (or insert) the confidence of an edge. Used by precise indexers.
pub fn set_edge_confidence(
    conn: &Connection,
    from: SymbolId,
    to: SymbolId,
    c: Confidence,
) -> Result<usize> {
    Ok(conn
        .prepare_cached(
            "UPDATE edges SET confidence = ?3 WHERE from_symbol = ?1 AND to_symbol = ?2",
        )?
        .execute(params![from.as_i64(), to.as_i64(), c.as_str()])?)
}

/// Number of callers outside the symbol's own file.
pub fn external_caller_count(conn: &Connection, sym: SymbolId, own_file: FileId) -> Result<u32> {
    let mut st = conn.prepare_cached(
        "SELECT COUNT(DISTINCT from_symbol) FROM edges WHERE to_symbol = ?1 AND from_file != ?2 AND kind = 'call'",
    )?;
    Ok(st.query_row(params![sym.as_i64(), own_file.as_i64()], |r| r.get(0))?)
}

pub fn edge_count(conn: &Connection) -> Result<u64> {
    Ok(conn.query_row("SELECT COUNT(*) FROM edges", [], |r| r.get::<_, i64>(0))? as u64)
}

// ---------------------------------------------------------------- modules

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModuleRow {
    pub id: String,
    pub name: String,
    pub role: Option<String>,
    pub color: String,
    pub source: String,
    pub dir: String,
}

pub fn modules_all(conn: &Connection) -> Result<Vec<ModuleRow>> {
    let mut st =
        conn.prepare_cached("SELECT id, name, role, color, source, dir FROM modules ORDER BY dir")?;
    Ok(st
        .query_map([], |r| {
            Ok(ModuleRow {
                id: r.get(0)?,
                name: r.get(1)?,
                role: r.get(2)?,
                color: r.get(3)?,
                source: r.get(4)?,
                dir: r.get(5)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Replace module membership. Modules whose source is `user` or `ai` keep their name, role
/// and color when the same id comes back.
pub fn replace_modules(
    conn: &Connection,
    modules: &[ModuleRow],
    members: &[(FileId, String)],
) -> Result<()> {
    let existing: std::collections::HashMap<String, ModuleRow> = modules_all(conn)?
        .into_iter()
        .map(|m| (m.id.clone(), m))
        .collect();
    conn.execute("DELETE FROM modules", [])?;
    conn.execute("DELETE FROM file_modules", [])?;
    let mut ins = conn.prepare_cached(
        "INSERT INTO modules (id, name, role, color, source, dir) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?;
    for m in modules {
        let keep = existing.get(&m.id).filter(|e| e.source != "cluster");
        let (name, role, color, source) = match keep {
            Some(e) => (
                e.name.clone(),
                e.role.clone(),
                e.color.clone(),
                e.source.clone(),
            ),
            None => (
                m.name.clone(),
                m.role.clone(),
                m.color.clone(),
                m.source.clone(),
            ),
        };
        ins.execute(params![m.id, name, role, color, source, m.dir])?;
    }
    let mut fm = conn.prepare_cached(
        "INSERT OR REPLACE INTO file_modules (file_id, module_id) VALUES (?1, ?2)",
    )?;
    for (f, m) in members {
        fm.execute(params![f.as_i64(), m])?;
    }
    Ok(())
}

pub fn update_module(
    conn: &Connection,
    id: &str,
    name: Option<&str>,
    role: Option<&str>,
    color: Option<&str>,
    source: &str,
) -> Result<()> {
    if let Some(n) = name {
        conn.execute(
            "UPDATE modules SET name = ?2, source = ?3 WHERE id = ?1",
            params![id, n, source],
        )?;
    }
    if let Some(r) = role {
        conn.execute("UPDATE modules SET role = ?2 WHERE id = ?1", params![id, r])?;
    }
    if let Some(c) = color {
        conn.execute(
            "UPDATE modules SET color = ?2, source = ?3 WHERE id = ?1",
            params![id, c, source],
        )?;
    }
    Ok(())
}

pub fn file_modules(conn: &Connection) -> Result<Vec<(FileId, String)>> {
    let mut st = conn.prepare_cached("SELECT file_id, module_id FROM file_modules")?;
    Ok(st
        .query_map([], |r| Ok((FileId::from_i64(r.get(0)?), r.get(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

// ---------------------------------------------------------------- derived cache, meta, tree

pub fn derived_get(conn: &Connection, kind: &str, key: u64) -> Result<Option<Vec<u8>>> {
    let mut st =
        conn.prepare_cached("SELECT value FROM derived WHERE kind = ?1 AND key_hash = ?2")?;
    Ok(st
        .query_row(params![kind, key as i64], |r| r.get(0))
        .optional()?)
}

pub fn derived_put(
    conn: &Connection,
    kind: &str,
    key: u64,
    version: u32,
    value: &[u8],
) -> Result<()> {
    conn.prepare_cached(
        "INSERT OR REPLACE INTO derived (kind, key_hash, version, value, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
    )?
    .execute(params![kind, key as i64, version, value, now_ms()])?;
    Ok(())
}

pub fn meta_get(conn: &Connection, key: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
        .optional()?)
}

pub fn meta_set(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
        params![key, value],
    )?;
    Ok(())
}

/// Replace the stored Merkle tree.
pub fn replace_tree<'a>(
    conn: &Connection,
    nodes: impl IntoIterator<Item = (&'a str, bool, &'a str, Option<&'a str>)>,
) -> Result<()> {
    conn.execute("DELETE FROM tree_nodes", [])?;
    let mut ins = conn.prepare_cached(
        "INSERT INTO tree_nodes (path, kind, hash, parent_path) VALUES (?1, ?2, ?3, ?4)",
    )?;
    for (path, is_dir, hash, parent) in nodes {
        ins.execute(params![
            path,
            if is_dir { "dir" } else { "file" },
            hash,
            parent
        ])?;
    }
    Ok(())
}

/// Load the stored Merkle tree as (path, is_dir, hash, parent).
/// (path, is_dir, hash, parent)
pub type TreeNodeRow = (String, bool, String, Option<String>);

pub fn load_tree(conn: &Connection) -> Result<Vec<TreeNodeRow>> {
    let mut st = conn.prepare_cached("SELECT path, kind, hash, parent_path FROM tree_nodes")?;
    Ok(st
        .query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get::<_, String>(1)? == "dir",
                r.get(2)?,
                r.get(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Counts used by eval: (files, symbols, imports, unresolved imports, edges).
pub fn counts(conn: &Connection) -> Result<(u64, u64, u64, u64, u64)> {
    let q =
        |sql: &str| -> Result<u64> { Ok(conn.query_row(sql, [], |r| r.get::<_, i64>(0))? as u64) };
    Ok((
        q("SELECT COUNT(*) FROM files")?,
        q("SELECT COUNT(*) FROM symbols")?,
        q("SELECT COUNT(*) FROM imports")?,
        q("SELECT COUNT(*) FROM imports WHERE unresolved = 1")?,
        q("SELECT COUNT(*) FROM edges")?,
    ))
}

/// Symbols the extractor returned, converted back from rows (for skeletons and AI inputs).
pub fn row_to_fact(r: &SymbolRow) -> SymbolFact {
    SymbolFact {
        name: r.name.clone(),
        qualified_name: r.qualified_name.clone(),
        kind: r.kind,
        start_line: r.start_line,
        end_line: r.end_line,
        signature: r.signature.clone(),
        doc: r.doc.clone(),
        exported: r.exported,
        is_async: r.is_async,
        params: Vec::new(),
        parent: r.parent.clone(),
        body_hash: r.body_hash,
    }
}

/// Paths for a set of file ids.
pub fn file_paths(
    conn: &Connection,
    ids: impl IntoIterator<Item = FileId>,
) -> Result<std::collections::HashMap<FileId, String>> {
    let mut st = conn.prepare_cached("SELECT path FROM files WHERE id = ?1")?;
    let mut out = std::collections::HashMap::new();
    for id in ids {
        if out.contains_key(&id) {
            continue;
        }
        if let Some(p) = st
            .query_row([id.as_i64()], |r| r.get::<_, String>(0))
            .optional()?
        {
            out.insert(id, p);
        }
    }
    Ok(out)
}

/// Paths containing `q` (case-insensitive), file-name matches first.
pub fn search_paths(conn: &Connection, q: &str, limit: usize) -> Result<Vec<String>> {
    let esc = q
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    let mut st = conn.prepare_cached(
        "SELECT path FROM files WHERE path LIKE ?1 ESCAPE '\\'
         ORDER BY (path LIKE ?2 ESCAPE '\\') DESC, length(path) LIMIT ?3",
    )?;
    let rows = st
        .query_map(
            params![format!("%{esc}%"), format!("%/{esc}%"), (limit * 4) as i64],
            |r| r.get::<_, String>(0),
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use iso_core::{ImportedName, SymbolFact};

    fn facts() -> FileFacts {
        FileFacts {
            symbols: vec![SymbolFact {
                name: "run".into(),
                qualified_name: "run".into(),
                kind: SymbolKind::Function,
                start_line: 3,
                end_line: 9,
                signature: "function run()".into(),
                doc: Some("Runs.".into()),
                exported: true,
                is_async: false,
                params: vec![],
                parent: None,
                body_hash: u64::MAX - 3,
            }],
            imports: vec![ImportFact {
                specifier: "./b".into(),
                kind: ImportKind::Static,
                names: vec![ImportedName {
                    name: "b".into(),
                    alias: None,
                    type_only: false,
                }],
                type_only: false,
                line: 1,
            }],
            ..Default::default()
        }
    }

    #[test]
    fn roundtrip_files_symbols_imports() {
        let t = tempfile::tempdir().unwrap();
        let idx = Index::open(&t.path().join("i.sqlite")).unwrap();
        let a = FileRow {
            id: FileId::of_path("a.ts"),
            path: "a.ts".into(),
            lang: Language::TypeScript,
            tier: Tier::Full,
            size: 10,
            mtime: 1,
            content_hash: "h".into(),
            lines: 9,
            parsed_at: Some(1),
            facts_hash: None,
            exports_hash: None,
            has_errors: false,
            status: "indexed".into(),
        };
        let b = FileRow {
            id: FileId::of_path("b.ts"),
            path: "b.ts".into(),
            ..a.clone()
        };
        idx.write(|tx| {
            upsert_file(tx, &a)?;
            upsert_file(tx, &b)?;
            replace_facts(tx, a.id, "a.ts", &facts())?;
            let imps = imports_for_file(tx, a.id)?;
            set_import_resolution(tx, imps[0].rowid, &ResolvedTo::File(b.id))?;
            Ok(())
        })
        .unwrap();
        idx.read(|c| {
            assert_eq!(files_all(c)?.len(), 2);
            let syms = symbols_for_file(c, a.id)?;
            assert_eq!(syms[0].body_hash, u64::MAX - 3);
            assert_eq!(
                syms[0].id,
                SymbolId::new("a.ts", "run", SymbolKind::Function)
            );
            assert_eq!(importers_of(c, b.id)?, vec![(a.id, 1)]);
            assert_eq!(search_symbols(c, "ru", 5)?.len(), 1);
            Ok(())
        })
        .unwrap();
        idx.write(|tx| delete_file(tx, a.id)).unwrap();
        idx.read(|c| {
            assert!(symbols_for_file(c, a.id)?.is_empty());
            assert!(imports_for_file(c, a.id)?.is_empty());
            Ok(())
        })
        .unwrap();
    }
}
