//! Tag-based provider routing with pluggable selection strategies.
//!
//! Design reference: providers-design.md §Tag-Based Routing

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use y_core::provider::{LlmProvider, ProviderError, RoutePriority, RouteRequest};

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
            .finish_non_exhaustive()
    }
}

/// Strategy for selecting among equally-qualified providers.
///
/// After tag/freeze filtering, this strategy determines which candidate
/// provider handles the request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SelectionStrategy {
    /// Select providers in declaration order (first match wins).
    /// This is the legacy behavior.
    #[default]
    Priority,
    /// Random selection among candidates.
    Random,
    /// Select the provider with the most available concurrency permits.
    LeastLoaded,
    /// Round-robin across candidates.
    RoundRobin,
    /// Select the cheapest provider by `cost_per_1k_input`.
    CostOptimized,
}

/// Tag-based router that selects a provider from a pool.
///
/// Routing criteria (in order):
/// 1. Provider must not be frozen
/// 2. Provider must match ALL required tags
/// 3. Preferred model gets priority if specified
/// 4. Priority-based concurrency reservation
/// 5. Selection strategy among equal candidates
pub struct TagBasedRouter {
    /// Counter for round-robin distribution.
    next_index: std::sync::atomic::AtomicUsize,
    /// The selection strategy to use.
    strategy: SelectionStrategy,
}

impl TagBasedRouter {
    /// Create a new router with default (Priority) strategy.
    pub fn new() -> Self {
        Self {
            next_index: std::sync::atomic::AtomicUsize::new(0),
            strategy: SelectionStrategy::default(),
        }
    }

    /// Create a new router with the specified selection strategy.
    pub fn with_strategy(strategy: SelectionStrategy) -> Self {
        Self {
            next_index: std::sync::atomic::AtomicUsize::new(0),
            strategy,
        }
    }

    /// Get the current selection strategy.
    pub fn strategy(&self) -> SelectionStrategy {
        self.strategy
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
        // Step 0: Explicit provider selection bypasses freeze/priority
        // prefiltering so manual user choice can retry a previously frozen
        // provider and surface the real provider error.
        if let Some(ref preferred_id) = route.preferred_provider_id {
            let Some((idx, provider)) = providers
                .iter()
                .enumerate()
                .find(|(_, p)| p.provider.metadata().id == *preferred_id)
            else {
                return Err(ProviderError::Other {
                    message: format!("preferred provider '{preferred_id}' is not registered"),
                });
            };

            let meta = provider.provider.metadata();
            if route
                .required_tags
                .iter()
                .all(|tag| meta.tags.contains(tag))
            {
                return Ok(idx);
            }

            return Err(ProviderError::Other {
                message: format!(
                    "preferred provider '{}' does not match required tags {:?}",
                    preferred_id, route.required_tags
                ),
            });
        }

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

        // Step 3: Apply selection strategy among remaining candidates.
        Ok(self.apply_strategy(providers, &candidates))
    }

    /// Apply the configured selection strategy to the candidate list.
    fn apply_strategy(&self, providers: &[RoutableProvider], candidates: &[usize]) -> usize {
        match self.strategy {
            SelectionStrategy::Priority => {
                // First candidate in declaration order.
                candidates[0]
            }
            SelectionStrategy::Random => {
                // Use a simple pseudo-random index based on the atomic counter and time.
                let seed = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .subsec_nanos() as usize;
                candidates[seed % candidates.len()]
            }
            SelectionStrategy::LeastLoaded => {
                // Select the provider with the most available permits.
                *candidates
                    .iter()
                    .max_by_key(|&&idx| providers[idx].concurrency_semaphore.available_permits())
                    .expect("candidates is non-empty")
            }
            SelectionStrategy::RoundRobin => {
                let counter = self
                    .next_index
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                candidates[counter % candidates.len()]
            }
            SelectionStrategy::CostOptimized => {
                // Select the cheapest provider by input token cost.
                *candidates
                    .iter()
                    .min_by(|&&a, &&b| {
                        let cost_a = providers[a].provider.metadata().cost_per_1k_input;
                        let cost_b = providers[b].provider.metadata().cost_per_1k_input;
                        cost_a
                            .partial_cmp(&cost_b)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .expect("candidates is non-empty")
            }
        }
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
                    capabilities: vec![ProviderCapability::Text],
                    max_concurrency: 5,
                    context_window: 128_000,
                    cost_per_1k_input: 0.01,
                    cost_per_1k_output: 0.03,
                    tool_calling_mode: ToolCallingMode::default(),
                },
            }
        }

        fn with_cost(id: &str, model: &str, tags: Vec<&str>, cost_input: f64) -> Self {
            Self {
                meta: ProviderMetadata {
                    id: y_core::types::ProviderId::from_string(id),
                    provider_type: ProviderType::OpenAi,
                    model: model.into(),
                    tags: tags.into_iter().map(String::from).collect(),
                    capabilities: vec![ProviderCapability::Text],
                    max_concurrency: 5,
                    context_window: 128_000,
                    cost_per_1k_input: cost_input,
                    cost_per_1k_output: 0.03,
                    tool_calling_mode: ToolCallingMode::default(),
                },
            }
        }
    }

    #[async_trait]
    impl LlmProvider for MockProvider {
        async fn chat_completion(
            &self,
            _request: &ChatRequest,
        ) -> Result<ChatResponse, ProviderError> {
            panic!("MockProvider::chat_completion should not be called in router tests")
        }
        async fn chat_completion_stream(
            &self,
            _request: &ChatRequest,
        ) -> Result<ChatStreamResponse, ProviderError> {
            panic!("MockProvider::chat_completion_stream should not be called in router tests")
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

    fn make_routable_with_cost(
        id: &str,
        model: &str,
        tags: Vec<&str>,
        cost: f64,
    ) -> RoutableProvider {
        RoutableProvider {
            provider: Arc::new(MockProvider::with_cost(id, model, tags, cost)),
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

    // -----------------------------------------------------------------------
    // Existing tests (adapted for backward compatibility)
    // -----------------------------------------------------------------------

    #[test]
    fn test_routing_selects_by_single_tag() {
        let router = TagBasedRouter::with_strategy(SelectionStrategy::Priority);
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
        let router = TagBasedRouter::with_strategy(SelectionStrategy::Priority);
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
        // Priority strategy selects first matching candidate (p2 at index 1).
        assert_eq!(idx, 1);
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
        assert!(matches!(
            result,
            Err(ProviderError::NoProviderAvailable { .. })
        ));
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
    fn test_routing_preferred_provider_bypasses_freeze() {
        let router = TagBasedRouter::new();
        let providers = vec![
            make_frozen_routable("p1", "seedream", vec!["image"]),
            make_routable("p2", "gpt-4o", vec!["general"]),
        ];

        let route = RouteRequest {
            preferred_provider_id: Some(y_core::types::ProviderId::from_string("p1")),
            ..Default::default()
        };

        let idx = router.select(&providers, &route).unwrap();
        assert_eq!(idx, 0, "preferred provider should bypass freeze prefilter");
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
        let router = TagBasedRouter::with_strategy(SelectionStrategy::RoundRobin);
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
        assert!(
            selections.len() >= 2,
            "should distribute load: {selections:?}"
        );
    }

    #[test]
    fn test_routing_idle_priority_defers_when_busy() {
        let router = TagBasedRouter::new();
        // Create a provider with 0 available permits.
        let rp = RoutableProvider {
            provider: Arc::new(MockProvider::new("p1", "gpt-4", vec!["gen"])),
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
        assert!(matches!(
            result,
            Err(ProviderError::NoProviderAvailable { .. })
        ));
    }

    // -----------------------------------------------------------------------
    // Selection strategy specific tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_strategy_priority_selects_first() {
        let router = TagBasedRouter::with_strategy(SelectionStrategy::Priority);
        let providers = vec![
            make_routable("p1", "gpt-4", vec!["gen"]),
            make_routable("p2", "gpt-4", vec!["gen"]),
            make_routable("p3", "gpt-4", vec!["gen"]),
        ];

        let route = RouteRequest {
            required_tags: vec!["gen".into()],
            ..Default::default()
        };

        // Priority always returns the first candidate.
        for _ in 0..5 {
            let idx = router.select(&providers, &route).unwrap();
            assert_eq!(
                idx, 0,
                "Priority strategy should always select first candidate"
            );
        }
    }

    #[test]
    fn test_strategy_least_loaded() {
        let router = TagBasedRouter::with_strategy(SelectionStrategy::LeastLoaded);
        // p1 has 1 permit, p2 has 10 permits => should pick p2.
        let providers = vec![
            RoutableProvider {
                provider: Arc::new(MockProvider::new("p1", "gpt-4", vec!["gen"])),
                freeze_manager: Arc::new(FreezeManager::new(30, 3600)),
                concurrency_semaphore: Arc::new(tokio::sync::Semaphore::new(1)),
                max_concurrency: 10,
            },
            RoutableProvider {
                provider: Arc::new(MockProvider::new("p2", "gpt-4", vec!["gen"])),
                freeze_manager: Arc::new(FreezeManager::new(30, 3600)),
                concurrency_semaphore: Arc::new(tokio::sync::Semaphore::new(10)),
                max_concurrency: 10,
            },
        ];

        let route = RouteRequest {
            required_tags: vec!["gen".into()],
            ..Default::default()
        };

        let idx = router.select(&providers, &route).unwrap();
        assert_eq!(idx, 1, "LeastLoaded should select p2 with more permits");
    }

    #[test]
    fn test_strategy_cost_optimized() {
        let router = TagBasedRouter::with_strategy(SelectionStrategy::CostOptimized);
        let providers = vec![
            make_routable_with_cost("p1", "gpt-4", vec!["gen"], 0.03),
            make_routable_with_cost("p2", "gpt-4o-mini", vec!["gen"], 0.001),
            make_routable_with_cost("p3", "claude", vec!["gen"], 0.015),
        ];

        let route = RouteRequest {
            required_tags: vec!["gen".into()],
            ..Default::default()
        };

        let idx = router.select(&providers, &route).unwrap();
        assert_eq!(idx, 1, "CostOptimized should select cheapest provider p2");
    }

    #[test]
    fn test_strategy_round_robin_distribution() {
        let router = TagBasedRouter::with_strategy(SelectionStrategy::RoundRobin);
        let providers = vec![
            make_routable("p1", "gpt-4", vec!["gen"]),
            make_routable("p2", "gpt-4", vec!["gen"]),
        ];

        let route = RouteRequest {
            required_tags: vec!["gen".into()],
            ..Default::default()
        };

        let first = router.select(&providers, &route).unwrap();
        let second = router.select(&providers, &route).unwrap();
        // Two consecutive calls should select different providers.
        assert_ne!(first, second, "RoundRobin should alternate providers");
    }

    #[test]
    fn test_strategy_serialization() {
        assert_eq!(
            serde_json::to_string(&SelectionStrategy::Priority).unwrap(),
            "\"priority\""
        );
        assert_eq!(
            serde_json::to_string(&SelectionStrategy::LeastLoaded).unwrap(),
            "\"least_loaded\""
        );
        assert_eq!(
            serde_json::to_string(&SelectionStrategy::CostOptimized).unwrap(),
            "\"cost_optimized\""
        );
        assert_eq!(
            serde_json::to_string(&SelectionStrategy::RoundRobin).unwrap(),
            "\"round_robin\""
        );
        assert_eq!(
            serde_json::to_string(&SelectionStrategy::Random).unwrap(),
            "\"random\""
        );
    }

    #[test]
    fn test_strategy_deserialization() {
        let s: SelectionStrategy = serde_json::from_str("\"least_loaded\"").unwrap();
        assert_eq!(s, SelectionStrategy::LeastLoaded);

        let s: SelectionStrategy = serde_json::from_str("\"cost_optimized\"").unwrap();
        assert_eq!(s, SelectionStrategy::CostOptimized);
    }
}
