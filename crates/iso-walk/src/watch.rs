//! File watcher with debounce. Emits batches of repo-relative paths that changed.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crossbeam_channel::{Receiver, RecvTimeoutError, unbounded};
use notify::{RecursiveMode, Watcher as _};

use crate::ALWAYS_SKIP;

/// Keeps the OS watcher alive; drop it to stop watching.
pub struct Watcher {
    _inner: notify::RecommendedWatcher,
    pub batches: Receiver<Vec<String>>,
}

/// Watch `root` recursively. Events are collected until `debounce` passes with no new event,
/// then delivered as one sorted, deduplicated batch. Paths inside skipped directories and
/// gitignored paths are dropped.
pub fn watch(root: &Path, debounce: Duration) -> anyhow::Result<Watcher> {
    let (raw_tx, raw_rx) = unbounded::<PathBuf>();
    let mut inner = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(ev) = res {
            if matches!(ev.kind, notify::EventKind::Access(_)) {
                return;
            }
            for p in ev.paths {
                let _ = raw_tx.send(p);
            }
        }
    })?;
    inner.watch(root, RecursiveMode::Recursive)?;
    let (tx, rx) = unbounded::<Vec<String>>();
    let root = root.to_path_buf();
    let gitignore = {
        let mut b = ignore::gitignore::GitignoreBuilder::new(&root);
        b.add(root.join(".gitignore"));
        b.add(root.join(".isolineignore"));
        b.build().ok()
    };
    std::thread::Builder::new()
        .name("iso-watch".into())
        .spawn(move || {
            let mut pending: BTreeSet<String> = BTreeSet::new();
            loop {
                match raw_rx.recv_timeout(if pending.is_empty() {
                    Duration::from_secs(3600)
                } else {
                    debounce
                }) {
                    Ok(p) => {
                        let Ok(rel) = p.strip_prefix(&root) else {
                            continue;
                        };
                        let rel = rel.to_string_lossy().replace('\\', "/");
                        if rel.is_empty() || rel.split('/').any(|seg| ALWAYS_SKIP.contains(&seg)) {
                            continue;
                        }
                        if gitignore.as_ref().is_some_and(|gi| {
                            gi.matched_path_or_any_parents(&rel, p.is_dir()).is_ignore()
                        }) {
                            continue;
                        }
                        pending.insert(rel);
                    }
                    Err(RecvTimeoutError::Timeout) => {
                        if !pending.is_empty()
                            && tx
                                .send(std::mem::take(&mut pending).into_iter().collect())
                                .is_err()
                        {
                            return;
                        }
                    }
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
        })?;
    Ok(Watcher {
        _inner: inner,
        batches: rx,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debounces_into_one_batch() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().canonicalize().unwrap();
        let w = watch(&root, Duration::from_millis(150)).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        std::fs::write(root.join("a.ts"), "1").unwrap();
        std::fs::write(root.join("b.ts"), "2").unwrap();
        std::fs::write(root.join("a.ts"), "3").unwrap();
        let batch = w
            .batches
            .recv_timeout(Duration::from_secs(5))
            .expect("a batch");
        assert!(batch.contains(&"a.ts".to_string()), "{batch:?}");
        assert!(
            batch.iter().all(|p| p == "a.ts" || p == "b.ts"),
            "{batch:?}"
        );
    }
}
