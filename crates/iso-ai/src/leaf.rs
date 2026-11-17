//! Leaf pass: summaries of functions and methods, one-sentence type summaries and file
//! overviews. Items whose answer is cached are skipped; the rest are packed, about 20 files
//! per request, behind the stable leaf system message.

use std::collections::{HashMap, HashSet};

use iso_core::ipc::{AiText, Summary};
use iso_core::{FileFacts, SymbolFact, hash64};
use serde::{Deserialize, Serialize};

use crate::cache::{AiCache, kind};
use crate::engine::{AiSettings, FileInput};
use crate::prompts::{
    OUT_TOKENS_ENVELOPE, OUT_TOKENS_FN, OUT_TOKENS_OVERVIEW, OUT_TOKENS_TYPE, leaf_system,
};
use crate::skeleton::{build_skeleton, file_outline, type_skeletons};
use crate::tokens::estimate_tokens;
use crate::{CompletionRequest, parse};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum LeafKind {
    Fn,
    Type,
    Overview,
}

impl LeafKind {
    fn prefix(self) -> char {
        match self {
            LeafKind::Fn => 'f',
            LeafKind::Type => 't',
            LeafKind::Overview => 'o',
        }
    }
    fn cache_kind(self) -> &'static str {
        match self {
            LeafKind::Overview => kind::FILE,
            _ => kind::LEAF,
        }
    }
    fn out_tokens(self) -> u64 {
        match self {
            LeafKind::Fn => OUT_TOKENS_FN,
            LeafKind::Type => OUT_TOKENS_TYPE,
            LeafKind::Overview => OUT_TOKENS_OVERVIEW,
        }
    }
}

/// A type summary as stored in the cache.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct TypeAnswer {
    pub desc: String,
    pub model: String,
}

/// Cache content key of a function or method: its body hash.
pub(crate) fn fn_key(sym: &SymbolFact) -> String {
    format!("{:016x}", sym.body_hash)
}

/// Cache content key of a type: the hash of its skeleton (signature, doc, members).
pub(crate) fn type_key(text: &str) -> String {
    format!("type:{:016x}", hash64(text.as_bytes()))
}

/// Cache content key of a file overview: the hash of its path and outline.
pub(crate) fn overview_key(path: &str, outline: &str) -> String {
    format!(
        "file:{:016x}",
        hash64(format!("{path}\u{0}{outline}").as_bytes())
    )
}

pub(crate) fn overview_key_for(path: &str, lang: iso_core::Language, facts: &FileFacts) -> String {
    overview_key(path, &file_outline(lang, facts))
}

/// One item to send.
#[derive(Clone, Debug)]
pub(crate) struct LeafItem {
    pub kind: LeafKind,
    pub path: String,
    /// Qualified name, or the path for an overview.
    pub name: String,
    pub content: String,
    pub text: String,
}

impl LeafItem {
    fn tokens(&self) -> u64 {
        estimate_tokens(&self.text) + estimate_tokens(&self.name) + 4
    }
}

/// One request's worth of items.
#[derive(Clone, Debug)]
pub(crate) struct LeafBatch {
    /// Items with their ids in the prompt.
    pub items: Vec<(String, LeafItem)>,
    pub request: CompletionRequest,
    pub est_out: u64,
}

pub(crate) struct LeafPlan {
    pub batches: Vec<LeafBatch>,
    /// Every summarizable item in scope.
    pub total: u32,
    /// Items already answered in the cache.
    pub cached: u32,
}

fn lang_of(f: &FileInput) -> &'static str {
    f.lang.label()
}

/// Items in scope that have no cached answer, and the count of those that do.
pub(crate) fn pending_items(
    files: &[FileInput],
    cache: &AiCache,
    model: &str,
    settings: &AiSettings,
) -> (Vec<Vec<LeafItem>>, u32, u32) {
    let mut seen: HashSet<(LeafKind, String)> = HashSet::new();
    let mut total = 0u32;
    let mut cached = 0u32;
    let mut out = Vec::with_capacity(files.len());
    for f in files {
        let mut items = Vec::new();
        // Callable skeletons need the source; build them only when something is missing.
        let mut skeleton: Option<HashMap<String, String>> = None;
        let mut callable_text = |qn: &str| -> String {
            skeleton
                .get_or_insert_with(|| {
                    build_skeleton(
                        &f.path,
                        f.lang,
                        &f.source,
                        &f.facts,
                        &f.flows,
                        &settings.skeleton,
                    )
                    .callables
                    .into_iter()
                    .map(|s| (s.qualified_name, s.text))
                    .collect()
                })
                .get(qn)
                .cloned()
                .unwrap_or_default()
        };
        let mut wanted: Vec<(LeafKind, String, String, Option<String>)> = Vec::new();
        for s in f.facts.symbols.iter().filter(|s| s.kind.is_callable()) {
            wanted.push((LeafKind::Fn, s.qualified_name.clone(), fn_key(s), None));
        }
        for t in type_skeletons(&f.facts, &settings.skeleton) {
            let key = type_key(&t.text);
            wanted.push((LeafKind::Type, t.qualified_name, key, Some(t.text)));
        }
        if settings.file_overviews && !f.facts.symbols.is_empty() {
            let outline = file_outline(f.lang, &f.facts);
            wanted.push((
                LeafKind::Overview,
                f.path.clone(),
                overview_key(&f.path, &outline),
                Some(outline),
            ));
        }
        for (kind, name, content, text) in wanted {
            total += 1;
            if cache.contains(&content, model, kind.cache_kind()) {
                cached += 1;
                continue;
            }
            // Identical bodies share one answer: send the first, the rest read the cache.
            if !seen.insert((kind, content.clone())) {
                continue;
            }
            let text = text.unwrap_or_else(|| callable_text(&name));
            items.push(LeafItem {
                kind,
                path: f.path.clone(),
                name,
                content,
                text,
            });
        }
        out.push(items);
    }
    (out, total, cached)
}

/// Packs pending items into requests.
pub(crate) fn plan(
    files: &[FileInput],
    cache: &AiCache,
    model: &str,
    settings: &AiSettings,
) -> LeafPlan {
    let (per_file, total, cached) = pending_items(files, cache, model, settings);
    let mut batches = Vec::new();
    let mut cur: Vec<(usize, LeafItem)> = Vec::new();
    let mut cur_files: HashSet<usize> = HashSet::new();
    let mut cur_tokens = 0u64;
    let flush = |cur: &mut Vec<(usize, LeafItem)>,
                 cur_files: &mut HashSet<usize>,
                 cur_tokens: &mut u64,
                 batches: &mut Vec<LeafBatch>| {
        if cur.is_empty() {
            return;
        }
        batches.push(build_batch(files, std::mem::take(cur), model, settings));
        cur_files.clear();
        *cur_tokens = 0;
    };
    for (fi, items) in per_file.into_iter().enumerate() {
        if items.is_empty() {
            continue;
        }
        let file_tokens: u64 = items.iter().map(|i| i.tokens()).sum();
        if !cur.is_empty()
            && (cur_files.len() >= settings.files_per_batch
                || cur_tokens + file_tokens > settings.max_batch_tokens
                || cur.len() + items.len() > settings.max_items_per_batch)
        {
            flush(&mut cur, &mut cur_files, &mut cur_tokens, &mut batches);
        }
        for item in items {
            let t = item.tokens();
            if !cur.is_empty()
                && (cur_tokens + t > settings.max_batch_tokens
                    || cur.len() >= settings.max_items_per_batch)
            {
                flush(&mut cur, &mut cur_files, &mut cur_tokens, &mut batches);
            }
            cur_tokens += t;
            cur_files.insert(fi);
            cur.push((fi, item));
        }
    }
    flush(&mut cur, &mut cur_files, &mut cur_tokens, &mut batches);
    LeafPlan {
        batches,
        total,
        cached,
    }
}

fn build_batch(
    files: &[FileInput],
    items: Vec<(usize, LeafItem)>,
    model: &str,
    settings: &AiSettings,
) -> LeafBatch {
    let mut prompt = String::from(
        "Task: leaf\nSummarize every item below. Answer with the JSON object described in the instructions.\n",
    );
    let mut counters: HashMap<char, u32> = HashMap::new();
    let mut last_file = usize::MAX;
    let mut out_items = Vec::with_capacity(items.len());
    let mut est_out = OUT_TOKENS_ENVELOPE;
    for (fi, item) in items {
        if fi != last_file {
            let f = &files[fi];
            prompt.push_str(&format!("\n## file {} ({})\n", f.path, lang_of(f)));
            last_file = fi;
        }
        let n = counters.entry(item.kind.prefix()).or_insert(0);
        *n += 1;
        let id = format!("{}{}", item.kind.prefix(), n);
        prompt.push_str(&format!("### {id} {}\n{}\n", item.name, item.text));
        est_out += item.kind.out_tokens();
        out_items.push((id, item));
    }
    let max_tokens = (est_out * 2 + 256).min(settings.max_output_tokens as u64) as u32;
    LeafBatch {
        items: out_items,
        request: CompletionRequest {
            model: model.to_string(),
            system: leaf_system(),
            prompt,
            max_tokens,
        },
        est_out,
    }
}

/// What one answered batch produced.
pub(crate) struct BatchResult {
    pub stored: u32,
    pub errors: Vec<(String, String)>,
    pub paths: Vec<String>,
}

/// Parses an answer and stores every good item. Bad or missing items are reported, not
/// stored, so the next run asks again.
pub(crate) fn store_answer(
    cache: &AiCache,
    batch: &LeafBatch,
    model: &str,
    answer_model: &str,
    text: &str,
    tokens_in: u64,
    tokens_out: u64,
) -> BatchResult {
    let mut res = BatchResult {
        stored: 0,
        errors: Vec::new(),
        paths: Vec::new(),
    };
    let label = |i: &LeafItem| match i.kind {
        LeafKind::Overview => i.path.clone(),
        _ => format!("{}#{}", i.path, i.name),
    };
    let value = match parse::extract_json(text) {
        Ok(v) => v,
        Err(e) => {
            res.errors = batch
                .items
                .iter()
                .map(|(_, i)| (label(i), e.clone()))
                .collect();
            return res;
        }
    };
    let list = match parse::items(&value) {
        Ok(l) => l,
        Err(e) => {
            res.errors = batch
                .items
                .iter()
                .map(|(_, i)| (label(i), e.clone()))
                .collect();
            return res;
        }
    };
    let by_id: HashMap<String, &serde_json::Value> = list
        .iter()
        .filter_map(|v| parse::item_id(v).map(|id| (id, v)))
        .collect();
    let n = batch.items.len().max(1) as u64;
    let (share_in, share_out) = (tokens_in / n, tokens_out / n);
    let mut paths: HashSet<String> = HashSet::new();
    for (id, item) in &batch.items {
        let Some(v) = by_id.get(id) else {
            res.errors
                .push((label(item), format!("the answer has no entry {id}")));
            continue;
        };
        let stored = match item.kind {
            LeafKind::Fn => parse::parse_summary(v, answer_model).and_then(|s: Summary| {
                cache
                    .put_json(&item.content, model, kind::LEAF, &s, share_in, share_out)
                    .map_err(|e| e.to_string())
            }),
            LeafKind::Type => parse::parse_type_desc(v).and_then(|desc| {
                let a = TypeAnswer {
                    desc,
                    model: answer_model.to_string(),
                };
                cache
                    .put_json(&item.content, model, kind::LEAF, &a, share_in, share_out)
                    .map_err(|e| e.to_string())
            }),
            LeafKind::Overview => parse::parse_overview(v).and_then(|text| {
                let a = AiText {
                    text,
                    model: answer_model.to_string(),
                };
                cache
                    .put_json(&item.content, model, kind::FILE, &a, share_in, share_out)
                    .map_err(|e| e.to_string())
            }),
        };
        match stored {
            Ok(()) => {
                res.stored += 1;
                paths.insert(item.path.clone());
            }
            Err(e) => res.errors.push((label(item), format!("entry {id}: {e}"))),
        }
    }
    let mut p: Vec<String> = paths.into_iter().collect();
    p.sort();
    res.paths = p;
    res
}

/// Cached answers for one file. Needs facts only (the source may be empty).
pub(crate) fn cached_for_file(
    cache: &AiCache,
    model: &str,
    settings: &AiSettings,
    f: &FileInput,
) -> (
    HashMap<String, Summary>,
    HashMap<String, String>,
    Option<AiText>,
) {
    let mut fns = HashMap::new();
    for s in f.facts.symbols.iter().filter(|s| s.kind.is_callable()) {
        if let Some(sum) = cache.get_json::<Summary>(&fn_key(s), model, kind::LEAF) {
            fns.insert(s.qualified_name.clone(), sum);
        }
    }
    let mut types = HashMap::new();
    for t in type_skeletons(&f.facts, &settings.skeleton) {
        if let Some(a) = cache.get_json::<TypeAnswer>(&type_key(&t.text), model, kind::LEAF) {
            types.insert(t.qualified_name.clone(), a.desc);
        }
    }
    let overview = cache.get_json::<AiText>(
        &overview_key_for(&f.path, f.lang, &f.facts),
        model,
        kind::FILE,
    );
    (fns, types, overview)
}
