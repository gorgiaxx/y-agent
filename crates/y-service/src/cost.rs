//! Cost computation service.
//!
//! Centralises cost estimation so all frontends share the same pricing logic.

use y_core::types::TokenUsage;

/// Token cost computation.
pub struct CostService;

impl CostService {
    // Per-1k-token rates. Placeholder pricing — in production these should come
    // from provider-specific configuration. Cache rates mirror typical provider
    // ratios: cache reads are far cheaper than fresh input, cache writes cost a
    // little more than fresh input.
    const INPUT_PER_1K: f64 = 0.03;
    const OUTPUT_PER_1K: f64 = 0.06;
    const CACHE_READ_PER_1K: f64 = 0.003;
    const CACHE_WRITE_PER_1K: f64 = 0.0375;

    /// Cache-aware cost estimate from a normalized [`TokenUsage`].
    ///
    /// Fresh input, cache reads, cache writes, and output are each priced at
    /// their own rate so that prompt caching is reflected as a real cost saving
    /// rather than being charged at the full input rate.
    pub fn compute_cost_from_usage(usage: &TokenUsage) -> f64 {
        Self::compute_cost_detailed(
            u64::from(usage.input_tokens),
            u64::from(usage.output_tokens),
            u64::from(usage.cache_read_tokens.unwrap_or(0)),
            u64::from(usage.cache_write_tokens.unwrap_or(0)),
        )
    }

    /// Cache-aware cost estimate from explicit token counts.
    pub fn compute_cost_detailed(
        input_tokens: u64,
        output_tokens: u64,
        cache_read_tokens: u64,
        cache_write_tokens: u64,
    ) -> f64 {
        (input_tokens as f64 / 1000.0) * Self::INPUT_PER_1K
            + (output_tokens as f64 / 1000.0) * Self::OUTPUT_PER_1K
            + (cache_read_tokens as f64 / 1000.0) * Self::CACHE_READ_PER_1K
            + (cache_write_tokens as f64 / 1000.0) * Self::CACHE_WRITE_PER_1K
    }

    /// Rough cost estimate based on fresh input/output token counts only.
    ///
    /// Prefer [`Self::compute_cost_from_usage`] when a full [`TokenUsage`] is
    /// available so cached tokens are priced correctly.
    pub fn compute_cost(input_tokens: u64, output_tokens: u64) -> f64 {
        Self::compute_cost_detailed(input_tokens, output_tokens, 0, 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_cost() {
        let cost = CostService::compute_cost(1000, 1000);
        assert!((cost - 0.09).abs() < 1e-6); // 0.03 + 0.06
    }

    #[test]
    fn test_compute_cost_zero() {
        assert_eq!(CostService::compute_cost(0, 0), 0.0);
    }

    #[test]
    fn test_cache_read_is_cheaper_than_fresh_input() {
        let fresh = CostService::compute_cost_detailed(1000, 0, 0, 0);
        let cached = CostService::compute_cost_detailed(0, 0, 1000, 0);
        assert!(cached < fresh);
        assert!((cached - 0.003).abs() < 1e-6);
    }

    #[test]
    fn test_compute_cost_from_usage_prices_cache_separately() {
        let usage = TokenUsage {
            input_tokens: 491,
            output_tokens: 145,
            cache_read_tokens: Some(80_384),
            cache_write_tokens: Some(0),
            ..TokenUsage::default()
        };

        let cache_aware = CostService::compute_cost_from_usage(&usage);
        // Charging the cached tokens at the full input rate would massively
        // overstate the cost; the cache-aware figure must be far lower.
        let if_charged_as_fresh = CostService::compute_cost(
            u64::from(usage.total_input_tokens()),
            u64::from(usage.output_tokens),
        );
        assert!(cache_aware < if_charged_as_fresh);

        let expected =
            (491.0 / 1000.0) * 0.03 + (145.0 / 1000.0) * 0.06 + (80_384.0 / 1000.0) * 0.003;
        assert!((cache_aware - expected).abs() < 1e-6);
    }
}
