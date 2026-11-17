//! AI orchestration. Every model call goes through [`Provider`]; tests use [`MockProvider`].
//!
//! Contract (frozen in phase 0): `Provider`, `CompletionRequest`, `Completion`, `ModelInfo`.
//! Owned by workstream I (with the sidecar, workstream F).

use serde::{Deserialize, Serialize};

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
}
