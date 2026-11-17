//! Module candidates from the directory tree plus the import graph, stored with colors.
//! The AI cluster pass and the user can rename and recolor; those names survive reclustering
//! because module ids are directory paths.

use std::collections::HashMap;

use anyhow::Result;
use iso_core::FileId;
use iso_core::ipc::{ModuleEdge, ModuleInfo, ModuleMap, ModuleSource};
use iso_graph::{ClusterOptions, Graph};
use iso_index::{FileRow, ModuleRow};

use crate::Engine;

pub(crate) fn recluster(eng: &Engine, files: &[FileRow]) -> Result<()> {
    let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
    let idx: HashMap<FileId, usize> = files.iter().enumerate().map(|(i, f)| (f.id, i)).collect();
    let edges: Vec<(usize, usize)> = eng
        .index
        .read(iso_index::file_import_edges)?
        .into_iter()
        .filter_map(|(a, b)| Some((*idx.get(&a)?, *idx.get(&b)?)))
        .collect();
    let opts = ClusterOptions {
        max_files: (files.len() / 12).clamp(40, 400),
        min_files: 3,
    };
    let cands = iso_graph::cluster(&paths, &edges, opts);
    let rows: Vec<ModuleRow> = cands
        .iter()
        .map(|m| ModuleRow {
            id: m.id.clone(),
            name: m.name.clone(),
            role: None,
            color: m.color.clone(),
            source: "cluster".into(),
            dir: m.dir.clone(),
        })
        .collect();
    let members: Vec<(FileId, String)> = cands
        .iter()
        .flat_map(|m| m.files.iter().map(move |&f| (files[f].id, m.id.clone())))
        .collect();
    eng.index
        .write(|tx| iso_index::replace_modules(tx, &rows, &members))
}

/// Put new files in the module whose directory is their nearest ancestor.
pub(crate) fn assign_by_directory(eng: &Engine, paths: &[String]) -> Result<()> {
    let mods = eng.index.read(iso_index::modules_all)?;
    if mods.is_empty() {
        return Ok(());
    }
    let pick = |p: &str| -> Option<&str> {
        mods.iter()
            .filter(|m| m.dir.is_empty() || p.starts_with(&format!("{}/", m.dir)))
            .max_by_key(|m| m.dir.len())
            .map(|m| m.id.as_str())
    };
    eng.index.write(|tx| {
        let mut st = tx.prepare_cached(
            "INSERT OR REPLACE INTO file_modules (file_id, module_id) VALUES (?1, ?2)",
        )?;
        for p in paths {
            if let Some(m) = pick(p) {
                st.execute(iso_index::rusqlite::params![FileId::of_path(p).as_i64(), m])?;
            }
        }
        Ok(())
    })
}

impl Engine {
    /// Modules and the aggregated import edges between them.
    pub fn module_map(&self) -> Result<ModuleMap> {
        let files = self.index.read(iso_index::files_all)?;
        let mods = self.index.read(iso_index::modules_all)?;
        let fm: HashMap<FileId, String> = self
            .index
            .read(iso_index::file_modules)?
            .into_iter()
            .collect();
        let edges = self.index.read(iso_index::file_import_edges)?;
        let mut stats: HashMap<&str, (u32, u64)> = HashMap::new();
        for f in &files {
            if let Some(m) = fm.get(&f.id) {
                let e = stats.entry(m.as_str()).or_default();
                e.0 += 1;
                e.1 += f.lines as u64;
            }
        }
        let mut agg: HashMap<(String, String), u32> = HashMap::new();
        for (a, b) in edges {
            if let (Some(ma), Some(mb)) = (fm.get(&a), fm.get(&b))
                && ma != mb
            {
                *agg.entry((ma.clone(), mb.clone())).or_default() += 1;
            }
        }
        let ix: HashMap<&str, usize> = mods
            .iter()
            .enumerate()
            .map(|(i, m)| (m.id.as_str(), i))
            .collect();
        let mut g = Graph::new(mods.len());
        for (a, b) in agg.keys() {
            if let (Some(&x), Some(&y)) = (ix.get(a.as_str()), ix.get(b.as_str())) {
                g.add_edge(x, y);
            }
        }
        let cycles = iso_graph::cycles(&g)
            .into_iter()
            .map(|c| c.into_iter().map(|i| mods[i].id.clone()).collect())
            .collect();
        let mut edges: Vec<ModuleEdge> = agg
            .into_iter()
            .map(|((from, to), weight)| ModuleEdge { from, to, weight })
            .collect();
        edges.sort_by(|a, b| {
            b.weight
                .cmp(&a.weight)
                .then(a.from.cmp(&b.from))
                .then(a.to.cmp(&b.to))
        });
        Ok(ModuleMap {
            modules: mods
                .iter()
                .map(|m| {
                    let (files, lines) = stats.get(m.id.as_str()).copied().unwrap_or_default();
                    ModuleInfo {
                        id: m.id.clone(),
                        name: m.name.clone(),
                        role: m.role.clone(),
                        color: m.color.clone(),
                        source: match m.source.as_str() {
                            "ai" => ModuleSource::Ai,
                            "user" => ModuleSource::User,
                            _ => ModuleSource::Cluster,
                        },
                        dir: m.dir.clone(),
                        files,
                        lines,
                    }
                })
                .collect(),
            edges,
            cycles,
        })
    }

    /// Rename or recolor a module. User edits survive reclustering and AI reruns.
    pub fn rename_module(
        &self,
        id: &str,
        name: Option<&str>,
        color: Option<&str>,
    ) -> Result<ModuleInfo> {
        self.index
            .write(|tx| iso_index::update_module(tx, id, name, None, color, "user"))?;
        self.module_map()?
            .modules
            .into_iter()
            .find(|m| m.id == id)
            .ok_or_else(|| anyhow::anyhow!("no module {id}"))
    }
}
