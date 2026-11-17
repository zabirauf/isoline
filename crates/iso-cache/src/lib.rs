//! Versioned content-addressed store with early cutoff.
//!
//! - [`Derived`]: typed get/put over the `derived` table, keyed by `iso_core::cache::derived_key`,
//!   with an in-memory LRU in front and hit/miss counters.
//! - [`facts_hash`] and [`exports_hash`]: the early-cutoff fingerprints. A file whose exports hash
//!   is unchanged leaves its dependents alone; a symbol whose body hash is unchanged keeps its
//!   flow tree and summary.

use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Result;
use iso_core::cache::{DerivedKind, derived_key};
use iso_core::{FileFacts, hash64};
use iso_index::rusqlite::Connection;
use lru::LruCache;
use parking_lot::Mutex;
use serde::Serialize;
use serde::de::DeserializeOwned;

/// Cache statistics, logged per operation by the eval loop.
#[derive(Default, Debug)]
pub struct Stats {
    pub hits: AtomicU64,
    pub misses: AtomicU64,
    pub mem_hits: AtomicU64,
}

impl Stats {
    pub fn hit_rate(&self) -> f64 {
        let h = self.hits.load(Ordering::Relaxed) as f64;
        let m = self.misses.load(Ordering::Relaxed) as f64;
        if h + m == 0.0 { 1.0 } else { h / (h + m) }
    }
    pub fn snapshot(&self) -> (u64, u64) {
        (
            self.hits.load(Ordering::Relaxed),
            self.misses.load(Ordering::Relaxed),
        )
    }
    pub fn reset(&self) {
        self.hits.store(0, Ordering::Relaxed);
        self.misses.store(0, Ordering::Relaxed);
        self.mem_hits.store(0, Ordering::Relaxed);
    }
}

type MemCache = LruCache<(DerivedKind, u64), std::sync::Arc<Vec<u8>>>;

/// Typed derived-result cache: memory LRU over the `derived` table.
pub struct Derived {
    mem: Mutex<MemCache>,
    pub stats: Stats,
}

impl Default for Derived {
    fn default() -> Self {
        Self::new(512)
    }
}

impl Derived {
    pub fn new(capacity: usize) -> Self {
        Derived {
            mem: Mutex::new(LruCache::new(NonZeroUsize::new(capacity.max(1)).unwrap())),
            stats: Stats::default(),
        }
    }

    pub fn key(kind: DerivedKind, content_hash: &str, version: u32) -> u64 {
        derived_key(kind, content_hash, version)
    }

    /// Look up a value: memory first, then the database.
    pub fn get<T: DeserializeOwned>(
        &self,
        conn: &Connection,
        kind: DerivedKind,
        content_hash: &str,
        version: u32,
    ) -> Result<Option<T>> {
        let key = Self::key(kind, content_hash, version);
        if let Some(b) = self.mem.lock().get(&(kind, key)).cloned() {
            self.stats.hits.fetch_add(1, Ordering::Relaxed);
            self.stats.mem_hits.fetch_add(1, Ordering::Relaxed);
            return Ok(Some(serde_json::from_slice(&b)?));
        }
        match iso_index::derived_get(conn, kind.as_str(), key)? {
            Some(b) => {
                self.stats.hits.fetch_add(1, Ordering::Relaxed);
                let v = serde_json::from_slice(&b)?;
                self.mem.lock().put((kind, key), std::sync::Arc::new(b));
                Ok(Some(v))
            }
            None => {
                self.stats.misses.fetch_add(1, Ordering::Relaxed);
                Ok(None)
            }
        }
    }

    /// Serialize once; store in memory now and return the bytes so the caller can write them to
    /// the database in its own batched transaction.
    pub fn remember<T: Serialize>(
        &self,
        kind: DerivedKind,
        content_hash: &str,
        version: u32,
        value: &T,
    ) -> Result<(u64, Vec<u8>)> {
        let key = Self::key(kind, content_hash, version);
        let bytes = serde_json::to_vec(value)?;
        self.mem
            .lock()
            .put((kind, key), std::sync::Arc::new(bytes.clone()));
        Ok((key, bytes))
    }

    pub fn put<T: Serialize>(
        &self,
        conn: &Connection,
        kind: DerivedKind,
        content_hash: &str,
        version: u32,
        value: &T,
    ) -> Result<()> {
        let (key, bytes) = self.remember(kind, content_hash, version, value)?;
        iso_index::derived_put(conn, kind.as_str(), key, version, &bytes)
    }
}

/// Hash of everything an extractor produced. Equal hashes mean nothing downstream changes.
pub fn facts_hash(f: &FileFacts) -> String {
    format!(
        "{:016x}",
        hash64(&serde_json::to_vec(f).unwrap_or_default())
    )
}

/// Hash of the file's exported surface: exported symbol names, kinds and signatures, plus
/// re-exports. A body-only or comment edit leaves it unchanged, so importers are not touched.
pub fn exports_hash(f: &FileFacts) -> String {
    let mut parts: Vec<String> = f
        .symbols
        .iter()
        .filter(|s| s.exported)
        .map(|s| format!("{}|{}|{}", s.qualified_name, s.kind.as_str(), s.signature))
        .collect();
    for i in &f.imports {
        if i.kind == iso_core::ImportKind::Reexport {
            let names: Vec<&str> = i.names.iter().map(|n| n.local()).collect();
            parts.push(format!("reexport|{}|{}", i.specifier, names.join(",")));
        }
    }
    parts.sort();
    format!("{:016x}", hash64(parts.join("\n").as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use iso_core::{SymbolFact, SymbolKind};

    fn sym(exported: bool, body: u64, line: u32) -> SymbolFact {
        SymbolFact {
            name: "f".into(),
            qualified_name: "f".into(),
            kind: SymbolKind::Function,
            start_line: line,
            end_line: line + 3,
            signature: "function f(a)".into(),
            doc: None,
            exported,
            is_async: false,
            params: vec!["a".into()],
            parent: None,
            body_hash: body,
        }
    }

    #[test]
    fn body_edit_keeps_exports_hash() {
        let a = FileFacts {
            symbols: vec![sym(true, 1, 1)],
            ..Default::default()
        };
        let b = FileFacts {
            symbols: vec![sym(true, 2, 5)],
            ..Default::default()
        };
        assert_eq!(exports_hash(&a), exports_hash(&b));
        assert_ne!(facts_hash(&a), facts_hash(&b));
        let c = FileFacts {
            symbols: vec![sym(false, 1, 1)],
            ..Default::default()
        };
        assert_ne!(exports_hash(&a), exports_hash(&c));
    }

    #[test]
    fn derived_roundtrip_counts_hits() {
        let t = tempfile_dir();
        let idx = iso_index::Index::open(&t.join("d.sqlite")).unwrap();
        let d = Derived::new(4);
        idx.write(|tx| {
            assert!(d.get::<Vec<u32>>(tx, DerivedKind::Facts, "h", 1)?.is_none());
            d.put(tx, DerivedKind::Facts, "h", 1, &vec![1u32, 2])?;
            Ok(())
        })
        .unwrap();
        let d2 = Derived::new(4);
        idx.read(|c| {
            assert_eq!(
                d2.get::<Vec<u32>>(c, DerivedKind::Facts, "h", 1)?,
                Some(vec![1, 2])
            );
            assert!(d2.get::<Vec<u32>>(c, DerivedKind::Facts, "h", 2)?.is_none());
            Ok(())
        })
        .unwrap();
        assert_eq!(d2.stats.snapshot(), (1, 1));
        std::fs::remove_dir_all(t).ok();
    }

    fn tempfile_dir() -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("iso-cache-test-{}", std::process::id()));
        std::fs::create_dir_all(&p).unwrap();
        p
    }
}
