//! Model prices and cost arithmetic.
//!
//! Prices come from the sidecar's `listModels` at startup. When a model is missing (mock
//! mode, or a model the user typed in), the defaults below apply: the analysis plan's
//! Copilot rates from November 15, 2026, with cached input at a tenth of fresh input.

use crate::ModelInfo;

/// Default leaf-pass model.
pub const DEFAULT_LEAF_MODEL: &str = "claude-haiku-5.5";
/// Default cluster-pass model.
pub const DEFAULT_CLUSTER_MODEL: &str = "claude-sonnet-5.5";
/// Default repo-pass model.
pub const DEFAULT_REPO_MODEL: &str = "claude-opus-5.5";

/// The analysis plan's pricing table (USD per million tokens).
pub fn default_models() -> Vec<ModelInfo> {
    let m = |id: &str, name: &str, i: f64, o: f64| ModelInfo {
        id: id.into(),
        name: name.into(),
        input_per_m: i,
        output_per_m: o,
        cached_input_per_m: i / 10.0,
    };
    vec![
        m(DEFAULT_LEAF_MODEL, "Claude Haiku 5.5", 0.10, 0.50),
        m(DEFAULT_CLUSTER_MODEL, "Claude Sonnet 5.5", 2.00, 10.00),
        m(DEFAULT_REPO_MODEL, "Claude Opus 5.5", 4.00, 20.00),
    ]
}

/// Prices by model id, with a fallback for unknown models.
#[derive(Clone, Debug, Default)]
pub struct PricingTable {
    models: Vec<ModelInfo>,
    /// The table came from a non-empty `listModels`.
    listed: bool,
}

impl PricingTable {
    /// A table from `listModels`, topped up with the defaults for ids it lacks.
    pub fn new(models: Vec<ModelInfo>) -> Self {
        let listed = !models.is_empty();
        let mut all = models;
        for d in default_models() {
            if !all.iter().any(|m| m.id == d.id) {
                all.push(d);
            }
        }
        PricingTable {
            models: all,
            listed,
        }
    }

    pub fn listed(&self) -> bool {
        self.listed
    }

    pub fn models(&self) -> &[ModelInfo] {
        &self.models
    }

    /// Prices for a model id. Unknown ids borrow the default of the same family (haiku,
    /// sonnet, opus) and otherwise the most expensive known model, so estimates err high.
    pub fn get(&self, id: &str) -> ModelInfo {
        if let Some(m) = self.models.iter().find(|m| m.id == id) {
            return m.clone();
        }
        let lower = id.to_ascii_lowercase();
        let defaults = default_models();
        let family = ["haiku", "sonnet", "opus"]
            .iter()
            .position(|f| lower.contains(f))
            .map(|i| defaults[i].clone());
        let mut m = family.unwrap_or_else(|| {
            self.models
                .iter()
                .max_by(|a, b| a.output_per_m.total_cmp(&b.output_per_m))
                .cloned()
                .unwrap_or_else(|| defaults[2].clone())
        });
        m.id = id.to_string();
        m
    }

    /// The cheapest and the most capable (most expensive) listed models, used when the
    /// settings name models the provider does not offer.
    pub fn cheapest(&self) -> Option<&ModelInfo> {
        self.models
            .iter()
            .filter(|m| m.output_per_m > 0.0)
            .min_by(|a, b| a.output_per_m.total_cmp(&b.output_per_m))
    }
}

/// Cost in USD of one call.
pub fn cost_usd(model: &ModelInfo, tokens_in: u64, cached_in: u64, tokens_out: u64) -> f64 {
    let cached = cached_in.min(tokens_in);
    let fresh = tokens_in - cached;
    (fresh as f64 * model.input_per_m
        + cached as f64 * model.cached_input_per_m
        + tokens_out as f64 * model.output_per_m)
        / 1_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_costs() {
        let t = PricingTable::new(vec![]);
        let haiku = t.get(DEFAULT_LEAF_MODEL);
        // The analysis plan: 4M in, 1.5M out on the leaf model is $1.15.
        let c = cost_usd(&haiku, 4_000_000, 0, 1_500_000);
        assert!((c - 1.15).abs() < 1e-9, "{c}");
        // Cached input costs a tenth.
        let c = cost_usd(&haiku, 1_000_000, 1_000_000, 0);
        assert!((c - 0.01).abs() < 1e-9, "{c}");
    }

    #[test]
    fn unknown_models_fall_back() {
        let t = PricingTable::new(vec![]);
        assert_eq!(t.get("claude-sonnet-9").output_per_m, 10.0);
        assert_eq!(t.get("gpt-x").output_per_m, 20.0);
        assert_eq!(t.get("gpt-x").id, "gpt-x");
        assert_eq!(t.cheapest().unwrap().id, DEFAULT_LEAF_MODEL);
    }
}
