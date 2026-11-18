//! Token estimates. One rule everywhere: `ceil(chars / 4)`, the same rule the sidecar's mock
//! mode uses, so estimates and mock counts agree by construction for input.

use crate::CompletionRequest;

/// Estimated tokens in a text: `ceil(chars / 4)`.
pub fn estimate_tokens(s: &str) -> u64 {
    (s.chars().count() as u64).div_ceil(4)
}

/// Estimated input tokens of a request: system and prompt are counted separately.
pub fn request_tokens(req: &CompletionRequest) -> u64 {
    estimate_tokens(&req.system) + estimate_tokens(&req.prompt)
}

/// How a provider's billed tokens relate to the estimate of a request. Copilot adds its own
/// framing to every prompt and bills reasoning as output, so a live answer costs more than its
/// text; the mock counts exactly what it sees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Calibration {
    pub in_factor: f64,
    pub in_overhead: u64,
    pub out_factor: f64,
    pub out_overhead: u64,
}

impl Calibration {
    pub const EXACT: Calibration = Calibration {
        in_factor: 1.0,
        in_overhead: 0,
        out_factor: 1.0,
        out_overhead: 0,
    };
    /// Measured on openclaw files with claude-haiku-5.5 through the Copilot CLI (November
    /// 2026): billed input ran 1.5 to 1.6 times the estimate, billed output 2.4 to 4.6 times.
    pub const COPILOT: Calibration = Calibration {
        in_factor: 1.25,
        in_overhead: 400,
        out_factor: 2.5,
        out_overhead: 300,
    };

    pub fn tokens_in(&self, estimated: u64) -> u64 {
        (estimated as f64 * self.in_factor).ceil() as u64 + self.in_overhead
    }

    pub fn tokens_out(&self, estimated: u64) -> u64 {
        (estimated as f64 * self.out_factor).ceil() as u64 + self.out_overhead
    }
}

impl Default for Calibration {
    fn default() -> Self {
        Calibration::EXACT
    }
}

/// Room for an answer of `est_out` tokens: reasoning models spend output tokens before the
/// first visible one, and an answer cut off at the limit cannot be read at all.
pub fn output_limit(est_out: u64, cap: u32) -> u32 {
    (est_out * 6 + 2048).min(cap as u64) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ceil_of_chars_over_four() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("a"), 1);
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("abcde"), 2);
        assert_eq!(estimate_tokens("ééééé"), 2);
    }
}
