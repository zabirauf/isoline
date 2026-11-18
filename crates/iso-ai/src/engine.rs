//! The engine-facing API: plan, estimate and run the passes, cache first, within a budget.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use iso_core::ipc::{AiEstimate, AiProgress, AiScope, AiText, ModuleEdge, Summary};
use iso_core::{FileFacts, FlowUnit, Language, SymbolFact};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use crate::cache::{AiCache, kind};
use crate::leaf;
use crate::pricing::{
    DEFAULT_CLUSTER_MODEL, DEFAULT_LEAF_MODEL, DEFAULT_REPO_MODEL, PricingTable, cost_usd,
};
use crate::skeleton::SkeletonOptions;
use crate::tokens::{Calibration, estimate_tokens, request_tokens};
use crate::{AiError, Completion, CompletionRequest, ModelInfo, Provider};

/// One file, as the engine builds it from the index. `source` is needed only for files
/// with callables that have no cached summary; lookups work with facts alone.
#[derive(Clone, Debug)]
pub struct FileInput {
    /// Repo-relative path with `/` separators.
    pub path: String,
    pub lang: Language,
    pub content_hash: String,
    pub source: String,
    pub facts: FileFacts,
    /// Call-flow units, when the file has been walked. May be empty.
    pub flows: Vec<FlowUnit>,
}

/// A candidate module from the graph.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModuleInput {
    pub id: String,
    /// Directory prefix the module is anchored to.
    pub dir: String,
    /// Repo-relative paths of member files.
    pub files: Vec<String>,
}

/// A module's name, role and color.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleSummary {
    pub id: String,
    pub dir: String,
    pub files: u32,
    pub name: String,
    pub role: String,
    pub responsibilities: Vec<String>,
    /// Light-theme CSS color.
    pub color: String,
    /// Dark-theme CSS color.
    pub color_dark: String,
    pub model: String,
    /// False when the name was derived from the directory because no answer exists yet.
    pub ai: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoLayer {
    pub name: String,
    /// Module ids, in the order given.
    pub modules: Vec<String>,
}

/// A major path that requests or data take through the modules.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoFlow {
    pub name: String,
    pub text: String,
    /// Module ids, in the order the data passes through them.
    pub modules: Vec<String>,
}

/// The architecture overview.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoOverview {
    pub overview: String,
    pub layers: Vec<RepoLayer>,
    pub flows: Vec<RepoFlow>,
    /// Color legend for the source tree: (module id, name, light color).
    pub legend: Vec<(String, String, String)>,
    pub model: String,
}

/// Cached answers for one file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CachedFile {
    /// Function and method summaries by qualified name.
    pub summaries: HashMap<String, Summary>,
    /// One-sentence type summaries by qualified name.
    pub types: HashMap<String, String>,
    pub overview: Option<AiText>,
}

/// Engine settings. Model ids come from `listModels` and are kept in user settings.
#[derive(Clone, Debug)]
pub struct AiSettings {
    pub leaf_model: String,
    pub cluster_model: String,
    pub repo_model: String,
    /// Run the cluster pass on the leaf model (the analysis plan's biggest cost lever).
    pub economy: bool,
    /// Files per leaf request.
    pub files_per_batch: usize,
    /// Upper bound on a leaf request's prompt, in tokens.
    pub max_batch_tokens: u64,
    pub max_items_per_batch: usize,
    /// Upper bound on one module's description in the cluster pass, in tokens.
    pub max_module_tokens: u64,
    /// Cap on `max_tokens` for any request.
    pub max_output_tokens: u32,
    /// Ask for a file overview per file in the leaf pass.
    pub file_overviews: bool,
    /// Stop a pass after this many failed requests in a row.
    pub max_consecutive_failures: u32,
    pub skeleton: SkeletonOptions,
}

impl Default for AiSettings {
    fn default() -> Self {
        AiSettings {
            leaf_model: DEFAULT_LEAF_MODEL.into(),
            cluster_model: DEFAULT_CLUSTER_MODEL.into(),
            repo_model: DEFAULT_REPO_MODEL.into(),
            // The analysis plan defaults new repos to Economy.
            economy: true,
            files_per_batch: 20,
            max_batch_tokens: 24_000,
            max_items_per_batch: 120,
            max_module_tokens: 6_000,
            max_output_tokens: 16_000,
            file_overviews: true,
            max_consecutive_failures: 3,
            skeleton: SkeletonOptions::default(),
        }
    }
}

impl AiSettings {
    /// The model the cluster pass uses: the leaf model in Economy.
    pub fn effective_cluster_model(&self) -> &str {
        if self.economy {
            &self.leaf_model
        } else {
            &self.cluster_model
        }
    }

    /// Settings whose models exist in `models`. A missing model is replaced by the newest
    /// listed model of the nearest family: Opus, then Sonnet for the repo pass; Sonnet, then
    /// Opus for the cluster pass; Haiku for the leaf pass. Without one, the leaf pass takes
    /// the cheapest model and the others the most expensive. "auto" is never chosen.
    pub fn fitted_to(mut self, models: &[ModelInfo]) -> Self {
        let listed: Vec<&ModelInfo> = models.iter().filter(|m| m.id != "auto").collect();
        if listed.is_empty() {
            return self;
        }
        let has = |id: &str| listed.iter().any(|m| m.id == id);
        let newest = |families: &[&str]| {
            families.iter().find_map(|f| {
                listed
                    .iter()
                    .filter(|m| m.id.starts_with(f))
                    .max_by(|a, b| version_key(&a.id).cmp(&version_key(&b.id)))
                    .map(|m| m.id.clone())
            })
        };
        let cheapest = || {
            listed
                .iter()
                .min_by(|a, b| a.output_per_m.total_cmp(&b.output_per_m))
                .map(|m| m.id.clone())
        };
        let priciest = || {
            listed
                .iter()
                .max_by(|a, b| a.output_per_m.total_cmp(&b.output_per_m))
                .map(|m| m.id.clone())
        };
        if !has(&self.leaf_model)
            && let Some(m) = newest(&["claude-haiku"]).or_else(cheapest)
        {
            self.leaf_model = m;
        }
        if !has(&self.cluster_model)
            && let Some(m) = newest(&["claude-sonnet", "claude-opus"]).or_else(priciest)
        {
            self.cluster_model = m;
        }
        if !has(&self.repo_model)
            && let Some(m) = newest(&["claude-opus", "claude-sonnet"]).or_else(priciest)
        {
            self.repo_model = m;
        }
        self
    }
}

/// Sort key for a model id: its numbers compare as numbers, so "claude-sonnet-5.10" is newer
/// than "claude-sonnet-5.9".
fn version_key(id: &str) -> Vec<u64> {
    id.split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse().ok())
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Pass {
    Leaf,
    Cluster,
    Repo,
}

/// The expected size and cost of one pass.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PassEstimate {
    pub model: String,
    /// Items in scope (functions, types, overviews; modules; 1 for the repo).
    pub items: u32,
    /// Items already answered in the cache.
    pub cached: u32,
    pub requests: u32,
    pub tokens_in: u64,
    /// Input tokens expected from the provider's prompt cache (the shared system message).
    pub cached_tokens_in: u64,
    pub tokens_out: u64,
    pub cost_usd: f64,
}

/// Estimate of a run.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Estimate {
    pub scope: AiScope,
    pub leaf: PassEstimate,
    pub cluster: Option<PassEstimate>,
    pub repo: Option<PassEstimate>,
}

impl Estimate {
    fn passes(&self) -> impl Iterator<Item = &PassEstimate> {
        std::iter::once(&self.leaf)
            .chain(self.cluster.as_ref())
            .chain(self.repo.as_ref())
    }
    pub fn requests(&self) -> u32 {
        self.passes().map(|p| p.requests).sum()
    }
    pub fn tokens_in(&self) -> u64 {
        self.passes().map(|p| p.tokens_in).sum()
    }
    pub fn tokens_out(&self) -> u64 {
        self.passes().map(|p| p.tokens_out).sum()
    }
    pub fn cost_usd(&self) -> f64 {
        self.passes().map(|p| p.cost_usd).sum()
    }
    /// The IPC payload of `ai_estimate`.
    pub fn to_ipc(&self) -> AiEstimate {
        // An architecture run sends modules, not symbols: count those instead.
        let (items, cached) = match (&self.cluster, self.scope) {
            (Some(c), AiScope::Architecture) => (c.items, c.cached),
            _ => (self.leaf.items, self.leaf.cached),
        };
        AiEstimate {
            scope: self.scope,
            symbols: items,
            cached,
            requests: self.requests(),
            tokens_in: self.tokens_in(),
            tokens_out: self.tokens_out(),
            cost_usd: self.cost_usd(),
            model: match (&self.cluster, self.scope) {
                (Some(c), AiScope::Architecture) => c.model.clone(),
                _ => self.leaf.model.clone(),
            },
        }
    }
}

/// Progress after each request.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub pass: Pass,
    /// Items answered or failed so far, of `total` items to send.
    pub done: u32,
    pub total: u32,
    pub requests: u32,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cost_usd: f64,
    pub finished: bool,
    pub error: Option<String>,
}

impl Progress {
    /// The IPC payload of `ai_progress`.
    pub fn to_ipc(&self, job_id: u64) -> AiProgress {
        AiProgress {
            job_id,
            done: self.done,
            total: self.total,
            tokens_in: self.tokens_in,
            tokens_out: self.tokens_out,
            cost_usd: self.cost_usd,
            finished: self.finished,
            error: self.error.clone(),
        }
    }
}

/// One item that got no usable answer. It is not cached, so the next run asks again.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ItemError {
    pub pass: Pass,
    /// `path#qualified.name`, a file path, or a module id.
    pub item: String,
    pub message: String,
}

/// What a pass did.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunReport {
    pub pass: Pass,
    pub model: String,
    /// Items in scope.
    pub items: u32,
    /// Items served from the cache without a request.
    pub cached: u32,
    /// Items answered and stored by this run.
    pub stored: u32,
    pub failed: u32,
    pub requests: u32,
    pub tokens_in: u64,
    pub cached_tokens_in: u64,
    pub tokens_out: u64,
    pub cost_usd: f64,
    /// The run stopped before the next request because it would have exceeded the budget.
    pub stopped_by_budget: bool,
    pub errors: Vec<ItemError>,
    /// Files with new answers (for `summary_ready` events).
    pub updated_paths: Vec<String>,
}

impl RunReport {
    pub(crate) fn new(pass: Pass, model: &str) -> Self {
        RunReport {
            pass,
            model: model.to_string(),
            items: 0,
            cached: 0,
            stored: 0,
            failed: 0,
            requests: 0,
            tokens_in: 0,
            cached_tokens_in: 0,
            tokens_out: 0,
            cost_usd: 0.0,
            stopped_by_budget: false,
            errors: Vec::new(),
            updated_paths: Vec::new(),
        }
    }
}

/// Result of the cluster pass: a summary for every module, cached or new.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ClusterReport {
    pub run: RunReport,
    pub modules: Vec<ModuleSummary>,
}

/// Result of the repo pass.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RepoReport {
    pub run: RunReport,
    pub overview: Option<RepoOverview>,
}

/// Result of [`AiEngine::run`].
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FullReport {
    pub leaf: RunReport,
    pub cluster: Option<ClusterReport>,
    pub repo: Option<RepoReport>,
}

impl FullReport {
    pub fn cost_usd(&self) -> f64 {
        self.leaf.cost_usd
            + self.cluster.as_ref().map_or(0.0, |c| c.run.cost_usd)
            + self.repo.as_ref().map_or(0.0, |r| r.run.cost_usd)
    }
}

const EPS: f64 = 1e-12;

/// A request about to be sent, with its expected output.
pub(crate) struct Planned<'a> {
    pub request: &'a CompletionRequest,
    pub est_out: u64,
}

/// Sums the expected tokens and cost of a list of requests. The system message of every
/// request after the first one with the same system message counts as cached.
pub(crate) fn sum_requests(model: &ModelInfo, cal: Calibration, reqs: &[Planned]) -> PassEstimate {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut e = PassEstimate {
        model: model.id.clone(),
        requests: reqs.len() as u32,
        ..Default::default()
    };
    for p in reqs {
        let tin = cal.tokens_in(request_tokens(p.request));
        let cached = if seen.insert(p.request.system.as_str()) {
            0
        } else {
            estimate_tokens(&p.request.system)
        };
        let tout = cal.tokens_out(p.est_out);
        e.tokens_in += tin;
        e.cached_tokens_in += cached;
        e.tokens_out += tout;
        e.cost_usd += cost_usd(model, tin, cached, tout);
    }
    e
}

/// Tracks spend and progress while a pass runs.
pub(crate) struct Meter<'a> {
    pub report: RunReport,
    pub price: ModelInfo,
    pub cal: Calibration,
    /// Budget left for this pass, in USD.
    pub budget: f64,
    pub total: u32,
    pub progress: &'a mut dyn FnMut(&Progress),
    systems: HashSet<String>,
    failures_in_row: u32,
}

impl<'a> Meter<'a> {
    pub fn new(
        pass: Pass,
        price: ModelInfo,
        cal: Calibration,
        budget: f64,
        total: u32,
        progress: &'a mut dyn FnMut(&Progress),
    ) -> Self {
        Meter {
            report: RunReport::new(pass, &price.id),
            price,
            cal,
            budget,
            total,
            progress,
            systems: HashSet::new(),
            failures_in_row: 0,
        }
    }

    /// The request with `max_tokens` lowered so that its output cannot take the spend past
    /// the budget. Providers stop at `max_tokens`, so the budget holds even when answers run
    /// longer than estimated.
    pub fn capped(&self, req: &CompletionRequest) -> CompletionRequest {
        self.capped_reserving(req, 0.0)
    }

    /// Like [`Meter::capped`], with `reserved` USD set aside for requests sent at the same time.
    fn capped_reserving(&self, req: &CompletionRequest, reserved: f64) -> CompletionRequest {
        let tin = self.cal.tokens_in(request_tokens(req));
        let cached = if self.systems.contains(&req.system) {
            estimate_tokens(&req.system)
        } else {
            0
        };
        let left =
            self.budget - self.report.cost_usd - reserved - cost_usd(&self.price, tin, cached, 0);
        let mut out = req.clone();
        if self.price.output_per_m > 0.0 {
            let affordable = (left.max(0.0) * 1_000_000.0 / self.price.output_per_m).floor();
            if affordable < out.max_tokens as f64 {
                out.max_tokens = affordable.max(1.0) as u32;
            }
        }
        out
    }

    /// The estimated cost of a request.
    fn cost_of(&self, req: &CompletionRequest, est_out: u64) -> f64 {
        let tin = self.cal.tokens_in(request_tokens(req));
        let cached = if self.systems.contains(&req.system) {
            estimate_tokens(&req.system)
        } else {
            0
        };
        cost_usd(&self.price, tin, cached, self.cal.tokens_out(est_out))
    }

    /// The next requests to send together: from the front of `next` (request, estimated
    /// output), as many as fit in the budget along with each other. Each comes back capped
    /// so that it cannot take the spend past the budget, counting what the others in the
    /// wave are expected to spend. Empty when not even the first one fits.
    pub fn wave(&self, next: &[(&CompletionRequest, u64)]) -> Vec<CompletionRequest> {
        let mut costs = Vec::new();
        let mut reserved = 0.0;
        for (req, est_out) in next {
            let c = self.cost_of(req, *est_out);
            if self.report.cost_usd + reserved + c > self.budget + EPS {
                break;
            }
            reserved += c;
            costs.push(c);
        }
        next.iter()
            .zip(&costs)
            .map(|((req, _), c)| self.capped_reserving(req, reserved - c))
            .collect()
    }

    pub fn record(&mut self, req: &CompletionRequest, c: &Completion) {
        self.systems.insert(req.system.clone());
        let r = &mut self.report;
        r.requests += 1;
        r.tokens_in += c.tokens_in;
        r.cached_tokens_in += c.cached_tokens_in;
        r.tokens_out += c.tokens_out;
        r.cost_usd += cost_usd(&self.price, c.tokens_in, c.cached_tokens_in, c.tokens_out);
        self.failures_in_row = 0;
    }

    /// Records a failed request. Returns the error to stop with, if the pass should stop.
    pub fn failed(&mut self, e: AiError, items: Vec<String>) -> Option<AiError> {
        let message = e.to_string();
        for item in items {
            self.item_error(item, message.clone());
        }
        if matches!(e, AiError::NotSignedIn) {
            return Some(e);
        }
        self.failures_in_row += 1;
        None
    }

    pub fn too_many_failures(&self, limit: u32) -> bool {
        self.failures_in_row >= limit.max(1)
    }

    pub fn item_error(&mut self, item: String, message: String) {
        self.report.failed += 1;
        self.report.errors.push(ItemError {
            pass: self.report.pass,
            item,
            message,
        });
    }

    /// Emits the one final progress event, with the reason the pass stopped early, if any.
    pub fn finish(&mut self, failure_limit: u32) {
        let err = if self.report.stopped_by_budget {
            Some("stopped at the budget".to_string())
        } else if self.too_many_failures(failure_limit) {
            Some("stopped after repeated failed requests".to_string())
        } else if self.report.stored == 0 && self.report.failed > 0 {
            let why = self
                .report
                .errors
                .first()
                .map(|e| e.message.as_str())
                .unwrap_or("");
            Some(format!("no answer could be used ({why})"))
        } else {
            None
        };
        self.emit(true, err);
    }

    pub fn emit(&mut self, finished: bool, error: Option<String>) {
        let r = &self.report;
        let p = Progress {
            pass: r.pass,
            done: r.stored + r.failed,
            total: self.total,
            requests: r.requests,
            tokens_in: r.tokens_in,
            tokens_out: r.tokens_out,
            cost_usd: r.cost_usd,
            finished,
            error,
        };
        (self.progress)(&p);
    }
}

/// The AI orchestrator: one cache, one provider.
pub struct AiEngine {
    pub(crate) cache: AiCache,
    pub(crate) provider: Arc<dyn Provider>,
    settings: RwLock<AiSettings>,
    pricing: RwLock<Option<PricingTable>>,
}

impl AiEngine {
    /// Opens the shared cache at `cache_path` (normally `<data>/ai_cache.sqlite`).
    pub fn open(cache_path: &Path, provider: Arc<dyn Provider>) -> anyhow::Result<Self> {
        Ok(Self::with_cache(AiCache::open(cache_path)?, provider))
    }

    /// An engine over an in-memory cache, for tests.
    pub fn in_memory(provider: Arc<dyn Provider>) -> anyhow::Result<Self> {
        Ok(Self::with_cache(AiCache::in_memory()?, provider))
    }

    pub fn with_cache(cache: AiCache, provider: Arc<dyn Provider>) -> Self {
        AiEngine {
            cache,
            provider,
            settings: RwLock::new(AiSettings::default()),
            pricing: RwLock::new(None),
        }
    }

    pub fn cache(&self) -> &AiCache {
        &self.cache
    }

    /// How billed tokens relate to estimates for the current provider.
    pub fn calibration(&self) -> Calibration {
        self.provider.calibration()
    }

    pub fn settings(&self) -> AiSettings {
        self.settings.read().clone()
    }

    pub fn set_settings(&self, s: AiSettings) {
        *self.settings.write() = s;
    }

    /// Lists models from the provider and keeps their prices. On failure the analysis
    /// plan's defaults are used and the error is returned.
    pub fn refresh_models(&self) -> Result<Vec<ModelInfo>, AiError> {
        match self.provider.list_models() {
            Ok(models) => {
                *self.pricing.write() = Some(PricingTable::new(models.clone()));
                Ok(models)
            }
            Err(e) => {
                let mut p = self.pricing.write();
                if p.is_none() {
                    *p = Some(PricingTable::new(Vec::new()));
                }
                Err(e)
            }
        }
    }

    /// Prices for a model. Lists models on first use only, so fully cached runs make no
    /// provider call at all.
    pub fn price(&self, model: &str) -> ModelInfo {
        if let Some(t) = self.pricing.read().as_ref() {
            return t.get(model);
        }
        let _ = self.refresh_models();
        self.pricing
            .read()
            .as_ref()
            .map(|t| t.get(model))
            .unwrap_or_else(|| PricingTable::new(Vec::new()).get(model))
    }

    /// The cached summary of one function or method, if any.
    pub fn cached_summary(&self, sym: &SymbolFact) -> Option<Summary> {
        let model = self.settings.read().leaf_model.clone();
        self.cache.get_json(&leaf::fn_key(sym), &model, kind::LEAF)
    }

    /// Every cached answer for a file: summaries, type sentences and the overview.
    pub fn cached_file(&self, file: &FileInput) -> CachedFile {
        let s = self.settings();
        let (summaries, types, overview) =
            leaf::cached_for_file(&self.cache, &s.leaf_model, &s, file);
        CachedFile {
            summaries,
            types,
            overview,
        }
    }

    /// Estimates the leaf pass over `files`.
    pub fn estimate_leaf(&self, files: &[FileInput]) -> PassEstimate {
        let s = self.settings();
        let plan = leaf::plan(files, &self.cache, &s.leaf_model, &s);
        let mut e = if plan.batches.is_empty() {
            PassEstimate {
                model: s.leaf_model.clone(),
                ..Default::default()
            }
        } else {
            let price = self.price(&s.leaf_model);
            let reqs: Vec<Planned> = plan
                .batches
                .iter()
                .map(|b| Planned {
                    request: &b.request,
                    est_out: b.est_out,
                })
                .collect();
            sum_requests(&price, self.calibration(), &reqs)
        };
        e.items = plan.total;
        e.cached = plan.cached;
        e
    }

    /// Estimates a run. File scope is the leaf pass; module scope adds the cluster pass for
    /// `modules`; repo scope adds the repo pass. Missing leaf summaries are counted at a
    /// typical size in the cluster estimate.
    pub fn estimate(
        &self,
        files: &[FileInput],
        modules: &[ModuleInput],
        edges: &[ModuleEdge],
        scope: AiScope,
    ) -> Estimate {
        // An architecture run names modules from whatever summaries are cached and skips
        // the leaf pass.
        let leaf = if scope == AiScope::Architecture {
            PassEstimate {
                model: self.settings().leaf_model,
                ..Default::default()
            }
        } else {
            self.estimate_leaf(files)
        };
        let cluster = (scope != AiScope::File && !modules.is_empty())
            .then(|| self.estimate_cluster(modules, edges, files, scope != AiScope::Architecture));
        let repo = (matches!(scope, AiScope::Repo | AiScope::Architecture) && !modules.is_empty())
            .then(|| self.estimate_repo(modules, edges, files));
        Estimate {
            scope,
            leaf,
            cluster,
            repo,
        }
    }

    /// Runs the leaf pass over `files`. Refuses with [`AiError::Budget`] when the estimate
    /// exceeds `budget_usd`; stops before any request that would exceed it.
    pub fn run_leaf(
        &self,
        files: &[FileInput],
        budget_usd: f64,
        on_progress: &mut dyn FnMut(&Progress),
    ) -> Result<RunReport, AiError> {
        let s = self.settings();
        let plan = leaf::plan(files, &self.cache, &s.leaf_model, &s);
        let pending: u32 = plan.batches.iter().map(|b| b.items.len() as u32).sum();
        if plan.batches.is_empty() {
            let mut r = RunReport::new(Pass::Leaf, &s.leaf_model);
            r.items = plan.total;
            r.cached = plan.cached;
            on_progress(&Progress {
                pass: Pass::Leaf,
                done: 0,
                total: 0,
                requests: 0,
                tokens_in: 0,
                tokens_out: 0,
                cost_usd: 0.0,
                finished: true,
                error: None,
            });
            return Ok(r);
        }
        let price = self.price(&s.leaf_model);
        let reqs: Vec<Planned> = plan
            .batches
            .iter()
            .map(|b| Planned {
                request: &b.request,
                est_out: b.est_out,
            })
            .collect();
        let est = sum_requests(&price, self.calibration(), &reqs);
        if est.cost_usd > budget_usd + EPS {
            return Err(AiError::Budget(format!(
                "the leaf pass is estimated at ${:.4}, over the budget of ${:.4}",
                est.cost_usd, budget_usd
            )));
        }
        let mut m = Meter::new(
            Pass::Leaf,
            price,
            self.calibration(),
            budget_usd,
            pending,
            on_progress,
        );
        m.report.items = plan.total;
        m.report.cached = plan.cached;
        let mut paths: HashSet<String> = HashSet::new();
        let conc = self.concurrency();
        let mut next = 0;
        while next < plan.batches.len() {
            let front: Vec<(&CompletionRequest, u64)> = plan.batches[next..]
                .iter()
                .take(conc)
                .map(|b| (&b.request, b.est_out))
                .collect();
            let wave = m.wave(&front);
            if wave.is_empty() {
                m.report.stopped_by_budget = true;
                break;
            }
            let answers = self.complete_many(&wave);
            let mut stop = None;
            for (batch, answer) in plan.batches[next..].iter().zip(answers) {
                self.leaf_answer(&mut m, &s, batch, answer, &mut paths, &mut stop);
                m.emit(false, None);
            }
            next += wave.len();
            if let Some(stop) = stop {
                m.emit(true, Some(stop.to_string()));
                return Err(stop);
            }
            if m.too_many_failures(s.max_consecutive_failures) {
                break;
            }
        }
        let mut p: Vec<String> = paths.into_iter().collect();
        p.sort();
        m.report.updated_paths = p;
        m.finish(s.max_consecutive_failures);
        Ok(m.report)
    }

    /// Records one leaf answer: stores what parses, and notes errors and a reason to stop.
    fn leaf_answer(
        &self,
        m: &mut Meter,
        s: &AiSettings,
        batch: &leaf::LeafBatch,
        answer: Result<Completion, AiError>,
        paths: &mut HashSet<String>,
        stop: &mut Option<AiError>,
    ) {
        match answer {
            Ok(c) => {
                m.record(&batch.request, &c);
                let res = leaf::store_answer(
                    &self.cache,
                    batch,
                    &s.leaf_model,
                    &c.model,
                    &c.text,
                    c.tokens_in,
                    c.tokens_out,
                );
                m.report.stored += res.stored;
                for (item, msg) in res.errors {
                    m.item_error(item, msg);
                }
                paths.extend(res.paths);
            }
            Err(e) => {
                let items = batch
                    .items
                    .iter()
                    .map(|(_, i)| format!("{}#{}", i.path, i.name))
                    .collect();
                if let Some(e) = m.failed(e, items) {
                    stop.get_or_insert(e);
                }
            }
        }
    }

    /// How many requests a pass sends at once.
    pub(crate) fn concurrency(&self) -> usize {
        self.provider.max_concurrency().max(1)
    }

    /// Sends the requests at once and returns the answers in the same order.
    pub(crate) fn complete_many(
        &self,
        reqs: &[CompletionRequest],
    ) -> Vec<Result<Completion, AiError>> {
        if reqs.len() == 1 {
            return vec![self.provider.complete(&reqs[0])];
        }
        std::thread::scope(|scope| {
            let handles: Vec<_> = reqs
                .iter()
                .map(|r| scope.spawn(move || self.provider.complete(r)))
                .collect();
            handles
                .into_iter()
                .map(|h| {
                    h.join()
                        .unwrap_or_else(|_| Err(AiError::Provider("a request failed".into())))
                })
                .collect()
        })
    }

    /// Runs the passes a scope needs, in order (leaf, then cluster, then repo), with one
    /// budget shared by all of them.
    #[allow(clippy::too_many_arguments)]
    pub fn run(
        &self,
        files: &[FileInput],
        modules: &[ModuleInput],
        edges: &[ModuleEdge],
        existing_colors: &HashMap<String, String>,
        scope: AiScope,
        budget_usd: f64,
        on_progress: &mut dyn FnMut(&Progress),
    ) -> Result<FullReport, AiError> {
        let est = self.estimate(files, modules, edges, scope);
        if est.cost_usd() > budget_usd + EPS {
            return Err(AiError::Budget(format!(
                "the run is estimated at ${:.4}, over the budget of ${:.4}",
                est.cost_usd(),
                budget_usd
            )));
        }
        let leaf = if scope == AiScope::Architecture {
            RunReport::new(Pass::Leaf, &self.settings().leaf_model)
        } else {
            self.run_leaf(files, budget_usd, on_progress)?
        };
        let mut left = budget_usd - leaf.cost_usd;
        let cluster = if scope != AiScope::File && !modules.is_empty() && !leaf.stopped_by_budget {
            let c = self.cluster_pass(modules, edges, files, existing_colors, left, on_progress)?;
            left -= c.run.cost_usd;
            Some(c)
        } else {
            None
        };
        let repo = match (&cluster, scope) {
            (Some(c), AiScope::Repo | AiScope::Architecture) if !c.run.stopped_by_budget => {
                Some(self.repo_pass(&c.modules, edges, left, on_progress)?)
            }
            _ => None,
        };
        Ok(FullReport {
            leaf,
            cluster,
            repo,
        })
    }
}
