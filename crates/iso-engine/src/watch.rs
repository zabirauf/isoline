//! Watcher loop: debounced batches of changed paths run through the incremental pipeline.

use std::sync::{Arc, Weak};
use std::time::Duration;

use anyhow::Result;

use crate::{Engine, Event};

impl Engine {
    /// Start watching the repo. Each batch reindexes only the changed paths and emits
    /// `FilesChanged` with the paths whose facts changed.
    pub fn start_watch(self: &Arc<Self>) -> Result<()> {
        let w = iso_walk::watch::watch(&self.root, Duration::from_millis(120))?;
        let rx = w.batches.clone();
        self.state.lock().watcher = Some(w);
        let weak: Weak<Engine> = Arc::downgrade(self);
        std::thread::Builder::new()
            .name("iso-reindex".into())
            .spawn(move || {
                iso_jobs::background_thread();
                while let Ok(batch) = rx.recv() {
                    let Some(eng) = weak.upgrade() else { return };
                    eng.fs_epoch
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    // index_paths waits for a running full index, so no batch is lost.
                    if let Ok(rep) = eng.index_paths(&batch)
                        && (rep.facts_changed > 0 || rep.deleted > 0 || rep.added > 0)
                    {
                        eng.emit(Event::FilesChanged(batch));
                        if let Ok(s) = eng.summary() {
                            eng.emit(Event::IndexDone(s));
                        }
                    }
                }
            })?;
        Ok(())
    }

    pub fn stop_watch(&self) {
        self.state.lock().watcher = None;
    }
}
