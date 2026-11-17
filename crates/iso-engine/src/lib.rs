//! The engine: every operation behind an IPC command or a CLI command.
//!
//! `iso-cli` and `app/src-tauri` are thin wrappers over [`Engine`], so the headless tool and
//! the app can never disagree.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result};
use iso_cache::Derived;
use iso_core::ipc::{IndexProgress, RepoSummary};
use iso_index::Index;
use iso_jobs::JobQueue;
use parking_lot::{Mutex, RwLock};

mod ai;
mod edges;
mod indexer;
mod modules;
mod precise;
pub mod sig;
mod view;
mod watch;

pub use ai::AiLayer;
pub use indexer::IndexReport;
pub use precise::PreciseReport;
pub use view::{AiHook, CachedAi};

/// Events the engine emits. The Tauri shell forwards them to the UI; the CLI prints them.
#[derive(Clone, Debug)]
pub enum Event {
    IndexProgress(IndexProgress),
    IndexDone(RepoSummary),
    /// Files whose facts changed after a watcher batch.
    FilesChanged(Vec<String>),
    JobProgress(iso_core::ipc::JobsState),
}

pub type Sink = Arc<dyn Fn(Event) + Send + Sync>;

/// Where Isoline keeps its databases. `ISOLINE_DATA_DIR` overrides the platform default.
pub fn data_dir() -> PathBuf {
    if let Ok(d) = std::env::var("ISOLINE_DATA_DIR") {
        return PathBuf::from(d);
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/dev.isoline.app")
    } else if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or(home)
            .join("Isoline")
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local/share"))
            .join("isoline")
    }
}

/// Database path for one repo root.
pub fn index_path(data: &Path, root: &Path) -> PathBuf {
    let key = format!(
        "{:016x}",
        iso_core::hash64(root.to_string_lossy().as_bytes())
    );
    data.join("repos").join(key).join("index.sqlite")
}

pub(crate) struct State {
    pub resolvers: Option<(u64, Arc<iso_resolve::Resolvers>)>,
    pub watcher: Option<iso_walk::watch::Watcher>,
    /// The Merkle tree, kept in memory after its first load and updated in place.
    pub tree: Option<iso_walk::MerkleTree>,
}

pub struct Engine {
    pub root: PathBuf,
    pub name: String,
    pub index: Index,
    pub derived: Derived,
    pub queue: JobQueue,
    pub(crate) sink: RwLock<Option<Sink>>,
    pub(crate) state: Mutex<State>,
    pub(crate) indexing: AtomicBool,
    /// Serializes index runs (full and incremental) so writes never interleave.
    pub(crate) index_lock: Mutex<()>,
    pub(crate) ai_enabled: AtomicBool,
    pub(crate) ai_hook: RwLock<Option<Arc<dyn AiHook>>>,
}

impl Engine {
    /// Open (or create) the index for a repo. Does not index; call [`Engine::index`].
    pub fn open(root: &Path) -> Result<Arc<Engine>> {
        Self::open_with_data(root, &data_dir())
    }

    pub fn open_with_data(root: &Path, data: &Path) -> Result<Arc<Engine>> {
        let root = root
            .canonicalize()
            .with_context(|| format!("no such folder: {}", root.display()))?;
        anyhow::ensure!(root.is_dir(), "not a folder: {}", root.display());
        let index = Index::open(&index_path(data, &root))?;
        index.write(|tx| {
            iso_index::meta_set(tx, "root", &root.to_string_lossy())?;
            // An upgraded extractor changes facts for unchanged files: reprocess everything once.
            let v = iso_core::cache::EXTRACTOR_VERSION.to_string();
            if iso_index::meta_get(tx, "extractor_version")?.as_deref() != Some(v.as_str()) {
                tx.execute("UPDATE files SET status = 'pending'", [])?;
                iso_index::meta_set(tx, "extractor_version", &v)?;
            }
            Ok(())
        })?;
        let name = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| root.to_string_lossy().into_owned());
        let ai = index
            .read(|c| iso_index::meta_get(c, "ai_enabled"))?
            .as_deref()
            == Some("1");
        remember_recent(data, &root);
        Ok(Arc::new(Engine {
            root,
            name,
            index,
            derived: Derived::new(2048),
            queue: JobQueue::new(),
            sink: RwLock::new(None),
            state: Mutex::new(State {
                resolvers: None,
                watcher: None,
                tree: None,
            }),
            indexing: AtomicBool::new(false),
            index_lock: Mutex::new(()),
            ai_enabled: AtomicBool::new(ai),
            ai_hook: RwLock::new(None),
        }))
    }

    pub fn set_sink(&self, sink: Option<Sink>) {
        *self.sink.write() = sink;
    }

    pub(crate) fn emit(&self, e: Event) {
        if let Some(s) = self.sink.read().as_ref() {
            s(e);
        }
    }

    pub fn is_indexing(&self) -> bool {
        self.indexing.load(Ordering::Relaxed)
    }

    pub fn set_ai_enabled(&self, on: bool) -> Result<()> {
        self.ai_enabled.store(on, Ordering::Relaxed);
        self.index
            .write(|tx| iso_index::meta_set(tx, "ai_enabled", if on { "1" } else { "0" }))
    }

    /// Background queue state and cache hit rate, for `get_jobs`.
    pub fn jobs_state(&self) -> iso_core::ipc::JobsState {
        let (queued, running) = self.queue.sizes();
        let (hits, misses) = self.derived.stats.snapshot();
        iso_core::ipc::JobsState {
            queued: queued as u32,
            running: running as u32,
            done: self.queue.counters.done.load(Ordering::Relaxed) as u32,
            failed: self.queue.counters.failed.load(Ordering::Relaxed) as u32,
            cache_hits: hits,
            cache_misses: misses,
            hit_rate: self.derived.stats.hit_rate(),
            pending_files: self
                .queue
                .queued_targets(iso_core::jobs::JobKind::Parse, 200),
        }
    }

    pub fn set_ai_hook(&self, hook: Option<Arc<dyn AiHook>>) {
        *self.ai_hook.write() = hook;
    }

    pub fn ai_enabled(&self) -> bool {
        self.ai_enabled.load(Ordering::Relaxed)
    }
}

/// Recently opened repos, most recent first.
pub fn recent_repos(data: &Path) -> Vec<String> {
    std::fs::read_to_string(data.join("recent.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn remember_recent(data: &Path, root: &Path) {
    let r = root.to_string_lossy().into_owned();
    let mut list = recent_repos(data);
    list.retain(|x| x != &r);
    list.insert(0, r);
    list.truncate(12);
    let _ = std::fs::create_dir_all(data);
    let _ = std::fs::write(
        data.join("recent.json"),
        serde_json::to_string(&list).unwrap_or_default(),
    );
}
