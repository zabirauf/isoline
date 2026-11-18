//! The AI layer as the engine sees it: inputs built from the index, cached output served to
//! file views, and module names and colors written back after a cluster pass.
//!
//! No code leaves the machine unless the user turned AI on for the repo; `estimate` and `run`
//! refuse otherwise. Serving cached output never calls a model.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Result, anyhow, bail};
use iso_ai::{AiEngine, FileInput, FullReport, ModuleInput, Progress};
use iso_core::ipc::{AiEstimate, AiScope, ArchFlow, ArchLayer, Architecture, ModuleEdge};
use iso_core::{FileFacts, FileId, FlowUnit, Tier};
use iso_index::FileRow;

use crate::Engine;
use crate::view::{AiHook, CachedAi};

/// Serves cached model output to file views.
pub struct AiLayer {
    pub ai: Arc<AiEngine>,
}

impl AiHook for AiLayer {
    fn cached(&self, row: &FileRow, facts: &FileFacts, flows: &[FlowUnit]) -> CachedAi {
        let input = FileInput {
            path: row.path.clone(),
            lang: row.lang,
            content_hash: row.content_hash.clone(),
            source: String::new(),
            facts: facts.clone(),
            flows: flows.to_vec(),
        };
        let c = self.ai.cached_file(&input);
        CachedAi {
            summaries: c.summaries,
            types: c.types,
            overview: c.overview,
        }
    }
}

/// Everything a run or an estimate needs.
pub struct AiInputs {
    pub files: Vec<FileInput>,
    pub modules: Vec<ModuleInput>,
    pub edges: Vec<ModuleEdge>,
    pub colors: HashMap<String, String>,
}

impl Engine {
    /// Build AI inputs for a scope: one file, the file's module, or the whole repo.
    pub fn ai_inputs(self: &Arc<Self>, scope: AiScope, path: Option<&str>) -> Result<AiInputs> {
        let all = self.index.read(iso_index::files_all)?;
        let fm: HashMap<FileId, String> = self
            .index
            .read(iso_index::file_modules)?
            .into_iter()
            .collect();
        let mods = self.index.read(iso_index::modules_all)?;
        let by_path: HashMap<&str, &FileRow> = all.iter().map(|f| (f.path.as_str(), f)).collect();
        let selected: Vec<&FileRow> = match scope {
            AiScope::File => {
                let p = path.ok_or_else(|| anyhow!("a file scope needs a path"))?;
                vec![*by_path.get(p).ok_or_else(|| anyhow!("not indexed: {p}"))?]
            }
            AiScope::Module => {
                let p = path.ok_or_else(|| anyhow!("a module scope needs a file path"))?;
                let row = by_path.get(p).ok_or_else(|| anyhow!("not indexed: {p}"))?;
                let m = fm.get(&row.id).cloned();
                all.iter().filter(|f| fm.get(&f.id) == m.as_ref()).collect()
            }
            AiScope::Repo | AiScope::Architecture => all.iter().collect(),
        };
        // The architecture run reads only cached summaries and symbol names: no source, no
        // call flows, which keeps it fast on large repos.
        let light = scope == AiScope::Architecture;
        let mut files = Vec::new();
        for row in selected.into_iter().filter(|f| f.tier != Tier::Text) {
            let facts = self.facts_for(row)?;
            if facts.symbols.is_empty() {
                continue;
            }
            let (source, flows) = if light {
                (String::new(), Vec::new())
            } else {
                let source = std::fs::read_to_string(self.root.join(&row.path)).unwrap_or_default();
                let flows = self.flow_units_for(row, &source).unwrap_or_default();
                (source, flows)
            };
            files.push(FileInput {
                path: row.path.clone(),
                lang: row.lang,
                content_hash: row.content_hash.clone(),
                source,
                facts,
                flows,
            });
        }
        let path_of: HashMap<FileId, &str> = all.iter().map(|f| (f.id, f.path.as_str())).collect();
        let wanted: Option<std::collections::HashSet<String>> = match scope {
            AiScope::Repo | AiScope::Architecture => None,
            _ => Some(
                files
                    .iter()
                    .filter_map(|f| fm.get(&FileId::of_path(&f.path)).cloned())
                    .collect(),
            ),
        };
        let modules: Vec<ModuleInput> = mods
            .iter()
            .filter(|m| wanted.as_ref().is_none_or(|w| w.contains(&m.id)))
            .map(|m| ModuleInput {
                id: m.id.clone(),
                dir: m.dir.clone(),
                files: fm
                    .iter()
                    .filter(|(_, mid)| *mid == &m.id)
                    .filter_map(|(f, _)| path_of.get(f).map(|p| p.to_string()))
                    .collect(),
            })
            .collect();
        let colors = mods
            .iter()
            .filter(|m| m.source != "cluster")
            .map(|m| (m.id.clone(), m.color.clone()))
            .collect();
        let edges = self.module_map()?.edges;
        Ok(AiInputs {
            files,
            modules,
            edges,
            colors,
        })
    }

    fn require_ai(&self) -> Result<()> {
        if !self.ai_enabled() {
            bail!(
                "AI analysis is off for this repo. Turn it on first; no code leaves the machine until then."
            );
        }
        Ok(())
    }

    pub fn ai_estimate(
        self: &Arc<Self>,
        ai: &AiEngine,
        scope: AiScope,
        path: Option<&str>,
    ) -> Result<AiEstimate> {
        self.require_ai()?;
        let i = self.ai_inputs(scope, path)?;
        Ok(ai.estimate(&i.files, &i.modules, &i.edges, scope).to_ipc())
    }

    /// Run the passes for a scope within a budget, then store module names, roles and colors.
    /// A module the user renamed keeps the user's name and color.
    pub fn ai_run(
        self: &Arc<Self>,
        ai: &AiEngine,
        scope: AiScope,
        path: Option<&str>,
        budget_usd: f64,
        on_progress: &mut dyn FnMut(&Progress),
    ) -> Result<FullReport> {
        self.require_ai()?;
        let i = self.ai_inputs(scope, path)?;
        let report = ai
            .run(
                &i.files,
                &i.modules,
                &i.edges,
                &i.colors,
                scope,
                budget_usd,
                on_progress,
            )
            .map_err(|e| anyhow!("{e}"))?;
        if let Some(c) = &report.cluster {
            let existing: HashMap<String, String> = self
                .index
                .read(iso_index::modules_all)?
                .into_iter()
                .map(|m| (m.id, m.source))
                .collect();
            self.index.write(|tx| {
                for m in c.modules.iter().filter(|m| m.ai) {
                    if existing.get(&m.id).map(String::as_str) == Some("user") {
                        continue;
                    }
                    iso_index::update_module(
                        tx,
                        &m.id,
                        Some(&m.name),
                        Some(&m.role),
                        Some(&m.color),
                        "ai",
                    )?;
                }
                Ok(())
            })?;
        }
        if let Some(o) = report.repo.as_ref().and_then(|r| r.overview.as_ref()) {
            let a = Architecture {
                overview: o.overview.clone(),
                layers: o
                    .layers
                    .iter()
                    .map(|l| ArchLayer {
                        name: l.name.clone(),
                        modules: l.modules.clone(),
                    })
                    .collect(),
                flows: o
                    .flows
                    .iter()
                    .map(|f| ArchFlow {
                        name: f.name.clone(),
                        text: f.text.clone(),
                        modules: f.modules.clone(),
                    })
                    .collect(),
                model: o.model.clone(),
                modules: o.legend.len() as u32,
                written_at: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs()),
            };
            let json = serde_json::to_string(&a)?;
            self.index
                .write(|tx| iso_index::meta_set(tx, ARCHITECTURE_KEY, &json))?;
        }
        Ok(report)
    }

    /// The last architecture overview written for this repo, if any. Reading it never calls
    /// a model.
    pub fn architecture(&self) -> Result<Option<Architecture>> {
        let raw = self
            .index
            .read(|c| iso_index::meta_get(c, ARCHITECTURE_KEY))?;
        Ok(raw.and_then(|s| serde_json::from_str(&s).ok()))
    }
}

/// Index meta key of the stored architecture overview (JSON).
const ARCHITECTURE_KEY: &str = "ai_architecture";
