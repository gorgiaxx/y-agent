//! Per-provider metrics tracking with cost accumulation.
//!
//! Design reference: providers-design.md §Observability

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Per-provider metrics counters.
///
/// Uses atomics for lock-free concurrent updates from multiple request tasks.
/// Cost is tracked as micro-dollars (`1_000_000` = $1.00) to avoid floating-point
/// atomics, while still providing sub-cent precision.
#[derive(Debug)]
pub struct ProviderMetrics {
    pub total_requests: AtomicU64,
    pub total_errors: AtomicU64,
    pub total_input_tokens: AtomicU64,
    pub total_output_tokens: AtomicU64,
    /// Estimated accumulated cost in micro-dollars (1e-6 USD).
    estimated_cost_micros: AtomicU64,
}

impl ProviderMetrics {
    /// Create a new zeroed metrics instance.
    pub fn new() -> Self {
        Self {
            total_requests: AtomicU64::new(0),
            total_errors: AtomicU64::new(0),
            total_input_tokens: AtomicU64::new(0),
            total_output_tokens: AtomicU64::new(0),
            estimated_cost_micros: AtomicU64::new(0),
        }
    }

    /// Record a successful request with token usage.
    pub fn record_success(&self, input_tokens: u32, output_tokens: u32) {
        self.total_requests.fetch_add(1, Ordering::Relaxed);
        self.total_input_tokens
            .fetch_add(u64::from(input_tokens), Ordering::Relaxed);
        self.total_output_tokens
            .fetch_add(u64::from(output_tokens), Ordering::Relaxed);
    }

    /// Record a successful request with token usage and cost calculation.
    ///
    /// Cost is computed as:
    /// `(input_tokens / 1000 * cost_per_1k_input) + (output_tokens / 1000 * cost_per_1k_output)`
    pub fn record_success_with_cost(
        &self,
        input_tokens: u32,
        output_tokens: u32,
        cost_per_1k_input: f64,
        cost_per_1k_output: f64,
    ) {
        self.record_success(input_tokens, output_tokens);

        // Calculate cost in micro-dollars.
        let input_cost = f64::from(input_tokens) / 1000.0 * cost_per_1k_input;
        let output_cost = f64::from(output_tokens) / 1000.0 * cost_per_1k_output;
        let total_micros = ((input_cost + output_cost) * 1_000_000.0) as u64;

        self.estimated_cost_micros
            .fetch_add(total_micros, Ordering::Relaxed);
    }

    /// Record a failed request.
    pub fn record_error(&self) {
        self.total_requests.fetch_add(1, Ordering::Relaxed);
        self.total_errors.fetch_add(1, Ordering::Relaxed);
    }

    /// Reset all counters to zero.
    pub fn reset(&self) {
        self.total_requests.store(0, Ordering::Relaxed);
        self.total_errors.store(0, Ordering::Relaxed);
        self.total_input_tokens.store(0, Ordering::Relaxed);
        self.total_output_tokens.store(0, Ordering::Relaxed);
        self.estimated_cost_micros.store(0, Ordering::Relaxed);
    }

    /// Get a snapshot of current metrics.
    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            total_requests: self.total_requests.load(Ordering::Relaxed),
            total_errors: self.total_errors.load(Ordering::Relaxed),
            total_input_tokens: self.total_input_tokens.load(Ordering::Relaxed),
            total_output_tokens: self.total_output_tokens.load(Ordering::Relaxed),
            estimated_cost_micros: self.estimated_cost_micros.load(Ordering::Relaxed),
        }
    }
}

impl Default for ProviderMetrics {
    fn default() -> Self {
        Self::new()
    }
}

/// An immutable snapshot of provider metrics.
#[derive(Debug, Clone)]
pub struct MetricsSnapshot {
    pub total_requests: u64,
    pub total_errors: u64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    /// Estimated accumulated cost in micro-dollars (1e-6 USD).
    pub estimated_cost_micros: u64,
}

impl MetricsSnapshot {
    /// Error rate as a fraction (0.0 to 1.0).
    pub fn error_rate(&self) -> f64 {
        if self.total_requests == 0 {
            return 0.0;
        }
        self.total_errors as f64 / self.total_requests as f64
    }

    /// Estimated total cost in US dollars.
    pub fn estimated_cost_usd(&self) -> f64 {
        self.estimated_cost_micros as f64 / 1_000_000.0
    }
}

/// Thread-safe shared metrics handle.
pub type SharedMetrics = Arc<ProviderMetrics>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_increment_total_requests() {
        let metrics = ProviderMetrics::new();
        metrics.record_success(100, 50);
        assert_eq!(metrics.total_requests.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_metrics_increment_total_errors() {
        let metrics = ProviderMetrics::new();
        metrics.record_error();
        assert_eq!(metrics.total_errors.load(Ordering::Relaxed), 1);
        assert_eq!(metrics.total_requests.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_metrics_track_token_usage() {
        let metrics = ProviderMetrics::new();
        metrics.record_success(100, 50);
        metrics.record_success(200, 100);
        assert_eq!(metrics.total_input_tokens.load(Ordering::Relaxed), 300);
        assert_eq!(metrics.total_output_tokens.load(Ordering::Relaxed), 150);
    }

    #[test]
    fn test_metrics_reset() {
        let metrics = ProviderMetrics::new();
        metrics.record_success(100, 50);
        metrics.record_error();

        metrics.reset();

        let snap = metrics.snapshot();
        assert_eq!(snap.total_requests, 0);
        assert_eq!(snap.total_errors, 0);
        assert_eq!(snap.total_input_tokens, 0);
        assert_eq!(snap.total_output_tokens, 0);
        assert_eq!(snap.estimated_cost_micros, 0);
    }

    #[test]
    fn test_metrics_error_rate() {
        let metrics = ProviderMetrics::new();
        metrics.record_success(10, 5);
        metrics.record_success(10, 5);
        metrics.record_error();

        let snap = metrics.snapshot();
        let rate = snap.error_rate();
        // 1 error out of 3 total requests
        assert!((rate - 1.0 / 3.0).abs() < f64::EPSILON * 10.0);
    }

    // -----------------------------------------------------------------------
    // Cost tracking tests (P1-6)
    // -----------------------------------------------------------------------

    #[test]
    fn test_cost_tracking_basic() {
        let metrics = ProviderMetrics::new();
        // 1000 input tokens at $0.01/1k = $0.01
        // 500 output tokens at $0.03/1k = $0.015
        // Total = $0.025 = 25000 micro-dollars
        metrics.record_success_with_cost(1000, 500, 0.01, 0.03);

        let snap = metrics.snapshot();
        assert_eq!(snap.estimated_cost_micros, 25_000);
        assert!((snap.estimated_cost_usd() - 0.025).abs() < 0.0001);
    }

    #[test]
    fn test_cost_tracking_accumulation() {
        let metrics = ProviderMetrics::new();
        // First request: 1000 input, 500 output
        metrics.record_success_with_cost(1000, 500, 0.01, 0.03);
        // Second request: 2000 input, 1000 output
        metrics.record_success_with_cost(2000, 1000, 0.01, 0.03);

        let snap = metrics.snapshot();
        // Request 1: $0.01 + $0.015 = $0.025
        // Request 2: $0.02 + $0.03 = $0.05
        // Total: $0.075 = 75000 micros
        assert_eq!(snap.estimated_cost_micros, 75_000);
        assert!((snap.estimated_cost_usd() - 0.075).abs() < 0.0001);
    }

    #[test]
    fn test_cost_tracking_zero_tokens() {
        let metrics = ProviderMetrics::new();
        metrics.record_success_with_cost(0, 0, 0.01, 0.03);

        let snap = metrics.snapshot();
        assert_eq!(snap.estimated_cost_micros, 0);
        assert!((snap.estimated_cost_usd()).abs() < f64::EPSILON);
    }

    #[test]
    fn test_cost_tracking_reset() {
        let metrics = ProviderMetrics::new();
        metrics.record_success_with_cost(1000, 500, 0.01, 0.03);
        assert!(metrics.snapshot().estimated_cost_micros > 0);

        metrics.reset();
        assert_eq!(metrics.snapshot().estimated_cost_micros, 0);
    }

    #[test]
    fn test_estimated_cost_usd_conversion() {
        let snap = MetricsSnapshot {
            total_requests: 10,
            total_errors: 0,
            total_input_tokens: 10_000,
            total_output_tokens: 5_000,
            estimated_cost_micros: 1_500_000, // $1.50
        };
        assert!((snap.estimated_cost_usd() - 1.5).abs() < 0.0001);
    }
}
