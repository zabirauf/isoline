//! The request log: every model request of this session, newest last, for the developer
//! panel. Kept in memory only. Holds at most [`MAX_ENTRIES`] requests and about
//! [`MAX_CHARS`] characters of prompts and answers; the oldest go first.

use std::collections::VecDeque;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use iso_core::ipc::{AiRequestDetail, AiRequestRow, AiRequestState, AiTask};
use parking_lot::Mutex;

use crate::{AiError, Completion, CompletionRequest};

pub const MAX_ENTRIES: usize = 300;
pub const MAX_CHARS: usize = 24_000_000;

struct Entry {
    row: AiRequestRow,
    started: Instant,
    system: String,
    prompt: String,
    answer: String,
}

impl Entry {
    fn chars(&self) -> usize {
        self.system.len() + self.prompt.len() + self.answer.len()
    }
}

#[derive(Default)]
struct Inner {
    next: u32,
    chars: usize,
    entries: VecDeque<Entry>,
}

#[derive(Default)]
pub struct RequestLog {
    inner: Mutex<Inner>,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn n32(n: u64) -> u32 {
    n.min(u32::MAX as u64) as u32
}

impl RequestLog {
    /// Records a request as it starts; returns its id.
    pub fn start(&self, task: AiTask, req: &CompletionRequest) -> u32 {
        let mut g = self.inner.lock();
        g.next += 1;
        let id = g.next;
        let e = Entry {
            row: AiRequestRow {
                id,
                task,
                model: req.model.clone(),
                started_ms: now_ms(),
                duration_ms: None,
                state: AiRequestState::Running,
                tokens_in: 0,
                tokens_out: 0,
                cached_tokens_in: 0,
                cost_usd: 0.0,
                max_tokens: req.max_tokens,
                system_chars: n32(req.system.chars().count() as u64),
                prompt_chars: n32(req.prompt.chars().count() as u64),
                answer_chars: 0,
                error: None,
            },
            started: Instant::now(),
            system: req.system.clone(),
            prompt: req.prompt.clone(),
            answer: String::new(),
        };
        g.chars += e.chars();
        g.entries.push_back(e);
        // Keep the newest; a request that still runs can go too, and its finish is ignored.
        while g.entries.len() > 1 && (g.entries.len() > MAX_ENTRIES || g.chars > MAX_CHARS) {
            if let Some(old) = g.entries.pop_front() {
                g.chars -= old.chars();
            }
        }
        id
    }

    /// Records how a request ended.
    pub fn finish(&self, id: u32, res: &Result<Completion, AiError>, cost_usd: f64) {
        let mut g = self.inner.lock();
        let Some(i) = g.entries.iter().position(|e| e.row.id == id) else {
            return;
        };
        let added = {
            let e = &mut g.entries[i];
            e.row.duration_ms = Some(n32(e.started.elapsed().as_millis() as u64));
            match res {
                Ok(c) => {
                    e.row.state = AiRequestState::Done;
                    e.row.model = c.model.clone();
                    e.row.tokens_in = n32(c.tokens_in);
                    e.row.tokens_out = n32(c.tokens_out);
                    e.row.cached_tokens_in = n32(c.cached_tokens_in);
                    e.row.cost_usd = cost_usd;
                    e.row.answer_chars = n32(c.text.chars().count() as u64);
                    e.answer = c.text.clone();
                    e.answer.len()
                }
                Err(err) => {
                    e.row.state = AiRequestState::Failed;
                    e.row.error = Some(err.to_string());
                    0
                }
            }
        };
        g.chars += added;
    }

    /// Every logged request, newest first.
    pub fn rows(&self) -> Vec<AiRequestRow> {
        self.inner
            .lock()
            .entries
            .iter()
            .rev()
            .map(|e| e.row.clone())
            .collect()
    }

    pub fn detail(&self, id: u32) -> Option<AiRequestDetail> {
        let g = self.inner.lock();
        g.entries
            .iter()
            .find(|e| e.row.id == id)
            .map(|e| AiRequestDetail {
                row: e.row.clone(),
                system: e.system.clone(),
                prompt: e.prompt.clone(),
                answer: e.answer.clone(),
            })
    }

    pub fn clear(&self) {
        let mut g = self.inner.lock();
        g.entries.clear();
        g.chars = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(p: &str) -> CompletionRequest {
        CompletionRequest {
            model: "m".into(),
            system: "sys".into(),
            prompt: p.into(),
            max_tokens: 10,
        }
    }

    #[test]
    fn records_start_finish_and_order() {
        let log = RequestLog::default();
        let a = log.start(AiTask::Summaries, &req("one"));
        let b = log.start(AiTask::Review, &req("two"));
        assert_eq!(log.rows()[0].state, AiRequestState::Running);
        log.finish(
            a,
            &Ok(Completion {
                text: "answer".into(),
                model: "m-2".into(),
                tokens_in: 5,
                tokens_out: 2,
                cached_tokens_in: 1,
            }),
            0.5,
        );
        log.finish(b, &Err(AiError::Provider("boom".into())), 0.0);
        let rows = log.rows();
        assert_eq!(rows.iter().map(|r| r.id).collect::<Vec<_>>(), vec![b, a]);
        assert_eq!(rows[1].state, AiRequestState::Done);
        assert_eq!(rows[1].model, "m-2");
        assert_eq!(rows[0].error.as_deref(), Some("provider error: boom"));
        let d = log.detail(a).unwrap();
        assert_eq!((d.prompt.as_str(), d.answer.as_str()), ("one", "answer"));
        log.clear();
        assert!(log.rows().is_empty());
    }

    #[test]
    fn drops_the_oldest_over_the_limit() {
        let log = RequestLog::default();
        for _ in 0..MAX_ENTRIES + 5 {
            log.start(AiTask::Summaries, &req("x"));
        }
        let rows = log.rows();
        assert_eq!(rows.len(), MAX_ENTRIES);
        assert_eq!(rows.last().unwrap().id, 6);
    }
}
