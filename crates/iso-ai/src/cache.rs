//! The AI cache: every model answer, keyed by `iso_core::cache::ai_key(content hash,
//! PROMPT_VERSION, model, kind)`. One database is shared by all repos, because an answer
//! depends only on the content it describes, the prompt version and the model.
//!
//! The schema is `crates/iso-index/migrations/ai_cache.sql`.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use std::borrow::Cow;
use std::collections::HashMap;

use parking_lot::{Mutex, RwLock};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::prompts::PROMPT_VERSION;

const SCHEMA: &str = include_str!("../../iso-index/migrations/ai_cache.sql");

/// Cache kinds.
pub mod kind {
    /// Function, method and type summaries.
    pub const LEAF: &str = "leaf";
    /// File overviews.
    pub const FILE: &str = "file";
    /// Module names and roles.
    pub const CLUSTER: &str = "cluster";
    /// The architecture overview.
    pub const REPO: &str = "repo";
    /// Change reviews (compare, pull requests).
    pub const REVIEW: &str = "review";
}

/// One stored answer.
#[derive(Clone, Debug, PartialEq)]
pub struct CacheEntry {
    pub output_json: String,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub created_at: i64,
}

/// The shared AI cache.
pub struct AiCache {
    conn: Mutex<Connection>,
    /// Per kind: the hash of a prompt the user wrote. It is added to the content hash, so
    /// answers to a changed prompt never mix with the default prompt's answers.
    salts: RwLock<HashMap<String, String>>,
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// The cache key, as SQLite stores it.
pub fn key(content_hash: &str, model: &str, kind: &str) -> i64 {
    iso_core::cache::ai_key(content_hash, PROMPT_VERSION, model, kind) as i64
}

impl AiCache {
    /// Opens (or creates) the cache database at `path`.
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        if let Some(dir) = path.parent()
            && !dir.as_os_str().is_empty()
        {
            let _ = std::fs::create_dir_all(dir);
        }
        let conn = Connection::open(path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        // journal_mode returns the new mode as a row, so it is read rather than executed.
        let _mode: String = conn.query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        Self::init(conn)
    }

    /// An in-memory cache, for tests.
    pub fn in_memory() -> rusqlite::Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> rusqlite::Result<Self> {
        conn.execute_batch(SCHEMA)?;
        Ok(AiCache {
            conn: Mutex::new(conn),
            salts: RwLock::new(HashMap::new()),
        })
    }

    /// Keeps the answers of one kind apart for a changed prompt (`salt` is its hash), or
    /// back with the default prompt's answers (`None`).
    pub fn set_salt(&self, kind: &str, salt: Option<String>) {
        let mut g = self.salts.write();
        match salt {
            Some(s) => g.insert(kind.to_string(), s),
            None => g.remove(kind),
        };
    }

    fn salted<'a>(&self, content_hash: &'a str, kind: &str) -> Cow<'a, str> {
        match self.salts.read().get(kind) {
            Some(s) => Cow::Owned(format!("{content_hash}~p{s}")),
            None => Cow::Borrowed(content_hash),
        }
    }

    /// The stored answer for this content, model and kind under the current prompt version.
    pub fn get(&self, content_hash: &str, model: &str, kind: &str) -> Option<CacheEntry> {
        let content_hash = &*self.salted(content_hash, kind);
        let conn = self.conn.lock();
        conn.query_row(
            "SELECT output_json, tokens_in, tokens_out, created_at FROM ai_cache
             WHERE key = ?1 AND content_hash = ?2 AND prompt_version = ?3 AND model = ?4 AND kind = ?5",
            params![
                key(content_hash, model, kind),
                content_hash,
                PROMPT_VERSION,
                model,
                kind
            ],
            |r| {
                Ok(CacheEntry {
                    output_json: r.get(0)?,
                    tokens_in: r.get::<_, i64>(1)? as u64,
                    tokens_out: r.get::<_, i64>(2)? as u64,
                    created_at: r.get(3)?,
                })
            },
        )
        .optional()
        .ok()
        .flatten()
    }

    /// Whether an answer is stored.
    pub fn contains(&self, content_hash: &str, model: &str, kind: &str) -> bool {
        self.get(content_hash, model, kind).is_some()
    }

    /// The stored answer, decoded.
    pub fn get_json<T: DeserializeOwned>(
        &self,
        content_hash: &str,
        model: &str,
        kind: &str,
    ) -> Option<T> {
        self.get(content_hash, model, kind)
            .and_then(|e| serde_json::from_str(&e.output_json).ok())
    }

    /// Stores (or replaces) an answer.
    pub fn put(
        &self,
        content_hash: &str,
        model: &str,
        kind: &str,
        output_json: &str,
        tokens_in: u64,
        tokens_out: u64,
    ) -> rusqlite::Result<()> {
        let content_hash = &*self.salted(content_hash, kind);
        let conn = self.conn.lock();
        conn.execute(
            "INSERT OR REPLACE INTO ai_cache
             (key, content_hash, prompt_version, model, kind, output_json, tokens_in, tokens_out, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                key(content_hash, model, kind),
                content_hash,
                PROMPT_VERSION,
                model,
                kind,
                output_json,
                tokens_in as i64,
                tokens_out as i64,
                now()
            ],
        )?;
        Ok(())
    }

    /// Stores a value as JSON.
    pub fn put_json<T: Serialize>(
        &self,
        content_hash: &str,
        model: &str,
        kind: &str,
        value: &T,
        tokens_in: u64,
        tokens_out: u64,
    ) -> rusqlite::Result<()> {
        let json = serde_json::to_string(value)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        self.put(content_hash, model, kind, &json, tokens_in, tokens_out)
    }

    /// Number of stored answers.
    pub fn len(&self) -> usize {
        let conn = self.conn.lock();
        conn.query_row("SELECT COUNT(*) FROM ai_cache", [], |r| r.get::<_, i64>(0))
            .unwrap_or(0) as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Removes answers written for older prompt versions.
    pub fn prune_old_versions(&self) -> rusqlite::Result<usize> {
        let conn = self.conn.lock();
        conn.execute(
            "DELETE FROM ai_cache WHERE prompt_version <> ?1",
            params![PROMPT_VERSION],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub/ai_cache.sqlite");
        let c = AiCache::open(&path).unwrap();
        assert!(c.is_empty());
        c.put("h1", "m", kind::LEAF, "{\"a\":1}", 10, 5).unwrap();
        let e = c.get("h1", "m", kind::LEAF).unwrap();
        assert_eq!(e.output_json, "{\"a\":1}");
        assert_eq!((e.tokens_in, e.tokens_out), (10, 5));
        assert!(c.get("h1", "other", kind::LEAF).is_none());
        assert!(c.get("h1", "m", kind::FILE).is_none());
        assert!(c.get("h2", "m", kind::LEAF).is_none());
        c.put("h1", "m", kind::LEAF, "{\"a\":2}", 1, 1).unwrap();
        assert_eq!(c.len(), 1);
        drop(c);
        // Persisted and shared: reopening sees the answer.
        let c = AiCache::open(&path).unwrap();
        let v: serde_json::Value = c.get_json("h1", "m", kind::LEAF).unwrap();
        assert_eq!(v["a"], 2);
        assert_eq!(c.prune_old_versions().unwrap(), 0);
        // A changed prompt keeps its own answers; resetting it brings the old ones back.
        c.set_salt(kind::LEAF, Some("abc".into()));
        assert!(c.get("h1", "m", kind::LEAF).is_none());
        c.put("h1", "m", kind::LEAF, "{\"a\":3}", 1, 1).unwrap();
        assert_eq!(
            c.get_json::<serde_json::Value>("h1", "m", kind::LEAF)
                .unwrap()["a"],
            3
        );
        c.set_salt(kind::LEAF, None);
        assert_eq!(
            c.get_json::<serde_json::Value>("h1", "m", kind::LEAF)
                .unwrap()["a"],
            2
        );
    }
}
