//! Priority background queue.
//!
//! Jobs are keyed by (kind, target) and deduplicated; enqueuing an existing job only raises its
//! priority. Workers block on [`JobQueue::pop`]. The queue can be paused (the UI pauses it while
//! the user scrolls or types) and drained on cancel. Persistence is the caller's: job state that
//! must survive a restart is derived from the index (a file is done when its stored content
//! hash matches), so a killed process redoes no finished work.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use iso_core::jobs::{JobKind, Priority};
use parking_lot::{Condvar, Mutex};

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct JobKey {
    pub kind: JobKind,
    pub target: String,
}

#[derive(Default)]
struct Inner {
    heap: BinaryHeap<(Reverse<u8>, Reverse<u64>, JobKey)>,
    /// Current priority of each queued job; heap entries with another priority are stale.
    queued: HashMap<JobKey, u8>,
    running: usize,
    seq: u64,
    closed: bool,
}

/// Counters for `get_jobs`.
#[derive(Default, Debug)]
pub struct Counters {
    pub done: AtomicU64,
    pub failed: AtomicU64,
}

#[derive(Clone, Default)]
pub struct JobQueue {
    inner: Arc<(Mutex<Inner>, Condvar)>,
    paused: Arc<AtomicBool>,
    pub counters: Arc<Counters>,
}

impl JobQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a job or raise its priority. Returns true when newly queued.
    pub fn push(&self, kind: JobKind, target: impl Into<String>, priority: Priority) -> bool {
        let key = JobKey {
            kind,
            target: target.into(),
        };
        let p = priority as u8;
        let (lock, cv) = &*self.inner;
        let mut g = lock.lock();
        let fresh = match g.queued.get(&key) {
            Some(&old) if old <= p => return false,
            Some(_) => false,
            None => true,
        };
        g.seq += 1;
        let seq = g.seq;
        g.queued.insert(key.clone(), p);
        g.heap.push((Reverse(p), Reverse(seq), key));
        cv.notify_one();
        fresh
    }

    /// Raise priority only for jobs already queued.
    pub fn bump(&self, kind: JobKind, target: &str, priority: Priority) {
        let key = JobKey {
            kind,
            target: target.to_string(),
        };
        let queued = self.inner.0.lock().queued.contains_key(&key);
        if queued {
            self.push(kind, target, priority);
        }
    }

    /// Block until a job is available (or the queue closes). Respects pause.
    pub fn pop(&self) -> Option<(JobKey, Priority)> {
        let (lock, cv) = &*self.inner;
        let mut g = lock.lock();
        loop {
            if g.closed {
                return None;
            }
            if !self.paused.load(Ordering::Relaxed) {
                while let Some((Reverse(p), _, key)) = g.heap.pop() {
                    if g.queued.get(&key) == Some(&p) {
                        g.queued.remove(&key);
                        g.running += 1;
                        return Some((key, priority_from(p)));
                    }
                }
            }
            cv.wait_for(&mut g, Duration::from_millis(50));
        }
    }

    /// Non-blocking pop.
    pub fn try_pop(&self) -> Option<(JobKey, Priority)> {
        let mut g = self.inner.0.lock();
        if self.paused.load(Ordering::Relaxed) {
            return None;
        }
        while let Some((Reverse(p), _, key)) = g.heap.pop() {
            if g.queued.get(&key) == Some(&p) {
                g.queued.remove(&key);
                g.running += 1;
                return Some((key, priority_from(p)));
            }
        }
        None
    }

    pub fn finish(&self, ok: bool) {
        let mut g = self.inner.0.lock();
        g.running = g.running.saturating_sub(1);
        if ok {
            self.counters.done.fetch_add(1, Ordering::Relaxed);
        } else {
            self.counters.failed.fetch_add(1, Ordering::Relaxed);
        }
        self.inner.1.notify_all();
    }

    pub fn pause(&self, on: bool) {
        self.paused.store(on, Ordering::Relaxed);
        self.inner.1.notify_all();
    }

    /// Drop every queued job of a kind (or all kinds).
    pub fn cancel(&self, kind: Option<JobKind>) -> usize {
        let mut g = self.inner.0.lock();
        let before = g.queued.len();
        g.queued.retain(|k, _| kind.is_some_and(|kd| k.kind != kd));
        let removed = before - g.queued.len();
        let queued = std::mem::take(&mut g.queued);
        g.heap.retain(|(Reverse(p), _, k)| queued.get(k) == Some(p));
        g.queued = queued;
        removed
    }

    pub fn close(&self) {
        self.inner.0.lock().closed = true;
        self.inner.1.notify_all();
    }

    /// (queued, running)
    pub fn sizes(&self) -> (usize, usize) {
        let g = self.inner.0.lock();
        (g.queued.len(), g.running)
    }

    /// Queued targets of one kind, highest priority first (for the per-file status dots).
    pub fn queued_targets(&self, kind: JobKind, limit: usize) -> Vec<String> {
        let g = self.inner.0.lock();
        let mut v: Vec<(&u8, &JobKey)> = g
            .queued
            .iter()
            .filter(|(k, _)| k.kind == kind)
            .map(|(k, p)| (p, k))
            .collect();
        v.sort();
        v.into_iter()
            .take(limit)
            .map(|(_, k)| k.target.clone())
            .collect()
    }

    /// Block until nothing is queued or running.
    pub fn wait_idle(&self) {
        let (lock, cv) = &*self.inner;
        let mut g = lock.lock();
        while !(g.queued.is_empty() && g.running == 0) && !g.closed {
            cv.wait_for(&mut g, Duration::from_millis(20));
        }
    }
}

fn priority_from(p: u8) -> Priority {
    match p {
        0 => Priority::VisibleFile,
        1 => Priority::Neighbors,
        2 => Priority::ExpandedOrRecent,
        3 => Priority::Repo,
        4 => Priority::Precise,
        _ => Priority::Ai,
    }
}

/// Worker count: all cores but one.
pub fn default_workers() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .saturating_sub(1)
        .max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priority_order_dedupe_and_bump() {
        let q = JobQueue::new();
        assert!(q.push(JobKind::Parse, "a", Priority::Repo));
        assert!(q.push(JobKind::Parse, "b", Priority::Repo));
        assert!(!q.push(JobKind::Parse, "a", Priority::Repo));
        q.push(JobKind::AiLeaf, "a", Priority::Ai);
        q.bump(JobKind::Parse, "b", Priority::VisibleFile);
        q.bump(JobKind::Parse, "zzz", Priority::VisibleFile); // not queued: ignored
        let order: Vec<String> = std::iter::from_fn(|| {
            q.try_pop()
                .map(|(k, _)| format!("{}:{}", k.kind.as_str(), k.target))
        })
        .collect();
        assert_eq!(order, vec!["parse:b", "parse:a", "ai_leaf:a"]);
    }

    #[test]
    fn pause_cancel_and_workers() {
        let q = JobQueue::new();
        q.pause(true);
        q.push(JobKind::Parse, "x", Priority::Repo);
        assert!(q.try_pop().is_none());
        q.pause(false);
        assert_eq!(q.cancel(Some(JobKind::Parse)), 1);
        assert!(q.try_pop().is_none());
        for i in 0..100 {
            q.push(JobKind::Parse, i.to_string(), Priority::Repo);
        }
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let q = q.clone();
                std::thread::spawn(move || {
                    while let Some(_job) = q.try_pop() {
                        q.finish(true);
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        q.wait_idle();
        assert_eq!(q.counters.done.load(Ordering::Relaxed), 100);
    }
}
