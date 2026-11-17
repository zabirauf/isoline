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
