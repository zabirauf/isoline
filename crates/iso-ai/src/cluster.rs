//! Cluster pass (module names, roles, colors) and repo pass (architecture overview).
//!
//! A module's answer is keyed by the hash of exactly what the model sees: its directory,
//! its members' cached summaries and its edges. So a module reruns only when a member's
//! summary or an edge changed, and the repo pass only when a module summary changed.

use std::collections::{HashMap, HashSet};

use iso_core::hash64;
use iso_core::ipc::ModuleEdge;
use serde::{Deserialize, Serialize};

use crate::cache::kind;
use crate::engine::{
    AiEngine, ClusterReport, FileInput, Meter, ModuleInput, ModuleSummary, Pass, PassEstimate,
    Planned, Progress, RepoLayer, RepoOverview, RepoReport, RunReport, sum_requests,
};
use crate::palette::{assign_colors, dark_variant};
use crate::prompts::{
    OUT_TOKENS_ENVELOPE, OUT_TOKENS_MODULE, OUT_TOKENS_REPO, OUT_TOKENS_REPO_PER_MODULE,
    cluster_system, repo_system,
};
use crate::skeleton::clip;
use crate::tokens::estimate_tokens;
use crate::{AiError, CompletionRequest, parse};

const EPS: f64 = 1e-12;

/// Stand-ins for summaries that do not exist yet, at a typical length, for estimates.
const PLACEHOLDER_DESC: &str =
    "Placeholder for a one-sentence summary of typical length, used only to estimate tokens.";
const PLACEHOLDER_OVERVIEW: &str = "Placeholder for a file overview of typical length, used only to estimate tokens. \
It stands for two or three sentences about the file and its entry points.";
const PLACEHOLDER_ROLE: &str =
    "Placeholder for a module role sentence, used only to estimate tokens.";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Use cached member summaries; leave missing ones out.
    Cached,
    /// Count missing member summaries at a typical size.
    Estimate,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct ClusterAnswer {
    name: String,
    role: String,
    responsibilities: Vec<String>,
    model: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct RepoAnswer {
    overview: String,
    layers: Vec<RepoLayer>,
    model: String,
}

fn title_from_dir(dir: &str, id: &str) -> String {
    let base = dir.trim_end_matches('/');
    let seg = base
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(id);
    let words = seg.replace(['_', '-'], " ");
    let mut c = words.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => "Root".to_string(),
    }
}

struct ModuleItem {
    index: usize,
    content: String,
    request: CompletionRequest,
}

impl AiEngine {
    fn module_block(
        &self,
        m: &ModuleInput,
        by_path: &HashMap<&str, &FileInput>,
        dirs: &HashMap<&str, &str>,
        edges: &[ModuleEdge],
        mode: Mode,
    ) -> String {
        let s = self.settings();
        let mut lines = vec![
            format!("dir: {}", if m.dir.is_empty() { "/" } else { &m.dir }),
            format!("files: {}", m.files.len()),
        ];
        let mut tokens = 0u64;
        let mut paths: Vec<&String> = m.files.iter().collect();
        paths.sort();
        for (i, path) in paths.iter().enumerate() {
            if tokens > s.max_module_tokens {
                lines.push(format!("- +{} more files", paths.len() - i));
                break;
            }
            let mut block = Vec::new();
            match by_path.get(path.as_str()) {
                Some(f) => {
                    let cached = self.cached_file(f);
                    let overview = match (&cached.overview, mode) {
                        (Some(o), _) => o.text.clone(),
                        (None, Mode::Estimate) => PLACEHOLDER_OVERVIEW.to_string(),
                        (None, Mode::Cached) => String::new(),
                    };
                    block.push(if overview.is_empty() {
                        format!("- {path}")
                    } else {
                        format!("- {path}: {overview}")
                    });
                    let mut syms: Vec<&iso_core::SymbolFact> = f
                        .facts
                        .symbols
                        .iter()
                        .filter(|x| x.kind.is_callable() || x.kind.is_type_like())
                        .collect();
                    syms.sort_by_key(|x| (!x.exported, x.start_line));
                    for sym in syms.into_iter().take(6) {
                        let desc = if sym.kind.is_callable() {
                            cached
                                .summaries
                                .get(&sym.qualified_name)
                                .map(|x| x.desc.clone())
                        } else {
                            cached.types.get(&sym.qualified_name).cloned()
                        };
                        match (desc, mode) {
                            (Some(d), _) => block.push(format!("  {}: {d}", sym.qualified_name)),
                            (None, Mode::Estimate) => {
                                block.push(format!("  {}: {PLACEHOLDER_DESC}", sym.qualified_name))
                            }
                            (None, Mode::Cached) => {}
                        }
                    }
                }
                None => block.push(format!("- {path}")),
            }
            for l in &block {
                tokens += estimate_tokens(l);
            }
            lines.extend(block);
        }
        let mut out: Vec<(&ModuleEdge, &str)> = edges
            .iter()
            .filter(|e| e.from == m.id && e.to != m.id)
            .map(|e| (e, e.to.as_str()))
            .collect();
        let mut inc: Vec<(&ModuleEdge, &str)> = edges
            .iter()
            .filter(|e| e.to == m.id && e.from != m.id)
            .map(|e| (e, e.from.as_str()))
            .collect();
        for (list, label) in [(&mut out, "depends on"), (&mut inc, "used by")] {
            if list.is_empty() {
                continue;
            }
            list.sort_by(|a, b| b.0.weight.cmp(&a.0.weight).then(a.1.cmp(b.1)));
            let parts: Vec<String> = list
                .iter()
                .take(8)
                .map(|(e, other)| {
                    let d = dirs.get(other).copied().unwrap_or(other);
                    format!(
                        "{} ({} imports)",
                        if d.is_empty() { "/" } else { d },
                        e.weight
                    )
                })
                .collect();
            lines.push(format!("{label}: {}", parts.join(", ")));
        }
        lines.join("\n")
    }

    fn module_items(
        &self,
        modules: &[ModuleInput],
        edges: &[ModuleEdge],
        files: &[FileInput],
        mode: Mode,
        model: &str,
    ) -> Vec<ModuleItem> {
        let by_path: HashMap<&str, &FileInput> =
            files.iter().map(|f| (f.path.as_str(), f)).collect();
        let dirs: HashMap<&str, &str> = modules
            .iter()
            .map(|m| (m.id.as_str(), m.dir.as_str()))
            .collect();
        let system = cluster_system();
        let s = self.settings();
        modules
            .iter()
            .enumerate()
            .map(|(index, m)| {
                let block = self.module_block(m, &by_path, &dirs, edges, mode);
                let content = format!("module:{:016x}", hash64(block.as_bytes()));
                let dir = if m.dir.is_empty() { "/" } else { m.dir.as_str() };
                let prompt = format!(
                    "Task: cluster\nName the module below. Answer with the JSON object described in the instructions.\n\n### m1 {dir}\n{block}\n{}",
                    crate::prompts::STE_REMINDER
                );
                let max_tokens = crate::tokens::output_limit(OUT_TOKENS_ENVELOPE + OUT_TOKENS_MODULE, s.max_output_tokens);
                ModuleItem {
                    index,
                    content,
                    request: CompletionRequest {
                        model: model.to_string(),
                        system: system.clone(),
                        prompt,
                        max_tokens,
                    },
                }
            })
            .collect()
    }

    fn summary_for(
        &self,
        m: &ModuleInput,
        answer: Option<ClusterAnswer>,
        color: &str,
    ) -> ModuleSummary {
        let (name, role, responsibilities, model, ai) = match answer {
            Some(a) => (a.name, a.role, a.responsibilities, a.model, true),
            None => (
                title_from_dir(&m.dir, &m.id),
                String::new(),
                Vec::new(),
                String::new(),
                false,
            ),
        };
        ModuleSummary {
            id: m.id.clone(),
            dir: m.dir.clone(),
            files: m.files.len() as u32,
            name,
            role,
            responsibilities,
            color: color.to_string(),
            color_dark: dark_variant(color),
            model,
            ai,
        }
    }

    fn colors(
        &self,
        modules: &[ModuleInput],
        existing: &HashMap<String, String>,
    ) -> HashMap<String, String> {
        let ids: Vec<String> = modules.iter().map(|m| m.id.clone()).collect();
        assign_colors(&ids, existing)
    }

    /// Module summaries from the cache only (no provider call). Modules without an answer
    /// get a name derived from their directory and `ai: false`. Colors are always assigned.
    pub fn cached_modules(
        &self,
        modules: &[ModuleInput],
        edges: &[ModuleEdge],
        files: &[FileInput],
        existing_colors: &HashMap<String, String>,
    ) -> Vec<ModuleSummary> {
        let model = self.settings().effective_cluster_model().to_string();
        let colors = self.colors(modules, existing_colors);
        self.module_items(modules, edges, files, Mode::Cached, &model)
            .into_iter()
            .map(|it| {
                let m = &modules[it.index];
                let answer =
                    self.cache
                        .get_json::<ClusterAnswer>(&it.content, &model, kind::CLUSTER);
                self.summary_for(m, answer, &colors[&m.id])
            })
            .collect()
    }

    /// Estimates the cluster pass. Members without cached summaries count at a typical size.
    pub fn estimate_cluster(
        &self,
        modules: &[ModuleInput],
        edges: &[ModuleEdge],
        files: &[FileInput],
    ) -> PassEstimate {
        let model = self.settings().effective_cluster_model().to_string();
        let items = self.module_items(modules, edges, files, Mode::Estimate, &model);
        let pending: Vec<&ModuleItem> = items
            .iter()
            .filter(|it| !self.cache.contains(&it.content, &model, kind::CLUSTER))
            .collect();
        let mut e = if pending.is_empty() {
            PassEstimate {
                model: model.clone(),
                ..Default::default()
            }
        } else {
            let price = self.price(&model);
            let reqs: Vec<Planned> = pending
                .iter()
                .map(|it| Planned {
                    request: &it.request,
                    est_out: OUT_TOKENS_ENVELOPE + OUT_TOKENS_MODULE,
                })
                .collect();
            sum_requests(&price, self.calibration(), &reqs)
        };
        e.items = items.len() as u32;
        e.cached = (items.len() - pending.len()) as u32;
        e
    }

    /// Runs the cluster pass: names, roles and responsibilities for every module whose
    /// answer is not cached, then colors for all of them. `existing_colors` (stored or
    /// user-chosen colors) are kept. Refuses when the estimate exceeds the budget.
    pub fn cluster_pass(
        &self,
        modules: &[ModuleInput],
        edges: &[ModuleEdge],
        files: &[FileInput],
        existing_colors: &HashMap<String, String>,
        budget_usd: f64,
        on_progress: &mut dyn FnMut(&Progress),
    ) -> Result<ClusterReport, AiError> {
        let s = self.settings();
        let model = s.effective_cluster_model().to_string();
        let items = self.module_items(modules, edges, files, Mode::Cached, &model);
        let pending: Vec<&ModuleItem> = items
            .iter()
            .filter(|it| !self.cache.contains(&it.content, &model, kind::CLUSTER))
            .collect();
        let mut report = RunReport::new(Pass::Cluster, &model);
        if !pending.is_empty() {
            let price = self.price(&model);
            let reqs: Vec<Planned> = pending
                .iter()
                .map(|it| Planned {
                    request: &it.request,
                    est_out: OUT_TOKENS_ENVELOPE + OUT_TOKENS_MODULE,
                })
                .collect();
            let est = sum_requests(&price, self.calibration(), &reqs);
            if est.cost_usd > budget_usd + EPS {
                return Err(AiError::Budget(format!(
                    "the cluster pass is estimated at ${:.4}, over the budget of ${:.4}",
                    est.cost_usd, budget_usd
                )));
            }
            let mut m = Meter::new(
                Pass::Cluster,
                price,
                self.calibration(),
                budget_usd,
                pending.len() as u32,
                on_progress,
            );
            for it in &pending {
                let module = &modules[it.index];
                let est_out = OUT_TOKENS_ENVELOPE + OUT_TOKENS_MODULE;
                if !m.can_afford(&it.request, est_out) {
                    m.report.stopped_by_budget = true;
                    break;
                }
                match self.provider.complete(&m.capped(&it.request)) {
                    Ok(c) => {
                        m.record(&it.request, &c);
                        let parsed = parse::extract_json(&c.text)
                            .and_then(|v| {
                                let list = parse::items(&v)?;
                                list.iter()
                                    .find(|x| parse::item_id(x).as_deref() == Some("m1"))
                                    .or(list.first())
                                    .cloned()
                                    .ok_or_else(|| "the answer has no entry m1".to_string())
                            })
                            .and_then(|v| parse::parse_module(&v));
                        match parsed {
                            Ok((name, role, responsibilities)) => {
                                let a = ClusterAnswer {
                                    name,
                                    role,
                                    responsibilities,
                                    model: c.model.clone(),
                                };
                                match self.cache.put_json(
                                    &it.content,
                                    &model,
                                    kind::CLUSTER,
                                    &a,
                                    c.tokens_in,
                                    c.tokens_out,
                                ) {
                                    Ok(()) => m.report.stored += 1,
                                    Err(e) => m.item_error(module.id.clone(), e.to_string()),
                                }
                            }
                            Err(e) => m.item_error(module.id.clone(), e),
                        }
                    }
                    Err(e) => {
                        if let Some(stop) = m.failed(e, vec![module.id.clone()]) {
                            m.emit(true, Some(stop.to_string()));
                            return Err(stop);
                        }
                        if m.too_many_failures(s.max_consecutive_failures) {
                            break;
                        }
                    }
                }
                m.emit(false, None);
            }
            m.finish(s.max_consecutive_failures);
            report = m.report;
        } else {
            on_progress(&Progress {
                pass: Pass::Cluster,
                done: 0,
                total: 0,
                requests: 0,
                tokens_in: 0,
                tokens_out: 0,
                cost_usd: 0.0,
                finished: true,
                error: None,
            });
        }
        report.items = items.len() as u32;
        report.cached = (items.len() - pending.len()) as u32;
        let summaries = self.cached_modules(modules, edges, files, existing_colors);
        Ok(ClusterReport {
            run: report,
            modules: summaries,
        })
    }

    fn repo_request(
        &self,
        modules: &[ModuleSummary],
        edges: &[ModuleEdge],
        estimate: bool,
    ) -> (String, CompletionRequest, u64) {
        let s = self.settings();
        let keys: HashMap<&str, String> = modules
            .iter()
            .enumerate()
            .map(|(i, m)| (m.id.as_str(), format!("m{}", i + 1)))
            .collect();
        let mut prompt = String::from(
            "Task: repo\nDescribe the architecture of the modules below. Answer with the JSON object described in the instructions.\n\n",
        );
        for (i, m) in modules.iter().enumerate() {
            let role = if m.role.is_empty() && estimate {
                PLACEHOLDER_ROLE.to_string()
            } else {
                m.role.clone()
            };
            prompt.push_str(&format!("### m{} {}\n", i + 1, m.name));
            prompt.push_str(&format!(
                "dir: {}\nfiles: {}\n",
                if m.dir.is_empty() { "/" } else { &m.dir },
                m.files
            ));
            if !role.is_empty() {
                prompt.push_str(&format!("role: {}\n", clip(&role, 240)));
            }
            let mut deps: Vec<&ModuleEdge> = edges
                .iter()
                .filter(|e| e.from == m.id && e.to != m.id)
                .collect();
            deps.sort_by(|a, b| b.weight.cmp(&a.weight).then(a.to.cmp(&b.to)));
            let deps: Vec<&str> = deps
                .iter()
                .filter_map(|e| keys.get(e.to.as_str()).map(String::as_str))
                .take(12)
                .collect();
            if !deps.is_empty() {
                prompt.push_str(&format!("depends on: {}\n", deps.join(", ")));
            }
        }
        prompt.push_str(crate::prompts::STE_REMINDER);
        let content = format!("repo:{:016x}", hash64(prompt.as_bytes()));
        let est_out = OUT_TOKENS_REPO + OUT_TOKENS_REPO_PER_MODULE * modules.len() as u64;
        let req = CompletionRequest {
            model: s.repo_model.clone(),
            system: repo_system(),
            prompt,
            max_tokens: crate::tokens::output_limit(est_out, s.max_output_tokens),
        };
        (content, req, est_out)
    }

    fn overview_from(&self, modules: &[ModuleSummary], a: RepoAnswer) -> RepoOverview {
        RepoOverview {
            overview: a.overview,
            layers: a.layers,
            legend: modules
                .iter()
                .map(|m| (m.id.clone(), m.name.clone(), m.color.clone()))
                .collect(),
            model: a.model,
        }
    }

    /// The cached architecture overview for these module summaries, if any.
    pub fn cached_repo(
        &self,
        modules: &[ModuleSummary],
        edges: &[ModuleEdge],
    ) -> Option<RepoOverview> {
        let (content, req, _) = self.repo_request(modules, edges, false);
        self.cache
            .get_json::<RepoAnswer>(&content, &req.model, kind::REPO)
            .map(|a| self.overview_from(modules, a))
    }

    /// Estimates the repo pass from the modules' cached (or placeholder) summaries.
    pub fn estimate_repo(
        &self,
        modules: &[ModuleInput],
        edges: &[ModuleEdge],
        files: &[FileInput],
    ) -> PassEstimate {
        let summaries = self.cached_modules(modules, edges, files, &HashMap::new());
        let (content, req, est_out) = self.repo_request(&summaries, edges, true);
        let cached =
            summaries.iter().all(|m| m.ai) && self.cache.contains(&content, &req.model, kind::REPO);
        let mut e = if cached {
            PassEstimate {
                model: req.model.clone(),
                ..Default::default()
            }
        } else {
            let price = self.price(&req.model);
            sum_requests(
                &price,
                self.calibration(),
                &[Planned {
                    request: &req,
                    est_out,
                }],
            )
        };
        e.items = 1;
        e.cached = cached as u32;
        e
    }

    /// Runs the repo pass: the architecture overview and layer names from the module
    /// summaries and the module graph. Refuses when the estimate exceeds the budget.
    pub fn repo_pass(
        &self,
        modules: &[ModuleSummary],
        edges: &[ModuleEdge],
        budget_usd: f64,
        on_progress: &mut dyn FnMut(&Progress),
    ) -> Result<RepoReport, AiError> {
        let (content, req, est_out) = self.repo_request(modules, edges, false);
        let model = req.model.clone();
        let mut report = RunReport::new(Pass::Repo, &model);
        report.items = 1;
        if let Some(a) = self
            .cache
            .get_json::<RepoAnswer>(&content, &model, kind::REPO)
        {
            report.cached = 1;
            on_progress(&Progress {
                pass: Pass::Repo,
                done: 0,
                total: 0,
                requests: 0,
                tokens_in: 0,
                tokens_out: 0,
                cost_usd: 0.0,
                finished: true,
                error: None,
            });
            return Ok(RepoReport {
                run: report,
                overview: Some(self.overview_from(modules, a)),
            });
        }
        let price = self.price(&model);
        let est = sum_requests(
            &price,
            self.calibration(),
            &[Planned {
                request: &req,
                est_out,
            }],
        );
        if est.cost_usd > budget_usd + EPS {
            return Err(AiError::Budget(format!(
                "the repo pass is estimated at ${:.4}, over the budget of ${:.4}",
                est.cost_usd, budget_usd
            )));
        }
        let mut m = Meter::new(
            Pass::Repo,
            price,
            self.calibration(),
            budget_usd,
            1,
            on_progress,
        );
        m.report.items = 1;
        let mut overview = None;
        match self.provider.complete(&m.capped(&req)) {
            Ok(c) => {
                m.record(&req, &c);
                let keys: HashMap<String, &str> = modules
                    .iter()
                    .enumerate()
                    .map(|(i, x)| (format!("m{}", i + 1), x.id.as_str()))
                    .collect();
                match parse::extract_json(&c.text).and_then(|v| parse::parse_repo(&v)) {
                    Ok((text, raw_layers)) => {
                        let mut placed: HashSet<&str> = HashSet::new();
                        let mut layers: Vec<RepoLayer> = raw_layers
                            .into_iter()
                            .map(|(name, ids)| RepoLayer {
                                name,
                                modules: ids
                                    .iter()
                                    .filter_map(|k| keys.get(k).copied())
                                    .filter(|id| placed.insert(id))
                                    .map(str::to_string)
                                    .collect(),
                            })
                            .filter(|l| !l.modules.is_empty())
                            .collect();
                        let missing: Vec<String> = modules
                            .iter()
                            .filter(|x| !placed.contains(x.id.as_str()))
                            .map(|x| x.id.clone())
                            .collect();
                        if !missing.is_empty() {
                            match layers.last_mut() {
                                Some(l) => l.modules.extend(missing),
                                None => layers.push(RepoLayer {
                                    name: "Modules".to_string(),
                                    modules: missing,
                                }),
                            }
                        }
                        let a = RepoAnswer {
                            overview: text,
                            layers,
                            model: c.model.clone(),
                        };
                        match self.cache.put_json(
                            &content,
                            &model,
                            kind::REPO,
                            &a,
                            c.tokens_in,
                            c.tokens_out,
                        ) {
                            Ok(()) => m.report.stored += 1,
                            Err(e) => m.item_error("repo".to_string(), e.to_string()),
                        }
                        overview = Some(self.overview_from(modules, a));
                    }
                    Err(e) => m.item_error("repo".to_string(), e),
                }
            }
            Err(e) => {
                if let Some(stop) = m.failed(e, vec!["repo".to_string()]) {
                    m.emit(true, Some(stop.to_string()));
                    return Err(stop);
                }
            }
        }
        let err = m.report.errors.first().map(|e| e.message.clone());
        m.emit(true, err);
        Ok(RepoReport {
            run: m.report,
            overview,
        })
    }
}
