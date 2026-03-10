//! Tag-based provider routing.

use std::sync::Arc;

use y_core::provider::{LlmProvider, ProviderError, RouteRequest, RoutePriority};

use crate::freeze::FreezeManager;

/// Entry representing a provider in the routing table.
pub struct RoutableProvider {
    pub provider: Arc<dyn LlmProvider>,
    pub freeze_manager: Arc<FreezeManager>,
    pub concurrency_semaphore: Arc<tokio::sync::Semaphore>,
    pub max_concurrency: usize,
}

impl std::fmt::Debug for RoutableProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RoutableProvider")
            .field("id", &self.provider.metadata().id)
            .field("frozen", &self.freeze_manager.is_frozen())
            .finish()
    }
}

/// Tag-based router that selects a provider from a pool.
///
/// Routing criteria (in order):
/// 1. Provider must not be frozen
/// 2. Provider must match ALL required tags
/// 3. Preferred model gets priority if specified
/// 4. Priority-based concurrency reservation
/// 5. Round-robin among equal candidates
pub struct TagBasedRouter {
    /// Counter for round-robin distribution.
    next_index: std::sync::atomic::AtomicUsize,
}

impl TagBasedRouter {
    /// Create a new router.
    pub fn new() -> Self {
        Self {
            next_index: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// Select the best provider for the given route request.
    ///
    /// Returns an index into the providers list, or an error if no provider
    /// matches the criteria.
    pub fn select(
        &self,
        providers: &[RoutableProvider],
        route: &RouteRequest,
    ) -> Result<usize, ProviderError> {
        // Step 1: Filter to non-frozen providers matching all required tags.
        let candidates: Vec<usize> = providers
            .iter()
            .enumerate()
            .filter(|(_, p)| !p.freeze_manager.is_frozen())
            .filter(|(_, p)| {
                let meta = p.provider.metadata();
                route
                    .required_tags
                    .iter()
                    .all(|tag| meta.tags.contains(tag))
            })
            .filter(|(_, p)| {
                // Priority-based filtering: idle requests are rejected when at capacity.
                match route.priority {
                    RoutePriority::Idle => {
                        let available = p.concurrency_semaphore.available_permits();
                        available > 0
                    }
                    RoutePriority::Critical => {
                        // Critical requests can use reserved capacity (last 20%).
                        true
                    }
                    RoutePriority::Normal => {
                        let available = p.concurrency_semaphore.available_permits();
                        let reserved = p.max_concurrency / 5; // 20% reserved for critical
                        available > reserved
                    }
                }
            })
            .map(|(i, _)| i)
            .collect();

        if candidates.is_empty() {
            return Err(ProviderError::NoProviderAvailable {
                tags: route.required_tags.clone(),
            });
        }

        // Step 2: Prefer exact model match if specified.
        if let Some(ref preferred) = route.preferred_model {
            for &idx in &candidates {
                if providers[idx].provider.metadata().model == *preferred {
                    return Ok(idx);
                }
            }
        }

        // Step 3: Round-robin among remaining candidates.
        let counter = self
            .next_index
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let idx = candidates[counter % candidates.len()];

        Ok(idx)
    }
}

impl Default for TagBasedRouter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::freeze::FreezeManager;
    use async_trait::async_trait;
    use std::sync::Arc;
    use y_core::provider::*;

    /// Mock provider for testing routing.
    struct MockProvider {
        meta: ProviderMetadata,
    }

    impl MockProvider {
        fn new(id: &str, model: &str, tags: Vec<&str>) -> Self {
            Self {
                meta: ProviderMetadata {
                    id: y_core::types::ProviderId::from_string(id),
                    provider_type: ProviderType::OpenAi,
                    model: model.into(),
                    tags: tags.into_iter().map(String::from).collect(),
                    max_concurrency: 5,
                    context_window: 128_000,
                    cost_per_1k_input: 0.01,
                    cost_per_1k_output: 0.03,
                },
            }
        }
    }

    #[async_trait]
    impl LlmProvider for MockProvider {
        async fn chat_completion(&self, _request: &ChatRequest) -> Result<ChatResponse, ProviderError> {
            unimplemented!("mock")
        }
        async fn chat_completion_stream(&self, _request: &ChatRequest) -> Result<ChatStream, ProviderError> {
            unimplemented!("mock")
        }
        fn metadata(&self) -> &ProviderMetadata {
            &self.meta
        }
    }

    fn make_routable(id: &str, model: &str, tags: Vec<&str>) -> RoutableProvider {
        RoutableProvider {
            provider: Arc::new(MockProvider::new(id, model, tags)),
            freeze_manager: Arc::new(FreezeManager::new(30, 3600)),
            concurrency_semaphore: Arc::new(tokio::sync::Semaphore::new(5)),
            max_concurrency: 5,
        }
    }

    fn make_frozen_routable(id: &str, model: &str, tags: Vec<&str>) -> RoutableProvider {
        let rp = make_routable(id, model, tags);
        rp.freeze_manager.freeze("test freeze".into(), None);
        rp
    }

    #[test]
    fn test_routing_selects_by_single_tag() {
        let router = TagBasedRouter::new();
        let providers = vec![
            make_routable("p1", "gpt-4", vec!["reasoning"]),
            make_routable("p2", "gpt-3.5", vec!["fast"]),
        ];

        let route = RouteRequest {
            required_tags: vec!["reasoning".into()],
            ..Default::default()
        };

        let idx = router.select(&providers, &route).unwrap();
        assert_eq!(idx, 0);
    }

    #[test]
    fn test_routing_selects_by_multiple_tags() {
        let router = TagBasedRouter::new();
        let providers = vec![
            make_routable("p1", "gpt-4", vec!["reasoning"]),
            make_routable("p2", "claude", vec!["fast", "code"]),
            make_routable("p3", "gpt-4o", vec!["fast", "code", "reasoning"]),
        ];

        let route = RouteRequest {
            required_tags: vec!["fast".into(), "code".into()],
            ..Default::default()
        };

        let idx = router.select(&providers, &route).unwrap();
        // Both p2 and p3 match, round-robin selects first candidate.
        assert!(idx == 1 || idx == 2);
    }

    #[test]
    fn test_routing_no_match_returns_error() {
        let router = TagBasedRouter::new();
        let providers = vec![make_routable("p1", "gpt-4", vec!["reasoning"])];

        let route = RouteRequest {
            required_tags: vec!["nonexistent".into()],
            ..Default::default()
        };

        let result = router.select(&providers, &route);
        assert!(matches!(result, Err(ProviderError::NoProviderAvailable { .. })));
    }

    #[test]
    fn test_routing_skips_frozen_providers() {
        let router = TagBasedRouter::new();
        let providers = vec![
            make_frozen_routable("p1", "gpt-4", vec!["reasoning"]),
            make_routable("p2", "claude", vec!["reasoning"]),
        ];

        let route = RouteRequest {
            required_tags: vec!["reasoning".into()],
            ..Default::default()
        };

        let idx = router.select(&providers, &route).unwrap();
        assert_eq!(idx, 1, "should skip frozen p1 and select p2");
    }

    #[test]
    fn test_routing_preferred_model_exact_match() {
        let router = TagBasedRouter::new();
        let providers = vec![
            make_routable("p1", "gpt-4", vec!["reasoning"]),
            make_routable("p2", "gpt-4o", vec!["reasoning"]),
        ];

        let route = RouteRequest {
            required_tags: vec!["reasoning".into()],
            preferred_model: Some("gpt-4o".into()),
            ..Default::default()
        };

        let idx = router.select(&providers, &route).unwrap();
        assert_eq!(idx, 1, "should prefer exact model match");
    }

    #[test]
    fn test_routing_round_robin_among_equal_candidates() {
        let router = TagBasedRouter::new();
        let providers = vec![
            make_routable("p1", "gpt-4", vec!["general"]),
            make_routable("p2", "gpt-4", vec!["general"]),
            make_routable("p3", "gpt-4", vec!["general"]),
        ];

        let route = RouteRequest {
            required_tags: vec!["general".into()],
            ..Default::default()
        };

        let mut selections = std::collections::HashSet::new();
        for _ in 0..6 {
            let idx = router.select(&providers, &route).unwrap();
            selections.insert(idx);
        }
        // Should have distributed across at least 2 of the 3.
        assert!(selections.len() >= 2, "should distribute load: {selections:?}");
    }

    #[test]
    fn test_routing_idle_priority_defers_when_busy() {
        let router = TagBasedRouter::new();
        // Create a provider with 0 available permits.
        let rp = RoutableProvider {
            provider: Arc::new(MockProvider::new("p1", "gpt-4", vec!["gen".into()])),
            freeze_manager: Arc::new(FreezeManager::new(30, 3600)),
            concurrency_semaphore: Arc::new(tokio::sync::Semaphore::new(0)),
            max_concurrency: 5,
        };
        let providers = vec![rp];

        let route = RouteRequest {
            required_tags: vec!["gen".into()],
            priority: RoutePriority::Idle,
            ..Default::default()
        };

        let result = router.select(&providers, &route);
        assert!(matches!(result, Err(ProviderError::NoProviderAvailable { .. })));
    }
}
