//! Per-provider metrics tracking.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Per-provider metrics counters.
///
/// Uses atomics for lock-free concurrent updates from multiple request tasks.
#[derive(Debug)]
pub struct ProviderMetrics {
    pub total_requests: AtomicU64,
    pub total_errors: AtomicU64,
    pub total_input_tokens: AtomicU64,
    pub total_output_tokens: AtomicU64,
}

impl ProviderMetrics {
    /// Create a new zeroed metrics instance.
    pub fn new() -> Self {
        Self {
            total_requests: AtomicU64::new(0),
            total_errors: AtomicU64::new(0),
            total_input_tokens: AtomicU64::new(0),
            total_output_tokens: AtomicU64::new(0),
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
    }

    /// Get a snapshot of current metrics.
    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            total_requests: self.total_requests.load(Ordering::Relaxed),
            total_errors: self.total_errors.load(Ordering::Relaxed),
            total_input_tokens: self.total_input_tokens.load(Ordering::Relaxed),
            total_output_tokens: self.total_output_tokens.load(Ordering::Relaxed),
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
}

impl MetricsSnapshot {
    /// Error rate as a fraction (0.0 to 1.0).
    pub fn error_rate(&self) -> f64 {
        if self.total_requests == 0 {
            0.0
        } else {
            self.total_errors as f64 / self.total_requests as f64
        }
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
}
