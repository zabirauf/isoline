//! Precise indexers: run what the machine has, import SCIP, upgrade and correct edges.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use iso_core::{Confidence, FileId, Language, SymbolId};
use iso_resolve::scip::ExactRef;
use serde::Serialize;

use crate::Engine;

#[derive(Clone, Debug, Default, Serialize)]
pub struct PreciseReport {
    pub indexers: Vec<String>,
    pub refs: u32,
    /// Heuristic edges confirmed by the index.
    pub upgraded: u32,
    /// Heuristic edges that pointed at the wrong symbol and were moved.
    pub corrected: u32,
    pub errors: Vec<String>,
}

impl Engine {
    /// Detect and run every installed precise indexer, then import their output.
    pub fn run_precise(self: &Arc<Self>) -> Result<PreciseReport> {
        let langs: Vec<Language> = self.summary()?.languages.iter().map(|l| l.lang).collect();
        let has = |ls: &[Language]| ls.iter().any(|l| langs.contains(l));
        let found = iso_resolve::scip::detect(
            &self.root,
            has(&[
                Language::TypeScript,
                Language::Tsx,
                Language::JavaScript,
                Language::Jsx,
            ]),
            has(&[Language::Python]),
            has(&[Language::Rust]),
        );
        let out = self
            .index
            .path()
            .parent()
            .unwrap_or(&self.root)
            .join("scip");
        let mut rep = PreciseReport::default();
        for ix in found {
            rep.indexers.push(ix.name.to_string());
            match iso_resolve::scip::run(&ix, &self.root, &out)
                .and_then(|b| iso_resolve::scip::read_index(&b))
            {
                Ok(refs) => {
                    let r = self.apply_exact_refs(&refs)?;
                    rep.refs += r.refs;
                    rep.upgraded += r.upgraded;
                    rep.corrected += r.corrected;
                }
                Err(e) => rep.errors.push(format!("{}: {e:#}", ix.name)),
            }
        }
        Ok(rep)
    }

    /// Import a SCIP file produced elsewhere (CI, or a manual run).
    pub fn import_scip(&self, bytes: &[u8]) -> Result<PreciseReport> {
        let refs = iso_resolve::scip::read_index(bytes)?;
        self.apply_exact_refs(&refs)
    }

    /// Upgrade heuristic edges that a precise index confirms, and move the ones it contradicts.
    pub fn apply_exact_refs(&self, refs: &[ExactRef]) -> Result<PreciseReport> {
        let mut rep = PreciseReport {
            refs: refs.len() as u32,
            ..Default::default()
        };
        // (file, line) → definitions referenced there.
        let mut at: HashMap<(&str, u32), Vec<&ExactRef>> = HashMap::new();
        for r in refs {
            at.entry((r.file.as_str(), r.line)).or_default().push(r);
        }
        let mut by_file: HashMap<&str, Vec<u32>> = HashMap::new();
        for (f, l) in at.keys() {
            by_file.entry(f).or_default().push(*l);
        }
        // Symbol whose declaration contains a definition line, preferring a name match.
        let mut sym_cache: HashMap<String, Vec<iso_index::SymbolRow>> = HashMap::new();
        let mut def_symbol = |path: &str, line: u32, name: &str| -> Option<SymbolId> {
            let syms = sym_cache.entry(path.to_string()).or_insert_with(|| {
                self.index
                    .read(|c| iso_index::symbols_for_file(c, FileId::of_path(path)))
                    .unwrap_or_default()
            });
            let mut best: Option<&iso_index::SymbolRow> = None;
            for s in syms
                .iter()
                .filter(|s| s.start_line <= line && line <= s.end_line)
            {
                let better = match best {
                    None => true,
                    Some(b) => (s.name == name, s.start_line) > (b.name == name, b.start_line),
                };
                if better {
                    best = Some(s);
                }
            }
            best.map(|s| s.id)
        };
        let mut updates: Vec<(SymbolId, SymbolId, SymbolId, u32)> = Vec::new();
        for file in by_file.keys() {
            let edges = self
                .index
                .read(|c| iso_index::edges_from_file(c, FileId::of_path(file)))?;
            for e in edges {
                let (Some(to), Confidence::Heuristic) = (e.to_symbol, e.confidence) else {
                    continue;
                };
                let Some(cands) = at.get(&(*file, e.line)) else {
                    continue;
                };
                let to_name = self
                    .index
                    .read(|c| iso_index::symbol_by_id(c, to))?
                    .map(|s| s.name)
                    .unwrap_or_default();
                let resolved: Vec<(SymbolId, bool)> = cands
                    .iter()
                    .filter_map(|r| {
                        def_symbol(&r.def_file, r.def_line, &r.name).map(|s| (s, r.name == to_name))
                    })
                    .collect();
                if resolved.iter().any(|(s, _)| *s == to) {
                    updates.push((e.from_symbol, to, to, e.line));
                    rep.upgraded += 1;
                    continue;
                }
                // Wrong target: take the only reference on the line, or the one with the same name.
                let right = if resolved.len() == 1 {
                    Some(resolved[0].0)
                } else {
                    resolved.iter().find(|(_, same)| *same).map(|(s, _)| *s)
                };
                if let Some(right) = right {
                    updates.push((e.from_symbol, to, right, e.line));
                    rep.corrected += 1;
                }
            }
        }
        self.index.write(|tx| {
            for (from, old, new, line) in &updates {
                iso_index::retarget_edge(tx, *from, *old, *new, *line, Confidence::Exact)?;
            }
            Ok(())
        })?;
        Ok(rep)
    }
}
