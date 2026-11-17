//! Read-side operations: file view, call flow, source, tree, search, impact.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use anyhow::{Context, Result, anyhow};
use iso_core::cache::DerivedKind;
use iso_core::ipc::*;
use iso_core::{
    FLOW_WALKER_VERSION, FileFacts, FileId, FlowUnit, ImportKind, SymbolFact, SymbolId, SymbolKind,
    Tier,
};
use iso_graph::Graph;
use iso_index::FileRow;

use crate::Engine;
use crate::edges::{Binding, bindings};

/// Cards are callables at the top level or directly inside a type (methods).
fn is_card(s: &SymbolFact, facts: &FileFacts) -> bool {
    if !s.kind.is_callable() {
        return false;
    }
    match &s.parent {
        None => true,
        Some(p) => facts.symbols.iter().any(|t| {
            &t.qualified_name == p
                && (t.kind.is_type_like()
                    || t.kind == SymbolKind::Impl
                    || t.kind == SymbolKind::Module)
        }),
    }
}

/// Nearest enclosing card for a qualified name (nested helpers count toward their card).
fn card_of<'a>(q: &str, cards: &'a HashMap<String, usize>) -> Option<(&'a str, usize)> {
    let mut cur = q;
    loop {
        if let Some((k, v)) = cards.get_key_value(cur) {
            return Some((k.as_str(), *v));
        }
        cur = cur.rsplit_once('.')?.0;
    }
}

fn doc_summary(doc: &Option<String>) -> Option<Summary> {
    let d = doc.as_ref()?.trim();
    if d.is_empty() {
        return None;
    }
    // First sentence or first line, whichever is shorter.
    let first_line = d.lines().next().unwrap_or(d);
    let sentence = match d.find(". ") {
        Some(i) if i < first_line.len() + 1 => &d[..=i],
        _ => first_line,
    };
    Some(Summary {
        desc: sentence.trim().to_string(),
        algo: Vec::new(),
        notes: Vec::new(),
        model: "doc comment".into(),
        ai: false,
    })
}

/// End line of the import block that starts or ends at `last` (1-based), scanning forward
/// across multi-line import statements.
fn import_block_end(lines: &[&str], last: u32) -> u32 {
    let mut end = last;
    let start = last as usize;
    for i in start..(start + 80).min(lines.len() + 1) {
        let l = lines[i - 1].trim_end();
        end = i as u32;
        let open = l.ends_with('{') || l.ends_with(',') || l.ends_with('(') || l.ends_with('\\');
        if l.contains(';')
            || (!open && i > start)
            || (!open
                && (l.contains(" from ")
                    || l.starts_with("import ")
                    || l.starts_with("use ")
                    || l.starts_with("from ")))
        {
            break;
        }
    }
    end
}

impl Engine {
    /// Index row for a path, reindexing it first if it changed on disk (or was never indexed).
    pub fn fresh_row(self: &std::sync::Arc<Self>, path: &str) -> Result<FileRow> {
        let row = self.index.read(|c| iso_index::file_by_path(c, path))?;
        let md = std::fs::metadata(self.root.join(path))
            .with_context(|| format!("no such file: {path}"))?;
        let mtime = md
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos() as i64)
            .unwrap_or(0);
        let stale = match &row {
            Some(r) => r.status == "pending" || r.size != md.len() as i64 || r.mtime != mtime,
            None => true,
        };
        if stale {
            self.queue.bump(
                iso_core::jobs::JobKind::Parse,
                path,
                iso_core::jobs::Priority::VisibleFile,
            );
            self.index_paths(&[path.to_string()])?;
        }
        self.index
            .read(|c| iso_index::file_by_path(c, path))?
            .ok_or_else(|| anyhow!("not indexed: {path}"))
    }

    /// Everything the reading list and the dependency map need for one file.
    pub fn file_view(self: &std::sync::Arc<Self>, path: &str) -> Result<FileView> {
        let row = self.fresh_row(path)?;
        let facts = self.facts_for(&row)?;
        let source = if row.tier == Tier::Text {
            String::new()
        } else {
            std::fs::read_to_string(self.root.join(path)).unwrap_or_default()
        };
        let src_lines: Vec<&str> = source.lines().collect();
        let imports = self
            .index
            .read(|c| iso_index::imports_for_file(c, row.id))?;
        let binds = bindings(&imports);
        let importer_rows = self.index.read(|c| iso_index::importers_of(c, row.id))?;
        let all_files: HashMap<FileId, String> = self.index.read(|c| {
            iso_index::file_paths(
                c,
                imports
                    .iter()
                    .filter_map(|i| i.resolved)
                    .chain(importer_rows.iter().map(|(f, _)| *f)),
            )
        })?;

        // ---- cards
        let card_syms: Vec<&SymbolFact> = facts
            .symbols
            .iter()
            .filter(|s| is_card(s, &facts))
            .collect();
        let card_ix: HashMap<String, usize> = card_syms
            .iter()
            .enumerate()
            .map(|(i, s)| (s.qualified_name.clone(), i))
            .collect();
        let top_by_name: HashMap<&str, usize> = card_syms
            .iter()
            .enumerate()
            .filter(|(_, s)| s.parent.is_none())
            .map(|(i, s)| (s.name.as_str(), i))
            .collect();
        let types: HashSet<&str> = facts
            .symbols
            .iter()
            .filter(|s| s.kind.is_type_like() || s.kind == SymbolKind::Impl)
            .map(|s| s.name.as_str())
            .collect();
        let n = card_syms.len();
        let mut calls: Vec<BTreeMap<usize, u32>> = vec![BTreeMap::new(); n];
        let mut uses: Vec<Vec<String>> = vec![Vec::new(); n];
        let mut type_uses: Vec<Vec<String>> = vec![Vec::new(); n];
        let import_local: HashSet<&str> = binds.keys().map(|s| s.as_str()).collect();
        let type_only_local: HashSet<String> = imports
            .iter()
            .flat_map(|i| {
                i.names
                    .iter()
                    .filter(move |nm| nm.type_only || i.type_only)
                    .map(|nm| nm.local().to_string())
            })
            .collect();
        let mut param_members: Vec<((String, String), String)> = Vec::new();
        let add_use = |v: &mut Vec<String>, name: &str| {
            if !v.iter().any(|x| x == name) {
                v.push(name.to_string());
            }
        };
        for c in &facts.calls {
            let Some(q) = &c.caller else { continue };
            let Some((_, from)) = card_of(q, &card_ix) else {
                continue;
            };
            let target = match c.receiver.as_deref() {
                None => top_by_name.get(c.name.as_str()).copied(),
                Some("this" | "self" | "Self" | "cls") => card_syms[from]
                    .parent
                    .as_ref()
                    .and_then(|p| card_ix.get(&format!("{p}.{}", c.name)).copied()),
                Some(r) if types.contains(r) => card_ix.get(&format!("{r}.{}", c.name)).copied(),
                _ => None,
            };
            if let Some(t) = target {
                if t != from {
                    calls[from].entry(t).or_insert(c.line);
                }
                continue;
            }
            let head = c
                .receiver
                .as_deref()
                .map(|r| r.split(['.', ':']).next().unwrap_or(r))
                .unwrap_or(c.name.as_str());
            if import_local.contains(head) && !type_only_local.contains(head) {
                add_use(&mut uses[from], head);
            }
            // `harness.runAttempt()` where the card declares `harness: AgentHarness` and
            // `AgentHarness` is imported: a member reached through a typed parameter.
            if let Some(r) = c.receiver.as_deref()
                && is_ident(r)
                && !matches!(r, "this" | "self" | "Self" | "cls")
                && let Some(ty) = param_type(&card_syms[from].signature, r)
                && binds.contains_key(ty.as_str())
            {
                add_use(&mut uses[from], &c.name);
                let key = (r.to_string(), ty);
                if !param_members.iter().any(|(k, m)| k == &key && m == &c.name) {
                    param_members.push((key, c.name.clone()));
                }
            }
        }
        for r in &facts.refs {
            let Some(q) = &r.from else { continue };
            let Some((_, from)) = card_of(q, &card_ix) else {
                continue;
            };
            if !import_local.contains(r.name.as_str()) {
                continue;
            }
            if r.type_position || type_only_local.contains(&r.name) {
                add_use(&mut type_uses[from], &r.name);
            } else {
                add_use(&mut uses[from], &r.name);
            }
        }
        for i in 0..n {
            let u = uses[i].clone();
            type_uses[i].retain(|t| !u.contains(t));
        }
        let mut g = Graph::new(n);
        for (a, outs) in calls.iter().enumerate() {
            for &b in outs.keys() {
                g.add_edge(a, b);
            }
        }
        let levels = iso_graph::layers(&g);
        let mut called_by: Vec<Vec<usize>> = vec![Vec::new(); n];
        for (a, outs) in calls.iter().enumerate() {
            for &b in outs.keys() {
                called_by[b].push(a);
            }
        }
        let flow_units = self.flow_units_for(&row, &source)?;
        let unit_ids: HashSet<&str> = flow_units.iter().map(|u| u.id.as_str()).collect();
        let cached = self.cached_ai(&row, &facts, &flow_units);
        let summaries = &cached.summaries;
        let mut functions = Vec::with_capacity(n);
        for (i, s) in card_syms.iter().enumerate() {
            let id = SymbolId::new(&row.path, &s.qualified_name, s.kind);
            let external_callers = self
                .index
                .read(|c| iso_index::external_caller_count(c, id, row.id))
                .unwrap_or(0);
            let name_of = |j: &usize| card_syms[*j].qualified_name.clone();
            functions.push(FnCard {
                id,
                name: s.qualified_name.clone(),
                qualified_name: s.qualified_name.clone(),
                kind: s.kind,
                start: s.start_line,
                end: s.end_line,
                exported: s.exported,
                is_async: s.is_async,
                signature: s.signature.clone(),
                doc: s.doc.clone(),
                level: levels[i],
                calls: calls[i].keys().map(name_of).collect(),
                called_by: called_by[i].iter().map(name_of).collect(),
                uses: uses[i].clone(),
                type_uses: type_uses[i].clone(),
                external_callers,
                summary: summaries
                    .get(&s.qualified_name)
                    .cloned()
                    .or_else(|| doc_summary(&s.doc)),
                unit: if unit_ids.contains(s.qualified_name.as_str()) {
                    Some(s.qualified_name.clone())
                } else {
                    None
                },
            });
        }

        // ---- types, outline
        let types_out: Vec<TypeDecl> = facts
            .symbols
            .iter()
            .filter(|s| s.kind.is_type_like() && s.parent.is_none())
            .map(|s| TypeDecl {
                id: SymbolId::new(&row.path, &s.qualified_name, s.kind),
                name: s.name.clone(),
                kind: s.kind,
                start: s.start_line,
                end: s.end_line,
                exported: s.exported,
                doc: s.doc.clone(),
                summary: cached.types.get(&s.qualified_name).cloned(),
            })
            .collect();
        let depth_of = |s: &SymbolFact| -> u32 {
            let mut d = 0;
            let mut p = s.parent.clone();
            while let Some(q) = p {
                d += 1;
                p = facts
                    .symbols
                    .iter()
                    .find(|t| t.qualified_name == q)
                    .and_then(|t| t.parent.clone());
                if d > 8 {
                    break;
                }
            }
            d
        };
        let outline: Vec<OutlineItem> = facts
            .symbols
            .iter()
            .map(|s| OutlineItem {
                name: s.name.clone(),
                kind: s.kind,
                start: s.start_line,
                end: s.end_line,
                depth: depth_of(s),
            })
            .collect();

        // ---- imported modules
        let mut modules: Vec<ImportModule> = Vec::new();
        for i in &imports {
            if i.kind == ImportKind::Mod && i.resolved.is_none() {
                continue;
            }
            let resolved = i.resolved.and_then(|f| all_files.get(&f).cloned());
            let entry = match modules.iter_mut().position(|m| m.id == i.specifier) {
                Some(p) => &mut modules[p],
                None => {
                    modules.push(ImportModule {
                        id: i.specifier.clone(),
                        kind: i.kind,
                        resolved: resolved.clone(),
                        external: i.external.is_some(),
                        symbols: Vec::new(),
                        line: i.line,
                    });
                    modules.last_mut().unwrap()
                }
            };
            for nm in &i.names {
                let local = nm.local().to_string();
                if !entry.symbols.iter().any(|s| s.name == local) {
                    entry.symbols.push(ImportSym {
                        name: local,
                        type_only: nm.type_only || i.type_only,
                        member: false,
                    });
                }
            }
        }
        // Member symbols reached through namespace bindings (`ns.fn()`), shown on their module.
        for c in &facts.calls {
            let Some(r) = c.receiver.as_deref() else {
                continue;
            };
            if let Some(Binding::File(_, n) | Binding::External(_, n)) = binds.get(r)
                && n == "*"
            {
                for i in &imports {
                    let last = i
                        .specifier
                        .rsplit(['.', ':', '/'])
                        .find(|s| !s.is_empty())
                        .unwrap_or(&i.specifier);
                    let is_ns = i.names.iter().any(|nm| nm.name == "*" && nm.local() == r)
                        || (i.names.is_empty() && last == r);
                    if is_ns
                        && let Some(m) = modules.iter_mut().find(|m| m.id == i.specifier)
                        && !m.symbols.iter().any(|s| s.name == c.name)
                    {
                        m.symbols.push(ImportSym {
                            name: c.name.clone(),
                            type_only: false,
                            member: true,
                        });
                    }
                }
            }
        }
        // One module per typed parameter whose members are called, like the prototype's
        // "harness (AgentHarness param)".
        for ((param, ty), member) in &param_members {
            let id = format!("{param} ({ty} param)");
            let src = imports
                .iter()
                .find(|i| i.names.iter().any(|n| n.local() == ty));
            if !modules.iter().any(|m| m.id == id) {
                modules.push(ImportModule {
                    id: id.clone(),
                    kind: ImportKind::Static,
                    resolved: src
                        .and_then(|i| i.resolved)
                        .and_then(|f| all_files.get(&f).cloned()),
                    external: src.is_some_and(|i| i.external.is_some()),
                    symbols: Vec::new(),
                    line: src.map(|i| i.line).unwrap_or(0),
                });
            }
            let m = modules.iter_mut().find(|m| m.id == id).expect("just added");
            if !m.symbols.iter().any(|s| &s.name == member) {
                m.symbols.push(ImportSym {
                    name: member.clone(),
                    type_only: false,
                    member: true,
                });
            }
        }
        let imports_range = {
            let static_lines: Vec<u32> = imports
                .iter()
                .filter(|i| {
                    matches!(
                        i.kind,
                        ImportKind::Static | ImportKind::Reexport | ImportKind::Mod
                    )
                })
                .map(|i| i.line)
                .collect();
            let first_sym = facts
                .symbols
                .iter()
                .map(|s| s.start_line)
                .min()
                .unwrap_or(u32::MAX);
            match (
                static_lines.iter().min(),
                static_lines.iter().filter(|&&l| l < first_sym).max(),
            ) {
                (Some(&a), Some(&b)) if a < first_sym => Some(LineRange {
                    start: a,
                    end: import_block_end(&src_lines, b),
                }),
                _ => None,
            }
        };
        let importers = importer_rows
            .into_iter()
            .filter_map(|(f, line)| {
                all_files.get(&f).map(|p| FileRef {
                    path: p.clone(),
                    line: Some(line),
                })
            })
            .collect();
        let module = self.index.read(|c| {
            let m: Option<String> = iso_index::rusqlite::OptionalExtension::optional(c.query_row(
                "SELECT module_id FROM file_modules WHERE file_id = ?1",
                [row.id.as_i64()],
                |r| r.get(0),
            ))?;
            Ok(match m {
                Some(m) => iso_index::modules_all(c)?
                    .into_iter()
                    .find(|x| x.id == m)
                    .map(|x| ModuleRef {
                        id: x.id,
                        name: x.name,
                        color: x.color,
                    }),
                None => None,
            })
        })?;
        let name = row.path.rsplit('/').next().unwrap_or(&row.path).to_string();
        let flow_unit_refs: Vec<FlowUnitRef> = flow_units
            .iter()
            .map(|u| FlowUnitRef {
                id: u.id.clone(),
                owner: u.owner.clone(),
                method: u.method.clone(),
                start: u.start,
                end: u.end,
                summary: summaries.get(&u.id).map(|x| x.desc.clone()).or_else(|| {
                    functions
                        .iter()
                        .find(|f| f.qualified_name == u.id)
                        .and_then(|f| f.summary.as_ref().map(|x| x.desc.clone()))
                }),
            })
            .collect();
        Ok(FileView {
            path: row.path.clone(),
            name,
            lang: row.lang,
            lang_label: row.lang.label().into(),
            monaco_lang: row.lang.monaco_id().into(),
            tier: row.tier,
            lines: row.lines,
            content_hash: row.content_hash.clone(),
            layers: if n == 0 {
                0
            } else {
                levels.iter().max().copied().unwrap_or(0) + 1
            },
            functions,
            types: types_out,
            modules,
            imports_range,
            outline,
            flow_units: flow_unit_refs,
            importers,
            overview: cached.overview.clone(),
            has_errors: row.has_errors,
            module,
        })
    }

    /// Call-flow units for a file, cached by content hash and walker version.
    pub(crate) fn flow_units_for(&self, row: &FileRow, source: &str) -> Result<Vec<FlowUnit>> {
        if row.tier != Tier::Full {
            return Ok(Vec::new());
        }
        if let Some(v) = self.index.read(|c| {
            self.derived.get::<Vec<FlowUnit>>(
                c,
                DerivedKind::Flow,
                &row.content_hash,
                FLOW_WALKER_VERSION,
            )
        })? {
            return Ok(v);
        }
        let path = std::path::Path::new(&row.path);
        let units = std::panic::catch_unwind(|| iso_parse::flow_units(path, row.lang, source))
            .unwrap_or_default();
        self.index.write(|tx| {
            self.derived.put(
                tx,
                DerivedKind::Flow,
                &row.content_hash,
                FLOW_WALKER_VERSION,
                &units,
            )
        })?;
        Ok(units)
    }

    /// One flow unit (`fn`, `Class.method`, `factory.method`).
    pub fn flow(self: &std::sync::Arc<Self>, path: &str, unit: &str) -> Result<FlowUnit> {
        let row = self.fresh_row(path)?;
        let source = std::fs::read_to_string(self.root.join(path))?;
        self.flow_units_for(&row, &source)?
            .into_iter()
            .find(|u| u.id == unit)
            .ok_or_else(|| anyhow!("no flow unit {unit} in {path}"))
    }

    /// All flow units of a file (CLI and eval).
    pub fn flows(self: &std::sync::Arc<Self>, path: &str) -> Result<Vec<FlowUnit>> {
        let row = self.fresh_row(path)?;
        let source = std::fs::read_to_string(self.root.join(path))?;
        self.flow_units_for(&row, &source)
    }

    /// Source text for a line range (1-based, inclusive). Whole file when no range is given.
    pub fn source(&self, path: &str, start: Option<u32>, end: Option<u32>) -> Result<SourceSlice> {
        let text = std::fs::read_to_string(self.root.join(path))
            .with_context(|| format!("read {path}"))?;
        let lines: Vec<&str> = text.lines().collect();
        let s = start.unwrap_or(1).max(1);
        let e = end
            .unwrap_or(lines.len() as u32)
            .min(lines.len() as u32)
            .max(s.saturating_sub(1));
        let slice = if lines.is_empty() || s as usize > lines.len() {
            String::new()
        } else {
            lines[(s - 1) as usize..e as usize].join("\n")
        };
        let lang = iso_core::Language::detect(path, lines.first().copied());
        Ok(SourceSlice {
            path: path.to_string(),
            start: s,
            end: e,
            text: slice,
            monaco_lang: lang.monaco_id().into(),
        })
    }

    /// Directory tree with module and color per file.
    pub fn tree(&self) -> Result<TreeNode> {
        let files = self.index.read(iso_index::files_all)?;
        let fm: HashMap<FileId, String> = self
            .index
            .read(iso_index::file_modules)?
            .into_iter()
            .collect();
        let colors: HashMap<String, String> = self
            .index
            .read(iso_index::modules_all)?
            .into_iter()
            .map(|m| (m.id, m.color))
            .collect();
        let mut root = TreeNode {
            name: self.name.clone(),
            path: String::new(),
            is_dir: true,
            lang: None,
            tier: None,
            module: None,
            color: None,
            status: FileStatus::Indexed,
            lines: 0,
            children: Vec::new(),
        };
        // Build with an index of directory nodes by path to stay linear.
        fn insert(node: &mut TreeNode, parts: &[&str], prefix: &str, leaf: TreeNode) {
            if parts.len() == 1 {
                node.lines += leaf.lines;
                node.children.push(leaf);
                return;
            }
            let dir_path = if prefix.is_empty() {
                parts[0].to_string()
            } else {
                format!("{prefix}/{}", parts[0])
            };
            let pos = match node
                .children
                .iter()
                .rposition(|c| c.is_dir && c.name == parts[0])
            {
                Some(p) => p,
                None => {
                    node.children.push(TreeNode {
                        name: parts[0].to_string(),
                        path: dir_path.clone(),
                        is_dir: true,
                        lang: None,
                        tier: None,
                        module: None,
                        color: None,
                        status: FileStatus::Indexed,
                        lines: 0,
                        children: Vec::new(),
                    });
                    node.children.len() - 1
                }
            };
            node.lines += leaf.lines;
            if leaf.status == FileStatus::Pending {
                node.children[pos].status = FileStatus::Pending;
            }
            insert(&mut node.children[pos], &parts[1..], &dir_path, leaf);
        }
        for f in &files {
            let parts: Vec<&str> = f.path.split('/').collect();
            let module = fm.get(&f.id).cloned();
            let leaf = TreeNode {
                name: parts.last().unwrap_or(&"").to_string(),
                path: f.path.clone(),
                is_dir: false,
                lang: Some(f.lang),
                tier: Some(f.tier),
                color: module.as_ref().and_then(|m| colors.get(m).cloned()),
                module,
                status: match f.status.as_str() {
                    "pending" => FileStatus::Pending,
                    "failed" => FileStatus::Failed,
                    "skipped" => FileStatus::Skipped,
                    _ => FileStatus::Indexed,
                },
                lines: f.lines,
                children: Vec::new(),
            };
            if leaf.status == FileStatus::Pending {
                root.status = FileStatus::Pending;
            }
            insert(&mut root, &parts, "", leaf);
        }
        fn sort(n: &mut TreeNode) {
            n.children.sort_by(|a, b| {
                b.is_dir
                    .cmp(&a.is_dir)
                    .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            });
            // A directory's color is its children's when they all agree.
            for c in n.children.iter_mut() {
                sort(c);
            }
            if n.is_dir {
                let mods: HashSet<&Option<String>> = n.children.iter().map(|c| &c.module).collect();
                if mods.len() == 1 {
                    n.module = n.children.first().and_then(|c| c.module.clone());
                    n.color = n.children.first().and_then(|c| c.color.clone());
                }
            }
        }
        sort(&mut root);
        Ok(root)
    }

    /// Symbols and files matching a query.
    pub fn search(&self, q: &str, limit: usize) -> Result<Vec<SearchHit>> {
        let q = q.trim();
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let mut hits: Vec<SearchHit> = self
            .index
            .read(|c| iso_index::search_symbols(c, q, limit))?
            .into_iter()
            .map(|(s, path)| {
                let score = if s.name.eq_ignore_ascii_case(q) {
                    3.0
                } else if s.name.to_lowercase().starts_with(&q.to_lowercase()) {
                    2.0
                } else {
                    1.0
                } + if s.exported { 0.25 } else { 0.0 };
                SearchHit {
                    kind: SearchKind::Symbol,
                    name: s.qualified_name,
                    path,
                    line: Some(s.start_line),
                    symbol_kind: Some(s.kind),
                    score,
                }
            })
            .collect();
        let ql = q.to_lowercase();
        let files = self.index.read(|c| iso_index::search_paths(c, q, limit))?;
        let mut fh: Vec<SearchHit> = files
            .iter()
            .filter_map(|path| {
                let pl = path.to_lowercase();
                let name = pl.rsplit('/').next().unwrap_or(&pl);
                let score = if name == ql {
                    3.5
                } else if name.starts_with(&ql) {
                    2.5
                } else if name.contains(&ql) {
                    1.5
                } else if pl.contains(&ql) {
                    0.8
                } else {
                    return None;
                };
                Some(SearchHit {
                    kind: SearchKind::File,
                    name: path.rsplit('/').next().unwrap_or(path).to_string(),
                    path: path.clone(),
                    line: None,
                    symbol_kind: None,
                    score,
                })
            })
            .collect();
        fh.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then(a.path.len().cmp(&b.path.len()))
        });
        fh.truncate(limit);
        hits.extend(fh);
        hits.sort_by(|a, b| b.score.total_cmp(&a.score));
        hits.truncate(limit);
        Ok(hits)
    }

    /// Dependents and dependencies of a symbol, direct or transitive.
    pub fn impact(&self, sym: SymbolId, transitive: bool) -> Result<Impact> {
        let paths_cache: parking_lot::Mutex<HashMap<FileId, String>> =
            parking_lot::Mutex::new(HashMap::new());
        let path_of = |f: FileId| -> String {
            if let Some(p) = paths_cache.lock().get(&f) {
                return p.clone();
            }
            let p = self
                .index
                .read(|c| iso_index::file_paths(c, [f]))
                .ok()
                .and_then(|m| m.get(&f).cloned())
                .unwrap_or_default();
            paths_cache.lock().insert(f, p.clone());
            p
        };
        let me = self
            .index
            .read(|c| iso_index::symbol_by_id(c, sym))?
            .ok_or_else(|| anyhow!("unknown symbol {sym}"))?;
        let to_ref = |id: SymbolId, conf: iso_core::Confidence, depth: u32| -> Option<SymRef> {
            let s = self
                .index
                .read(|c| iso_index::symbol_by_id(c, id))
                .ok()
                .flatten()?;
            Some(SymRef {
                id,
                name: s.qualified_name,
                path: path_of(s.file_id),
                line: s.start_line,
                confidence: conf,
                depth,
            })
        };
        let max_depth = if transitive { 12 } else { 1 };
        let walk = |forward: bool| -> Result<Vec<SymRef>> {
            let mut seen: HashSet<SymbolId> = HashSet::from([sym]);
            let mut out = Vec::new();
            let mut q = VecDeque::from([(sym, 0u32)]);
            while let Some((s, d)) = q.pop_front() {
                if d >= max_depth || out.len() >= 400 {
                    continue;
                }
                let edges = self.index.read(|c| {
                    if forward {
                        iso_index::edges_from(c, s)
                    } else {
                        iso_index::edges_to(c, s)
                    }
                })?;
                for e in edges {
                    let next = if forward {
                        e.to_symbol
                    } else {
                        Some(e.from_symbol)
                    };
                    let Some(nx) = next else { continue };
                    if seen.insert(nx) {
                        if let Some(r) = to_ref(nx, e.confidence, d + 1) {
                            out.push(r);
                        }
                        q.push_back((nx, d + 1));
                    }
                }
            }
            Ok(out)
        };
        let dependents = walk(false)?;
        let dependencies = walk(true)?;
        let files_affected = dependents
            .iter()
            .map(|r| r.path.as_str())
            .collect::<HashSet<_>>()
            .len() as u32;
        Ok(Impact {
            symbol: SymRef {
                id: sym,
                name: me.qualified_name,
                path: path_of(me.file_id),
                line: me.start_line,
                confidence: iso_core::Confidence::Exact,
                depth: 0,
            },
            dependents,
            dependencies,
            files_affected,
        })
    }

    /// Cached model output for a file. Reads the AI cache only; never calls a model.
    pub(crate) fn cached_ai(
        &self,
        row: &FileRow,
        facts: &FileFacts,
        flows: &[FlowUnit],
    ) -> CachedAi {
        match self.ai_hook.read().as_ref() {
            Some(h) => h.cached(row, facts, flows),
            None => CachedAi::default(),
        }
    }
}

/// Cached model output for one file.
#[derive(Clone, Debug, Default)]
pub struct CachedAi {
    /// By qualified name (functions, methods, flow units).
    pub summaries: HashMap<String, Summary>,
    /// One sentence per type, by qualified name.
    pub types: HashMap<String, String>,
    pub overview: Option<AiText>,
}

fn is_ident(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '$')
}

/// Declared type of parameter `name` in a signature: `name: Type`, `name: &mut Type<T>`,
/// `name?: Type`. Returns the bare type name.
fn param_type(signature: &str, name: &str) -> Option<String> {
    let mut from = 0;
    while let Some(i) = signature[from..].find(name) {
        let at = from + i;
        from = at + name.len();
        let before_ok = at == 0
            || !signature[..at]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '$' || c == '.');
        let rest = &signature[at + name.len()..];
        let rest = rest.strip_prefix('?').unwrap_or(rest).trim_start();
        let Some(rest) = rest.strip_prefix(':').filter(|r| !r.starts_with(':')) else {
            continue;
        };
        if !before_ok {
            continue;
        }
        let rest = rest.trim_start().trim_start_matches('&').trim_start();
        let rest = rest.strip_prefix("mut ").unwrap_or(rest).trim_start();
        let ty: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '$')
            .collect();
        if !ty.is_empty() {
            return Some(ty);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::param_type;

    #[test]
    fn parameter_types_from_signatures() {
        let sig = "export async function run(harness: AgentHarness, params: AgentHarnessAttemptParamsV2, execute = …)";
        assert_eq!(param_type(sig, "harness").as_deref(), Some("AgentHarness"));
        assert_eq!(
            param_type(sig, "params").as_deref(),
            Some("AgentHarnessAttemptParamsV2")
        );
        assert_eq!(param_type(sig, "execute"), None);
        assert_eq!(
            param_type("fn f(store: &mut Store<T>)", "store").as_deref(),
            Some("Store")
        );
        assert_eq!(
            param_type("def f(self, repo: Repo)", "repo").as_deref(),
            Some("Repo")
        );
        assert_eq!(param_type("fn f(a: x::Y)", "a").as_deref(), Some("x"));
        assert_eq!(
            param_type("f(opts?: Options)", "opts").as_deref(),
            Some("Options")
        );
    }
}

/// Supplies cached model output to file views. Implemented by the AI layer.
pub trait AiHook: Send + Sync {
    fn cached(&self, row: &FileRow, facts: &FileFacts, flows: &[FlowUnit]) -> CachedAi;
}
