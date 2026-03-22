//! Concurrency controller: limits global and per-resource concurrent tasks.
//!
//! Design reference: orchestrator-design.md, Performance

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;

/// Type of resource that a task consumes.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceType {
    /// LLM provider calls.
    Llm,
    /// Network/HTTP requests.
    Network,
    /// Compute-intensive operations.
    Compute,
    /// Custom named resource.
    Custom(String),
}

/// Controls concurrency at global and per-resource levels.
#[derive(Debug)]
pub struct ConcurrencyController {
    /// Global limit across all tasks.
    global: Arc<Semaphore>,
    /// Per-resource limits.
    resource_limits: HashMap<ResourceType, Arc<Semaphore>>,
}

impl ConcurrencyController {
    /// Create a controller with a global limit and no resource-specific limits.
    pub fn new(global_limit: usize) -> Self {
        Self {
            global: Arc::new(Semaphore::new(global_limit)),
            resource_limits: HashMap::new(),
        }
    }

    /// Add a per-resource concurrency limit.
    pub fn add_resource_limit(&mut self, resource: ResourceType, limit: usize) {
        self.resource_limits
            .insert(resource, Arc::new(Semaphore::new(limit)));
    }

    /// Get the global semaphore.
    pub fn global(&self) -> &Arc<Semaphore> {
        &self.global
    }

    /// Get the semaphore for a specific resource type.
    pub fn resource(&self, resource: &ResourceType) -> Option<&Arc<Semaphore>> {
        self.resource_limits.get(resource)
    }

    /// Number of resource types with limits.
    pub fn resource_count(&self) -> usize {
        self.resource_limits.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// T-P3-05: Concurrency controller limits global concurrent tasks.
    #[tokio::test]
    async fn test_global_semaphore_limits() {
        let controller = ConcurrencyController::new(2);
        let sem = controller.global().clone();

        // Acquire 2 permits (should succeed).
        let p1 = sem.acquire().await.unwrap();
        let p2 = sem.acquire().await.unwrap();
        assert_eq!(sem.available_permits(), 0);

        // Release one permit.
        drop(p1);
        assert_eq!(sem.available_permits(), 1);
        drop(p2);
        assert_eq!(sem.available_permits(), 2);
    }

    /// T-P3-06: Concurrency controller limits per-resource tasks.
    #[tokio::test]
    async fn test_resource_semaphore_limits() {
        let mut controller = ConcurrencyController::new(10);
        controller.add_resource_limit(ResourceType::Llm, 2);
        controller.add_resource_limit(ResourceType::Network, 5);

        let llm_sem = controller.resource(&ResourceType::Llm).unwrap().clone();
        assert_eq!(llm_sem.available_permits(), 2);

        let net_sem = controller.resource(&ResourceType::Network).unwrap().clone();
        assert_eq!(net_sem.available_permits(), 5);

        assert!(controller.resource(&ResourceType::Compute).is_none());
        assert_eq!(controller.resource_count(), 2);
    }

    /// Custom resource type works.
    #[test]
    fn test_custom_resource_type() {
        let mut controller = ConcurrencyController::new(10);
        controller.add_resource_limit(ResourceType::Custom("gpu".into()), 1);

        assert!(controller
            .resource(&ResourceType::Custom("gpu".into()))
            .is_some());
        assert!(controller
            .resource(&ResourceType::Custom("cpu".into()))
            .is_none());
    }
}
