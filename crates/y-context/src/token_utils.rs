//! Shared token estimation utilities.

/// Estimate token count using the chars/4 heuristic.
pub fn estimate_tokens(text: &str) -> u32 {
    u32::try_from(text.len().div_ceil(4)).unwrap_or(u32::MAX)
}
