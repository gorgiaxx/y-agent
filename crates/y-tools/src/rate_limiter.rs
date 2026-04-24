//! Per-tool rate limiter using a token-bucket algorithm.
//!
//! Each tool can have its own rate limit to prevent abuse or overuse.
//! Uses a non-blocking token-bucket approach:
//! - Tokens refill at a configurable rate
//! - Each tool call consumes one token
//! - Requests are rejected when tokens are exhausted
//!
//! Design reference: tools-design.md §Execution Pipeline

use std::collections::{hash_map::Entry, HashMap};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;

/// Rate limit configuration for a single tool.
#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    /// Maximum number of requests allowed in the window.
    pub max_requests: u32,
    /// Time window for the rate limit.
    pub window: Duration,
}

impl RateLimitConfig {
    /// Create a new rate limit: `max_requests` per `window`.
    pub fn new(max_requests: u32, window: Duration) -> Self {
        Self {
            max_requests,
            window,
        }
    }

    /// Create a rate limit of N requests per second.
    pub fn per_second(n: u32) -> Self {
        Self::new(n, Duration::from_secs(1))
    }

    /// Create a rate limit of N requests per minute.
    pub fn per_minute(n: u32) -> Self {
        Self::new(n, Duration::from_secs(60))
    }
}

/// Token bucket state for a single tool.
#[derive(Debug)]
struct TokenBucket {
    config: RateLimitConfig,
    tokens: f64,
    last_refill: Instant,
}

impl TokenBucket {
    fn new(config: RateLimitConfig) -> Self {
        Self {
            tokens: f64::from(config.max_requests),
            last_refill: Instant::now(),
            config,
        }
    }

    /// Refill tokens based on elapsed time.
    fn refill(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill);
        let refill_rate = f64::from(self.config.max_requests) / self.config.window.as_secs_f64();
        self.tokens += elapsed.as_secs_f64() * refill_rate;
        if self.tokens > f64::from(self.config.max_requests) {
            self.tokens = f64::from(self.config.max_requests);
        }
        self.last_refill = now;
    }

    /// Try to consume one token. Returns true if allowed, false if rate limited.
    fn try_acquire(&mut self) -> bool {
        self.refill();
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    /// Returns the time until the next token is available.
    fn time_until_available(&mut self) -> Duration {
        self.refill();
        if self.tokens >= 1.0 {
            Duration::ZERO
        } else {
            let needed = 1.0 - self.tokens;
            let refill_rate =
                f64::from(self.config.max_requests) / self.config.window.as_secs_f64();
            if refill_rate <= 0.0 {
                Duration::from_secs(60) // Fallback
            } else {
                Duration::from_secs_f64(needed / refill_rate)
            }
        }
    }
}

/// Result of a rate limit check.
#[derive(Debug, Clone, PartialEq)]
pub enum RateLimitResult {
    /// Request is allowed.
    Allowed,
    /// Request is denied; retry after the specified duration.
    Denied { retry_after: Duration },
}

/// Per-tool rate limiter.
///
/// Thread-safe: all operations use an internal lock.
#[derive(Debug, Clone)]
pub struct RateLimiter {
    inner: Arc<Mutex<RateLimiterInner>>,
}

#[derive(Debug)]
struct RateLimiterInner {
    buckets: HashMap<String, TokenBucket>,
    default_config: Option<RateLimitConfig>,
}

impl RateLimiter {
    /// Create a new rate limiter with no default config.
    ///
    /// Tools without explicit configs will not be rate limited.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(RateLimiterInner {
                buckets: HashMap::new(),
                default_config: None,
            })),
        }
    }

    /// Create a new rate limiter with a default config for tools without explicit configs.
    pub fn with_default(default: RateLimitConfig) -> Self {
        Self {
            inner: Arc::new(Mutex::new(RateLimiterInner {
                buckets: HashMap::new(),
                default_config: Some(default),
            })),
        }
    }

    /// Set a rate limit for a specific tool.
    pub async fn set_limit(&self, tool_name: &str, config: RateLimitConfig) {
        let mut inner = self.inner.lock().await;
        inner
            .buckets
            .insert(tool_name.to_string(), TokenBucket::new(config));
    }

    /// Remove the rate limit for a specific tool.
    pub async fn remove_limit(&self, tool_name: &str) {
        let mut inner = self.inner.lock().await;
        inner.buckets.remove(tool_name);
    }

    /// Check and consume a rate limit token for the given tool.
    ///
    /// Returns `Allowed` if the request can proceed, or `Denied` with
    /// a retry-after duration if rate limited.
    ///
    pub async fn check(&self, tool_name: &str) -> RateLimitResult {
        let mut inner = self.inner.lock().await;
        let default_config = inner.default_config.clone();
        let bucket = match inner.buckets.entry(tool_name.to_string()) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let Some(default) = default_config else {
                    return RateLimitResult::Allowed;
                };
                entry.insert(TokenBucket::new(default))
            }
        };
        if bucket.try_acquire() {
            RateLimitResult::Allowed
        } else {
            RateLimitResult::Denied {
                retry_after: bucket.time_until_available(),
            }
        }
    }

    /// Get the number of tools with active rate limits.
    pub async fn active_limits(&self) -> usize {
        let inner = self.inner.lock().await;
        inner.buckets.len()
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_rate_limiter_allows_within_limit() {
        let limiter = RateLimiter::new();
        limiter
            .set_limit("test_tool", RateLimitConfig::per_second(5))
            .await;

        for _ in 0..5 {
            assert_eq!(limiter.check("test_tool").await, RateLimitResult::Allowed);
        }
    }

    #[tokio::test]
    async fn test_rate_limiter_denies_over_limit() {
        let limiter = RateLimiter::new();
        limiter
            .set_limit("test_tool", RateLimitConfig::per_second(2))
            .await;

        assert_eq!(limiter.check("test_tool").await, RateLimitResult::Allowed);
        assert_eq!(limiter.check("test_tool").await, RateLimitResult::Allowed);

        // Third request should be denied.
        match limiter.check("test_tool").await {
            RateLimitResult::Denied { retry_after } => {
                assert!(retry_after > Duration::ZERO);
            }
            RateLimitResult::Allowed => panic!("expected denied"),
        }
    }

    #[tokio::test]
    async fn test_rate_limiter_refills_over_time() {
        let limiter = RateLimiter::new();
        limiter
            .set_limit(
                "test_tool",
                RateLimitConfig::new(1, Duration::from_millis(50)),
            )
            .await;

        assert_eq!(limiter.check("test_tool").await, RateLimitResult::Allowed);

        // Should be denied immediately.
        match limiter.check("test_tool").await {
            RateLimitResult::Denied { .. } => {}
            RateLimitResult::Allowed => panic!("expected denied"),
        }

        // Wait for refill.
        tokio::time::sleep(Duration::from_millis(100)).await;

        assert_eq!(limiter.check("test_tool").await, RateLimitResult::Allowed);
    }

    #[tokio::test]
    async fn test_rate_limiter_no_limit_configured() {
        let limiter = RateLimiter::new();

        // Tool without a configured limit should always be allowed.
        for _ in 0..100 {
            assert_eq!(
                limiter.check("unconfigured").await,
                RateLimitResult::Allowed
            );
        }
    }

    #[tokio::test]
    async fn test_rate_limiter_default_config() {
        let limiter = RateLimiter::with_default(RateLimitConfig::per_second(2));

        assert_eq!(limiter.check("any_tool").await, RateLimitResult::Allowed);
        assert_eq!(limiter.check("any_tool").await, RateLimitResult::Allowed);

        match limiter.check("any_tool").await {
            RateLimitResult::Denied { .. } => {}
            RateLimitResult::Allowed => panic!("expected denied with default config"),
        }
    }

    #[tokio::test]
    async fn test_rate_limiter_per_tool_isolation() {
        let limiter = RateLimiter::new();
        limiter
            .set_limit("tool_a", RateLimitConfig::per_second(1))
            .await;
        limiter
            .set_limit("tool_b", RateLimitConfig::per_second(1))
            .await;

        assert_eq!(limiter.check("tool_a").await, RateLimitResult::Allowed);
        // tool_b should still be allowed even though tool_a is exhausted.
        assert_eq!(limiter.check("tool_b").await, RateLimitResult::Allowed);
    }

    #[tokio::test]
    async fn test_rate_limiter_remove_limit() {
        let limiter = RateLimiter::new();
        limiter
            .set_limit("test_tool", RateLimitConfig::per_second(1))
            .await;

        assert_eq!(limiter.check("test_tool").await, RateLimitResult::Allowed);
        // Rate limited now.
        match limiter.check("test_tool").await {
            RateLimitResult::Denied { .. } => {}
            _ => panic!("expected denied"),
        }

        // Remove the limit.
        limiter.remove_limit("test_tool").await;

        // Should be allowed now (no config).
        assert_eq!(limiter.check("test_tool").await, RateLimitResult::Allowed);
    }

    #[tokio::test]
    async fn test_rate_limiter_active_limits_count() {
        let limiter = RateLimiter::new();
        assert_eq!(limiter.active_limits().await, 0);

        limiter
            .set_limit("tool_a", RateLimitConfig::per_second(1))
            .await;
        assert_eq!(limiter.active_limits().await, 1);

        limiter
            .set_limit("tool_b", RateLimitConfig::per_second(1))
            .await;
        assert_eq!(limiter.active_limits().await, 2);
    }

    #[test]
    fn test_rate_limit_config_constructors() {
        let per_sec = RateLimitConfig::per_second(10);
        assert_eq!(per_sec.max_requests, 10);
        assert_eq!(per_sec.window, Duration::from_secs(1));

        let per_min = RateLimitConfig::per_minute(60);
        assert_eq!(per_min.max_requests, 60);
        assert_eq!(per_min.window, Duration::from_secs(60));
    }
}
