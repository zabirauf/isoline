-- Isoline index schema v1. One database per repo, WAL mode.
CREATE TABLE IF NOT EXISTS files (
  id INTEGER PRIMARY KEY,            -- FileId (hash of repo-relative path)
  path TEXT NOT NULL UNIQUE,
  lang TEXT NOT NULL,
  tier TEXT NOT NULL,
  size INTEGER NOT NULL,
  mtime INTEGER NOT NULL,            -- nanoseconds since epoch
  content_hash TEXT NOT NULL,
  lines INTEGER NOT NULL DEFAULT 0,
  parsed_at INTEGER,                 -- NULL until facts are stored
  facts_hash TEXT,                   -- hash of the extracted facts (early cutoff)
  exports_hash TEXT,                 -- hash of exported symbol signatures (early cutoff)
  has_errors INTEGER NOT NULL DEFAULT 0,
  status TEXT NOT NULL DEFAULT 'pending'
);
CREATE INDEX IF NOT EXISTS files_hash ON files(content_hash);

-- Merkle tree: a file's hash is its content hash; a directory's is the hash of its children.
CREATE TABLE IF NOT EXISTS tree_nodes (
  path TEXT PRIMARY KEY,             -- "" for the root
  kind TEXT NOT NULL,                -- file | dir
  hash TEXT NOT NULL,
  parent_path TEXT
);
CREATE INDEX IF NOT EXISTS tree_nodes_parent ON tree_nodes(parent_path);

CREATE TABLE IF NOT EXISTS symbols (
  id INTEGER PRIMARY KEY,            -- SymbolId
  file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  qualified_name TEXT NOT NULL,
  kind TEXT NOT NULL,
  start_line INTEGER NOT NULL,
  end_line INTEGER NOT NULL,
  signature TEXT NOT NULL,
  doc TEXT,
  exported INTEGER NOT NULL,
  is_async INTEGER NOT NULL DEFAULT 0,
  parent TEXT,
  body_hash INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS symbols_file ON symbols(file_id);
CREATE INDEX IF NOT EXISTS symbols_name ON symbols(name);

CREATE TABLE IF NOT EXISTS imports (
  file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
  specifier TEXT NOT NULL,
  kind TEXT NOT NULL,
  resolved_file_id INTEGER,          -- NULL when external or unresolved
  external TEXT,                     -- package name when external
  unresolved INTEGER NOT NULL DEFAULT 0,
  names_json TEXT NOT NULL,
  type_only INTEGER NOT NULL,
  line INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS imports_file ON imports(file_id);
CREATE INDEX IF NOT EXISTS imports_target ON imports(resolved_file_id);

CREATE TABLE IF NOT EXISTS edges (
  from_symbol INTEGER NOT NULL,
  to_symbol INTEGER,                 -- NULL when the target is external or unresolved
  to_external TEXT,                  -- "module\u0000name" or unresolved text
  kind TEXT NOT NULL,
  confidence TEXT NOT NULL,
  line INTEGER NOT NULL,
  from_file INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS edges_from ON edges(from_symbol);
CREATE INDEX IF NOT EXISTS edges_to ON edges(to_symbol);
CREATE INDEX IF NOT EXISTS edges_file ON edges(from_file);

CREATE TABLE IF NOT EXISTS modules (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  role TEXT,
  color TEXT NOT NULL,
  source TEXT NOT NULL,              -- cluster | ai | user
  dir TEXT NOT NULL DEFAULT ''
);

CREATE TABLE IF NOT EXISTS file_modules (
  file_id INTEGER PRIMARY KEY,
  module_id TEXT NOT NULL
);

-- Cache for facts, flows and graph aggregates, keyed by content hash plus version.
CREATE TABLE IF NOT EXISTS derived (
  kind TEXT NOT NULL,
  key_hash INTEGER NOT NULL,
  version INTEGER NOT NULL,
  value BLOB NOT NULL,
  created_at INTEGER NOT NULL,
  PRIMARY KEY (kind, key_hash)
);

CREATE TABLE IF NOT EXISTS jobs (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  kind TEXT NOT NULL,
  target TEXT NOT NULL,
  priority INTEGER NOT NULL,
  state TEXT NOT NULL,
  attempts INTEGER NOT NULL DEFAULT 0,
  updated_at INTEGER NOT NULL,
  UNIQUE (kind, target)
);
CREATE INDEX IF NOT EXISTS jobs_state ON jobs(state, priority);

CREATE TABLE IF NOT EXISTS meta (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
