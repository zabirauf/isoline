//! AI orchestration. Every model call goes through [`Provider`]; tests use [`MockProvider`].
//!
//! Contract (frozen in phase 0): `Provider`, `CompletionRequest`, `Completion`, `ModelInfo`.
//! Owned by workstream I (with the sidecar, workstream F).
//!
//! The rest of the crate is built around those types:
//!
//! - [`skeleton`] turns file facts, source and call flow into compact per-symbol skeletons.
//! - [`AiCache`] keeps every answer in the shared `ai_cache` table, keyed by content hash,
//!   [`PROMPT_VERSION`], model and kind.
//! - [`AiEngine`] plans, estimates and runs the leaf, cluster and repo passes, cache first,
//!   within a budget.
//! - [`SidecarProvider`] talks to the Go Copilot sidecar over JSON-RPC on stdio.
//!
//! See `crates/iso-ai/README.md` for the engine-facing API.

use serde::{Deserialize, Serialize};

pub mod cache;
pub mod engine;
pub mod log;
pub mod mock;
pub mod palette;
pub mod parse;
pub mod prefs;
pub mod pricing;
pub mod prompts;
pub mod sidecar;
pub mod skeleton;
pub mod tokens;

mod cluster;
mod leaf;
pub mod review;

pub use cache::AiCache;
pub use engine::{
    AiEngine, AiSettings, CachedFile, ClusterReport, Estimate, FileInput, FullReport, ItemError,
    ModuleInput, ModuleSummary, Pass, PassEstimate, Progress, RepoFlow, RepoLayer, RepoOverview,
    RepoReport, RunReport,
};
pub use mock::MockProvider;
pub use palette::{ModuleColor, PALETTE};
pub use prefs::AiPrefs;
pub use pricing::{PricingTable, default_models};
pub use prompts::PROMPT_VERSION;
pub use review::{
    ChangeReview, ReviewEstimate, ReviewFile, ReviewInput, ReviewModule, ReviewNote, ReviewNoteKind,
};
pub use sidecar::{DeviceCode, SidecarConfig, SidecarProvider, SidecarStatus};
pub use skeleton::{FileSkeleton, SkeletonOptions, SymbolSkeleton, build_skeleton};
pub use tokens::{Calibration, estimate_tokens};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    /// USD per million fresh input tokens.
    pub input_per_m: f64,
    /// USD per million output tokens.
    pub output_per_m: f64,
    /// USD per million cached input tokens.
    pub cached_input_per_m: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionRequest {
    pub model: String,
    /// Replaces the SDK's system message entirely.
    pub system: String,
    pub prompt: String,
    pub max_tokens: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Completion {
    pub text: String,
    pub model: String,
    pub tokens_in: u64,
    pub tokens_out: u64,
    /// Input tokens served from the provider's prompt cache.
    pub cached_tokens_in: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum AiError {
    #[error("not signed in")]
    NotSignedIn,
    #[error("budget exceeded: {0}")]
    Budget(String),
    #[error("provider error: {0}")]
    Provider(String),
}

/// The only way to reach a model.
pub trait Provider: Send + Sync {
    fn list_models(&self) -> Result<Vec<ModelInfo>, AiError>;
    fn complete(&self, req: &CompletionRequest) -> Result<Completion, AiError>;
    /// How billed tokens relate to estimates for this provider.
    fn calibration(&self) -> Calibration {
        Calibration::EXACT
    }
    /// How many requests a pass may have in flight at once.
    fn max_concurrency(&self) -> usize {
        1
    }
}

impl<P: Provider + ?Sized> Provider for std::sync::Arc<P> {
    fn list_models(&self) -> Result<Vec<ModelInfo>, AiError> {
        (**self).list_models()
    }
    fn complete(&self, req: &CompletionRequest) -> Result<Completion, AiError> {
        (**self).complete(req)
    }
    fn calibration(&self) -> Calibration {
        (**self).calibration()
    }
    fn max_concurrency(&self) -> usize {
        (**self).max_concurrency()
    }
}
