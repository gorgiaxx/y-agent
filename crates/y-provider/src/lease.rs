//! Request lease management for provider request lifecycle tracking.
//!
//! A lease represents an active LLM request. It tracks:
//! - When the request started
//! - Which provider is handling it
//! - How long the request has been active
//! - Automatic cleanup on lease drop
//!
//! Design reference: providers-design.md §Concurrency Management

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;
use uuid::Uuid;

/// Unique identifier for a request lease.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LeaseId(pub String);

impl LeaseId {
    /// Create a new unique lease ID.
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }
}

impl Default for LeaseId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for LeaseId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Information about an active request lease.
#[derive(Debug, Clone)]
pub struct LeaseInfo {
    /// Unique lease identifier.
    pub id: LeaseId,
    /// Provider handling this request.
    pub provider_id: String,
    /// When the lease was acquired.
    pub acquired_at: Instant,
    /// Optional description for debugging.
    pub description: Option<String>,
}

impl LeaseInfo {
    /// How long this lease has been active.
    pub fn elapsed(&self) -> Duration {
        self.acquired_at.elapsed()
    }
}

/// Manages active request leases with automatic expiry detection.
///
/// Thread-safe: all operations use an internal lock.
#[derive(Debug, Clone)]
pub struct LeaseManager {
    inner: Arc<Mutex<LeaseManagerInner>>,
    /// Maximum lease duration before it's considered expired.
    max_lease_duration: Duration,
}

#[derive(Debug)]
struct LeaseManagerInner {
    active_leases: HashMap<LeaseId, LeaseInfo>,
}

impl LeaseManager {
    /// Create a new lease manager.
    ///
    /// `max_lease_duration` defines when a lease is considered expired/stale.
    pub fn new(max_lease_duration: Duration) -> Self {
        Self {
            inner: Arc::new(Mutex::new(LeaseManagerInner {
                active_leases: HashMap::new(),
            })),
            max_lease_duration,
        }
    }

    /// Acquire a new lease for a request to the given provider.
    ///
    /// Returns a `LeaseGuard` that automatically releases the lease on drop.
    pub async fn acquire(&self, provider_id: &str, description: Option<String>) -> LeaseGuard {
        let lease_id = LeaseId::new();
        let info = LeaseInfo {
            id: lease_id.clone(),
            provider_id: provider_id.to_string(),
            acquired_at: Instant::now(),
            description,
        };

        {
            let mut inner = self.inner.lock().await;
            inner.active_leases.insert(lease_id.clone(), info);
        }

        LeaseGuard {
            lease_id,
            manager: self.clone(),
        }
    }

    /// Release a lease by ID. Usually called automatically by `LeaseGuard::drop`.
    pub async fn release(&self, lease_id: &LeaseId) {
        let mut inner = self.inner.lock().await;
        inner.active_leases.remove(lease_id);
    }

    /// Get the number of currently active leases.
    pub async fn active_count(&self) -> usize {
        let inner = self.inner.lock().await;
        inner.active_leases.len()
    }

    /// Get the number of active leases for a specific provider.
    pub async fn active_count_for_provider(&self, provider_id: &str) -> usize {
        let inner = self.inner.lock().await;
        inner
            .active_leases
            .values()
            .filter(|l| l.provider_id == provider_id)
            .count()
    }

    /// Check for and return any expired leases (exceeded `max_lease_duration`).
    ///
    /// Does NOT automatically remove them — caller decides whether to force-release.
    pub async fn find_expired(&self) -> Vec<LeaseInfo> {
        let inner = self.inner.lock().await;
        inner
            .active_leases
            .values()
            .filter(|l| l.elapsed() > self.max_lease_duration)
            .cloned()
            .collect()
    }

    /// Force-release all expired leases and return how many were cleaned up.
    pub async fn cleanup_expired(&self) -> usize {
        let mut inner = self.inner.lock().await;
        let max_duration = self.max_lease_duration;
        let before = inner.active_leases.len();
        inner
            .active_leases
            .retain(|_, v| v.acquired_at.elapsed() <= max_duration);
        before - inner.active_leases.len()
    }

    /// Get a snapshot of all active leases.
    pub async fn snapshot(&self) -> Vec<LeaseInfo> {
        let inner = self.inner.lock().await;
        inner.active_leases.values().cloned().collect()
    }
}

/// RAII guard that releases the lease when dropped.
///
/// The lease is automatically released when this guard is dropped,
/// either by going out of scope or explicit `.release()`.
pub struct LeaseGuard {
    lease_id: LeaseId,
    manager: LeaseManager,
}

impl LeaseGuard {
    /// Get the lease ID.
    pub fn id(&self) -> &LeaseId {
        &self.lease_id
    }

    /// Explicitly release the lease and consume the guard.
    ///
    /// If you simply drop the guard, the lease will be released in the
    /// background (via `tokio::spawn`).
    pub async fn release(self) {
        self.manager.release(&self.lease_id).await;
        // Prevent Drop from also releasing.
        std::mem::forget(self);
    }
}

impl Drop for LeaseGuard {
    fn drop(&mut self) {
        let lease_id = self.lease_id.clone();
        let manager = self.manager.clone();
        // Spawn a background task to release the lease since Drop is synchronous.
        tokio::spawn(async move {
            manager.release(&lease_id).await;
        });
    }
}

impl std::fmt::Debug for LeaseGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LeaseGuard")
            .field("lease_id", &self.lease_id)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_lease_acquire_release() {
        let manager = LeaseManager::new(Duration::from_secs(60));

        let guard = manager.acquire("openai-main", None).await;
        assert_eq!(manager.active_count().await, 1);

        guard.release().await;
        assert_eq!(manager.active_count().await, 0);
    }

    #[tokio::test]
    async fn test_lease_multiple_providers() {
        let manager = LeaseManager::new(Duration::from_secs(60));

        let _g1 = manager.acquire("openai-main", None).await;
        let _g2 = manager.acquire("openai-main", None).await;
        let _g3 = manager.acquire("anthropic-main", None).await;

        assert_eq!(manager.active_count().await, 3);
        assert_eq!(manager.active_count_for_provider("openai-main").await, 2);
        assert_eq!(manager.active_count_for_provider("anthropic-main").await, 1);
    }

    #[tokio::test]
    async fn test_lease_snapshot() {
        let manager = LeaseManager::new(Duration::from_secs(60));

        let _g1 = manager
            .acquire("openai-main", Some("chat request".into()))
            .await;

        let snapshot = manager.snapshot().await;
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].provider_id, "openai-main");
        assert_eq!(snapshot[0].description.as_deref(), Some("chat request"));
    }

    #[tokio::test]
    async fn test_lease_cleanup_expired() {
        let manager = LeaseManager::new(Duration::from_millis(50));

        let _g1 = manager.acquire("openai-main", None).await;

        // Wait for the lease to expire.
        tokio::time::sleep(Duration::from_millis(100)).await;

        let expired = manager.find_expired().await;
        assert_eq!(expired.len(), 1);

        let cleaned = manager.cleanup_expired().await;
        assert_eq!(cleaned, 1);
        assert_eq!(manager.active_count().await, 0);
    }

    #[tokio::test]
    async fn test_lease_not_expired() {
        let manager = LeaseManager::new(Duration::from_secs(60));

        let _g1 = manager.acquire("openai-main", None).await;

        let expired = manager.find_expired().await;
        assert!(expired.is_empty());

        let cleaned = manager.cleanup_expired().await;
        assert_eq!(cleaned, 0);
    }

    #[tokio::test]
    async fn test_lease_elapsed() {
        let manager = LeaseManager::new(Duration::from_secs(60));

        let guard = manager.acquire("test", None).await;

        tokio::time::sleep(Duration::from_millis(50)).await;

        let snapshot = manager.snapshot().await;
        assert!(snapshot[0].elapsed() >= Duration::from_millis(40));

        guard.release().await;
    }

    #[tokio::test]
    async fn test_lease_id_uniqueness() {
        let id1 = LeaseId::new();
        let id2 = LeaseId::new();
        assert_ne!(id1, id2);
    }
}
