//! Cost computation service.
//!
//! Centralises cost estimation so all frontends share the same pricing logic.

/// Token cost computation.
pub struct CostService;

impl CostService {
    /// Rough cost estimate based on token counts.
    ///
    /// Placeholder — in production, costs should come from provider-specific
    /// pricing in configuration.
    pub fn compute_cost(input_tokens: u64, output_tokens: u64) -> f64 {
        let input_cost_per_1k = 0.03;
        let output_cost_per_1k = 0.06;
        (input_tokens as f64 / 1000.0) * input_cost_per_1k
            + (output_tokens as f64 / 1000.0) * output_cost_per_1k
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
}
