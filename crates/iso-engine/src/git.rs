//! Git operations behind the branch menu: status, refs, switching, fetching, pull requests.

use std::sync::Arc;

use anyhow::{Result, anyhow};
use iso_core::ipc::{GitRefs, GitStatus, PullRequests};

use crate::{Engine, Event};

fn not_a_repo() -> GitStatus {
    GitStatus {
        is_repo: false,
        branch: None,
        head: None,
        upstream: None,
        ahead: 0,
        behind: 0,
        changed: 0,
        untracked: 0,
        operation: None,
        github: false,
    }
}

#[derive(Clone, Debug)]
pub(crate) struct StatusKey {
    files: Vec<(u64, u128)>,
    epoch: u64,
    at: std::time::Instant,
}

/// Paths the walker would also index: no hidden or always-skipped folder on the way.
fn walkable(p: &str) -> bool {
    !p.split('/')
        .any(|seg| seg.starts_with('.') || iso_walk::ALWAYS_SKIP.contains(&seg))
}

impl Engine {
    /// The git work tree this repo is in, if any.
    pub fn git_repo(&self) -> Option<&iso_git::Repo> {
        self.git
            .get_or_init(|| iso_git::Repo::open(&self.root))
            .as_ref()
    }

    pub(crate) fn git_or_err(&self) -> Result<&iso_git::Repo> {
        self.git_repo()
            .ok_or_else(|| anyhow!("{} is not in a git repository.", self.name))
    }

    /// Status, recomputed only when git's own files or the watched work tree changed (or a
    /// minute passed): `git status` on a 60k-file repo costs about half a second, and the UI
    /// asks on every focus and every few seconds.
    pub fn git_status(&self) -> Result<GitStatus> {
        let Some(r) = self.git_repo() else {
            return Ok(not_a_repo());
        };
        let key = StatusKey {
            files: r.state_stamp(),
            epoch: self.fs_epoch.load(std::sync::atomic::Ordering::Relaxed),
            at: std::time::Instant::now(),
        };
        if let Some((k, st)) = self.git_status_cache.lock().as_ref()
            && k.files == key.files
            && k.epoch == key.epoch
            && k.at.elapsed() < std::time::Duration::from_secs(60)
        {
            return Ok(st.clone());
        }
        let st = r.status()?;
        *self.git_status_cache.lock() = Some((key, st.clone()));
        Ok(st)
    }

    pub fn git_refs(&self) -> Result<GitRefs> {
        self.git_or_err()?.refs()
    }

    pub fn git_prs(&self) -> PullRequests {
        match self.git_repo() {
            Some(r) => r.pull_requests(),
            None => PullRequests {
                available: false,
                message: Some("This folder is not in a git repository.".into()),
                items: Vec::new(),
            },
        }
    }

    /// Switch branches, then reindex the files that differ between the two commits right away
    /// (the watcher would see them too, a little later and in pieces).
    pub fn git_switch(self: &Arc<Self>, name: &str, create: bool) -> Result<GitStatus> {
        let r = self.git_or_err()?;
        let before = r.rev_parse("HEAD").ok();
        r.switch(name, create)?;
        let after = r.rev_parse("HEAD").ok();
        *self.git_status_cache.lock() = None;
        let st = self.git_status()?;
        self.emit(Event::GitChanged(st.clone()));
        if let (Some(b), Some(a)) = (before, after)
            && a != b
        {
            let changed = r.diff(&b, Some(&a))?;
            let mut paths: Vec<String> = changed
                .iter()
                .flat_map(|c| std::iter::once(c.path.clone()).chain(c.old_path.clone()))
                .filter(|p| walkable(p))
                .collect();
            paths.sort();
            paths.dedup();
            if !paths.is_empty() {
                let eng = Arc::clone(self);
                std::thread::Builder::new()
                    .name("iso-switch-reindex".into())
                    .spawn(move || {
                        if let Ok(rep) = eng.index_paths(&paths) {
                            if rep.facts_changed > 0 || rep.added > 0 || rep.deleted > 0 {
                                eng.emit(Event::FilesChanged(paths));
                            }
                            if let Ok(s) = eng.summary() {
                                eng.emit(Event::IndexDone(s));
                            }
                        }
                    })?;
            }
        }
        Ok(st)
    }

    pub fn git_fetch(&self) -> Result<GitStatus> {
        let r = self.git_or_err()?;
        r.fetch()?;
        *self.git_status_cache.lock() = None;
        let st = self.git_status()?;
        self.emit(Event::GitChanged(st.clone()));
        Ok(st)
    }
}
