//! Provider pool implementation — the main ProviderPool trait impl.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tracing::instrument;

use y_core::provider::{
    ChatRequest, ChatResponse, ChatStream, LlmProvider, ProviderError, ProviderPool,
    ProviderStatus, RouteRequest,
};
use y_core::types::ProviderId;

use crate::config::ProviderPoolConfig;


use crate::freeze::FreezeManager;
use crate::health::HealthChecker;
use crate::metrics::{ProviderMetrics, SharedMetrics};
use crate::router::{RoutableProvider, TagBasedRouter};

/// Concrete implementation of the `ProviderPool` trait.
///
/// Manages a set of LLM providers with tag-based routing, freeze/thaw,
/// per-provider concurrency limits, and metrics tracking.
pub struct ProviderPoolImpl {
    providers: Vec<ProviderEntry>,
    router: TagBasedRouter,
    health_checker: HealthChecker,
}

/// An entry in the pool combining provider, freeze state, semaphore, and metrics.
struct ProviderEntry {
    provider: Arc<dyn LlmProvider>,
    freeze_manager: Arc<FreezeManager>,
    semaphore: Arc<tokio::sync::Semaphore>,
    max_concurrency: usize,
    metrics: SharedMetrics,
}

impl std::fmt::Debug for ProviderPoolImpl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderPoolImpl")
            .field("provider_count", &self.providers.len())
            .finish()
    }
}

impl ProviderPoolImpl {
    /// Create a new pool from a list of pre-constructed providers.
    ///
    /// Use this for testing or when providers are built externally.
    pub fn from_providers(
        providers: Vec<Arc<dyn LlmProvider>>,
        config: &ProviderPoolConfig,
    ) -> Self {
        let entries: Vec<ProviderEntry> = providers
            .into_iter()
            .map(|p| {
                let max_conc = p.metadata().max_concurrency;
                ProviderEntry {
                    provider: p,
                    freeze_manager: Arc::new(FreezeManager::new(
                        config.default_freeze_duration_secs,
                        config.max_freeze_duration_secs,
                    )),
                    semaphore: Arc::new(tokio::sync::Semaphore::new(max_conc)),
                    max_concurrency: max_conc,
                    metrics: Arc::new(ProviderMetrics::new()),
                }
            })
            .collect();

        Self {
            providers: entries,
            router: TagBasedRouter::new(),
            health_checker: HealthChecker::new(Duration::from_secs(10)),
        }
    }

    /// Build the routable providers list for the router.
    fn routable_providers(&self) -> Vec<RoutableProvider> {
        self.providers
            .iter()
            .map(|e| RoutableProvider {
                provider: Arc::clone(&e.provider),
                freeze_manager: Arc::clone(&e.freeze_manager),
                concurrency_semaphore: Arc::clone(&e.semaphore),
                max_concurrency: e.max_concurrency,
            })
            .collect()
    }

    /// Find an entry by provider ID.
    fn find_entry(&self, provider_id: &ProviderId) -> Option<&ProviderEntry> {
        self.providers
            .iter()
            .find(|e| e.provider.metadata().id == *provider_id)
    }

    /// Get metrics for a specific provider.
    pub fn provider_metrics(&self, provider_id: &ProviderId) -> Option<SharedMetrics> {
        self.find_entry(provider_id)
            .map(|e| Arc::clone(&e.metrics))
    }
}

#[async_trait]
impl ProviderPool for ProviderPoolImpl {
    #[instrument(skip(self, request), fields(tags = ?route.required_tags))]
    async fn chat_completion(
        &self,
        request: &ChatRequest,
        route: &RouteRequest,
    ) -> Result<ChatResponse, ProviderError> {
        let routable = self.routable_providers();
        let idx = self.router.select(&routable, route)?;
        let entry = &self.providers[idx];

        // Acquire semaphore permit for concurrency control.
        let _permit = entry
            .semaphore
            .acquire()
            .await
            .map_err(|_| ProviderError::Other {
                message: "semaphore closed".into(),
            })?;

        let result = entry.provider.chat_completion(request).await;

        match &result {
            Ok(response) => {
                entry
                    .metrics
                    .record_success(response.usage.input_tokens, response.usage.output_tokens);
            }
            Err(e) => {
                entry.metrics.record_error();
                self.report_error(&entry.provider.metadata().id, e);
            }
        }

        result
    }

    #[instrument(skip(self, request), fields(tags = ?route.required_tags))]
    async fn chat_completion_stream(
        &self,
        request: &ChatRequest,
        route: &RouteRequest,
    ) -> Result<ChatStream, ProviderError> {
        let routable = self.routable_providers();
        let idx = self.router.select(&routable, route)?;
        let entry = &self.providers[idx];

        let _permit = entry
            .semaphore
            .acquire()
            .await
            .map_err(|_| ProviderError::Other {
                message: "semaphore closed".into(),
            })?;

        // For streaming, we record the request but can't easily track tokens
        // until the stream completes. Token tracking for streams happens
        // at the caller level (the orchestrator reads the final chunk).
        entry.metrics.record_success(0, 0);

        entry.provider.chat_completion_stream(request).await
    }

    fn report_error(&self, provider_id: &ProviderId, error: &ProviderError) {
        if let Some(entry) = self.find_entry(provider_id) {
            use y_core::error::ErrorSeverity;
            match error.severity() {
                ErrorSeverity::Permanent => {
                    entry
                        .freeze_manager
                        .freeze_permanent(format!("{error}"));
                    tracing::warn!(
                        provider_id = %provider_id,
                        error = %error,
                        "provider permanently frozen"
                    );
                }
                ErrorSeverity::Transient => {
                    if let ProviderError::RateLimited {
                        retry_after_secs, ..
                    } = error
                    {
                        entry.freeze_manager.freeze_with_retry_after(
                            format!("{error}"),
                            Duration::from_secs(*retry_after_secs),
                        );
                    } else {
                        entry
                            .freeze_manager
                            .freeze(format!("{error}"), None);
                    }
                    tracing::info!(
                        provider_id = %provider_id,
                        error = %error,
                        "provider frozen (transient error)"
                    );
                }
                ErrorSeverity::UserActionRequired => {
                    tracing::error!(
                        provider_id = %provider_id,
                        error = %error,
                        "provider error requires user action"
                    );
                }
            }
        }
    }

    async fn provider_statuses(&self) -> Vec<ProviderStatus> {
        self.providers
            .iter()
            .map(|entry| {
                let meta = entry.provider.metadata();
                let freeze_status = entry.freeze_manager.status();
                let metrics = entry.metrics.snapshot();

                ProviderStatus {
                    id: meta.id.clone(),
                    is_frozen: freeze_status.is_frozen,
                    frozen_since: None, // Instant doesn't convert directly to Timestamp
                    thaw_at: None,
                    freeze_reason: freeze_status.reason,
                    active_requests: entry.max_concurrency
                        - entry.semaphore.available_permits(),
                    total_requests: metrics.total_requests,
                    total_errors: metrics.total_errors,
                }
            })
            .collect()
    }

    async fn freeze(&self, provider_id: &ProviderId, reason: String) {
        if let Some(entry) = self.find_entry(provider_id) {
            entry.freeze_manager.freeze(reason, None);
        }
    }

    async fn thaw(&self, provider_id: &ProviderId) -> Result<(), ProviderError> {
        let entry = self
            .find_entry(provider_id)
            .ok_or_else(|| ProviderError::Other {
                message: format!("provider not found: {provider_id}"),
            })?;

        self.health_checker
            .check_and_thaw(&entry.provider, &entry.freeze_manager)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use y_core::provider::*;
    use y_core::types::TokenUsage;

    struct MockProvider {
        meta: ProviderMetadata,
        should_fail: bool,
    }

    impl MockProvider {
        fn ok(id: &str, tags: Vec<&str>) -> Arc<dyn LlmProvider> {
            Arc::new(Self {
                meta: ProviderMetadata {
                    id: ProviderId::from_string(id),
                    provider_type: ProviderType::OpenAi,
                    model: "test-model".into(),
                    tags: tags.into_iter().map(String::from).collect(),
                    max_concurrency: 5,
                    context_window: 128_000,
                    cost_per_1k_input: 0.01,
                    cost_per_1k_output: 0.03,
                },
                should_fail: false,
            })
        }

        fn failing(id: &str, tags: Vec<&str>) -> Arc<dyn LlmProvider> {
            Arc::new(Self {
                meta: ProviderMetadata {
                    id: ProviderId::from_string(id),
                    provider_type: ProviderType::OpenAi,
                    model: "test-model".into(),
                    tags: tags.into_iter().map(String::from).collect(),
                    max_concurrency: 5,
                    context_window: 128_000,
                    cost_per_1k_input: 0.01,
                    cost_per_1k_output: 0.03,
                },
                should_fail: true,
            })
        }
    }

    #[async_trait]
    impl LlmProvider for MockProvider {
        async fn chat_completion(
            &self,
            _request: &ChatRequest,
        ) -> Result<ChatResponse, ProviderError> {
            if self.should_fail {
                return Err(ProviderError::ServerError {
                    provider: self.meta.id.to_string(),
                    message: "mock failure".into(),
                });
            }
            Ok(ChatResponse {
                id: "resp-1".into(),
                model: self.meta.model.clone(),
                content: Some("test response".into()),
                tool_calls: vec![],
                usage: TokenUsage {
                    input_tokens: 10,
                    output_tokens: 5,
                    cache_read_tokens: None,
                    cache_write_tokens: None,
                },
                finish_reason: FinishReason::Stop,
            })
        }

        async fn chat_completion_stream(
            &self,
            _request: &ChatRequest,
        ) -> Result<ChatStream, ProviderError> {
            unimplemented!("mock streaming")
        }

        fn metadata(&self) -> &ProviderMetadata {
            &self.meta
        }
    }

    fn test_config() -> ProviderPoolConfig {
        ProviderPoolConfig {
            providers: vec![],
            default_freeze_duration_secs: 30,
            max_freeze_duration_secs: 3600,
            health_check_interval_secs: 60,
        }
    }

    fn test_request() -> ChatRequest {
        ChatRequest {
            messages: vec![y_core::types::Message {
                role: y_core::types::Role::User,
                content: "test".into(),
                tool_call_id: None,
                tool_calls: vec![],
                timestamp: chrono::Utc::now(),
                metadata: serde_json::Value::Null,
            }],
            model: None,
            max_tokens: Some(100),
            temperature: None,
            tools: vec![],
            stop: vec![],
            extra: serde_json::Value::Null,
        }
    }

    #[tokio::test]
    async fn test_concurrency_semaphore_release_on_completion() {
        let pool = ProviderPoolImpl::from_providers(
            vec![MockProvider::ok("p1", vec!["gen"])],
            &test_config(),
        );

        let route = RouteRequest {
            required_tags: vec!["gen".into()],
            ..Default::default()
        };

        let before = pool.providers[0].semaphore.available_permits();
        let _ = pool.chat_completion(&test_request(), &route).await;
        let after = pool.providers[0].semaphore.available_permits();

        assert_eq!(before, after, "semaphore permits should be released after completion");
    }

    #[tokio::test]
    async fn test_concurrency_semaphore_release_on_error() {
        let pool = ProviderPoolImpl::from_providers(
            vec![MockProvider::failing("p1", vec!["gen"])],
            &test_config(),
        );

        let route = RouteRequest {
            required_tags: vec!["gen".into()],
            ..Default::default()
        };

        let before = pool.providers[0].semaphore.available_permits();
        let _ = pool.chat_completion(&test_request(), &route).await;
        let after = pool.providers[0].semaphore.available_permits();

        assert_eq!(before, after, "semaphore permits should be released even on error");
    }

    #[tokio::test]
    async fn test_pool_routes_to_best_provider() {
        let pool = ProviderPoolImpl::from_providers(
            vec![
                MockProvider::ok("p1", vec!["reasoning"]),
                MockProvider::ok("p2", vec!["fast"]),
                MockProvider::ok("p3", vec!["reasoning", "code"]),
            ],
            &test_config(),
        );

        let route = RouteRequest {
            required_tags: vec!["reasoning".into()],
            ..Default::default()
        };

        let result = pool.chat_completion(&test_request(), &route).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_pool_all_providers_frozen() {
        let pool = ProviderPoolImpl::from_providers(
            vec![
                MockProvider::ok("p1", vec!["gen"]),
                MockProvider::ok("p2", vec!["gen"]),
            ],
            &test_config(),
        );

        // Freeze all providers.
        for entry in &pool.providers {
            entry.freeze_manager.freeze("test".into(), None);
        }

        let route = RouteRequest {
            required_tags: vec!["gen".into()],
            ..Default::default()
        };

        let result = pool.chat_completion(&test_request(), &route).await;
        assert!(matches!(result, Err(ProviderError::NoProviderAvailable { .. })));
    }

    #[tokio::test]
    async fn test_pool_failover_on_error() {
        let pool = ProviderPoolImpl::from_providers(
            vec![
                MockProvider::failing("p1", vec!["gen"]),
                MockProvider::ok("p2", vec!["gen"]),
            ],
            &test_config(),
        );

        let route = RouteRequest {
            required_tags: vec!["gen".into()],
            ..Default::default()
        };

        // First call may hit p1 (fails) or p2 (succeeds) — depends on round-robin.
        // After p1 fails and gets frozen, subsequent calls should use p2.
        let _ = pool.chat_completion(&test_request(), &route).await;
        // Now p1 should be frozen from the error report.
        let result = pool.chat_completion(&test_request(), &route).await;
        assert!(result.is_ok(), "should route to p2 after p1 is frozen");
    }

    #[tokio::test]
    async fn test_pool_provider_statuses() {
        let pool = ProviderPoolImpl::from_providers(
            vec![MockProvider::ok("p1", vec!["gen"])],
            &test_config(),
        );

        let statuses = pool.provider_statuses().await;
        assert_eq!(statuses.len(), 1);
        assert!(!statuses[0].is_frozen);
        assert_eq!(statuses[0].total_requests, 0);
    }
}
