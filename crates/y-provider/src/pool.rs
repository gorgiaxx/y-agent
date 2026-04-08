//! Provider pool implementation — the main `ProviderPool` trait impl.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tracing::instrument;

use y_core::provider::{
    ChatRequest, ChatResponse, ChatStreamResponse, LlmProvider, ProviderError, ProviderPool,
    ProviderStatus, RouteRequest,
};
use y_core::types::ProviderId;

use crate::config::ProviderPoolConfig;
use crate::error::ProviderPoolError;
use crate::error_classifier;

use crate::freeze::FreezeManager;
use crate::health::HealthChecker;
use crate::metrics::{ProviderMetrics, SharedMetrics};
use crate::router::{RoutableProvider, TagBasedRouter};

/// Concrete implementation of the `ProviderPool` trait.
///
/// Manages a set of LLM providers with tag-based routing, freeze/thaw,
/// per-provider concurrency limits, global concurrency limit, and metrics tracking.
pub struct ProviderPoolImpl {
    providers: Vec<ProviderEntry>,
    router: TagBasedRouter,
    health_checker: HealthChecker,
    /// Global concurrency semaphore (across all providers).
    global_semaphore: Option<Arc<tokio::sync::Semaphore>>,
}

/// An entry in the pool combining provider, freeze state, semaphore, and metrics.
struct ProviderEntry {
    provider: Arc<dyn LlmProvider>,
    freeze_manager: Arc<FreezeManager>,
    semaphore: Arc<tokio::sync::Semaphore>,
    max_concurrency: usize,
    metrics: SharedMetrics,
    /// Explicit counter for active in-flight requests (including streaming).
    /// The semaphore is released when `chat_completion_stream` returns, but
    /// the stream may still be consumed. This counter is decremented only
    /// after the request/stream is fully complete.
    active_requests: Arc<AtomicUsize>,
}

impl std::fmt::Debug for ProviderPoolImpl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderPoolImpl")
            .field("provider_count", &self.providers.len())
            .finish_non_exhaustive()
    }
}

/// RAII guard that decrements a provider's active-request counter on drop.
///
/// Used to track streaming requests: the guard is moved into the wrapped
/// stream and lives until the stream is fully consumed or dropped.
struct ActiveRequestGuard(Arc<AtomicUsize>);

impl Drop for ActiveRequestGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
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
                    active_requests: Arc::new(AtomicUsize::new(0)),
                }
            })
            .collect();

        let global_semaphore = config
            .max_global_concurrency
            .map(|limit| Arc::new(tokio::sync::Semaphore::new(limit)));

        Self {
            providers: entries,
            router: TagBasedRouter::with_strategy(config.selection_strategy),
            health_checker: HealthChecker::new(Duration::from_secs(10)),
            global_semaphore,
        }
    }

    /// Create a new pool from a `ProviderPoolConfig`.
    ///
    /// Validates the config, resolves API keys and proxy URLs per provider,
    /// constructs the appropriate provider backend for each entry, and
    /// delegates to [`from_providers`](Self::from_providers).
    ///
    /// Providers with `enabled = false` are silently skipped.
    pub fn from_config(config: &ProviderPoolConfig) -> Result<Self, ProviderPoolError> {
        config.validate()?;
        let providers = build_providers(config);
        Ok(Self::from_providers(providers, config))
    }
}

/// Build provider instances from configuration.
///
/// Resolves API keys and proxy URLs per provider, constructs the appropriate
/// provider backend for each entry. Providers with `enabled = false` or
/// missing API keys are silently skipped (logged at info/warn level).
///
/// This is the **single source of truth** for provider construction.
/// Both `ProviderPoolImpl::from_config` and `ServiceContainer` must use
/// this function to avoid behavioral divergence.
pub fn build_providers(config: &ProviderPoolConfig) -> Vec<Arc<dyn LlmProvider>> {
    let mut providers: Vec<Arc<dyn LlmProvider>> = Vec::with_capacity(config.providers.len());

    for cfg in &config.providers {
        // Skip disabled providers.
        if !cfg.enabled {
            tracing::info!(
                provider_id = %cfg.id,
                "provider is disabled, skipping"
            );
            continue;
        }

        let Some(api_key) = cfg.resolve_api_key() else {
            let env_var = cfg.api_key_env.as_deref().unwrap_or("(not configured)");
            tracing::warn!(
                provider_id = %cfg.id,
                env_var = %env_var,
                "Skipping provider: API key not found in environment"
            );
            continue;
        };

        let proxy_url = config.resolve_proxy_url(&cfg.id, &cfg.tags);
        let tool_calling_mode = cfg.resolve_tool_calling_mode();

        // DeepSeek uses an OpenAI-compatible REST API with a default base URL.
        let base_url_for_deepseek = || {
            cfg.base_url
                .clone()
                .or_else(|| Some("https://api.deepseek.com/v1".to_string()))
        };

        // Macro to reduce per-variant boilerplate: every provider constructor
        // takes the same 9 arguments in the same order.
        macro_rules! make_provider {
            ($ty:ty, $base:expr) => {
                Arc::new(<$ty>::new(
                    &cfg.id,
                    &cfg.model,
                    api_key.clone(),
                    $base,
                    proxy_url.clone(),
                    cfg.tags.clone(),
                    cfg.max_concurrency,
                    cfg.context_window,
                    tool_calling_mode,
                )) as Arc<dyn LlmProvider>
            };
        }

        let provider: Option<Arc<dyn LlmProvider>> = match cfg.provider_type.as_str() {
            // openai-compat / openai_compatible / custom all use the OpenAI provider.
            "openai" | "openai-compat" | "openai_compatible" | "custom" => Some(make_provider!(
                crate::providers::openai::OpenAiProvider,
                cfg.base_url.clone()
            )),
            "anthropic" => Some(make_provider!(
                crate::providers::anthropic::AnthropicProvider,
                cfg.base_url.clone()
            )),
            "gemini" => Some(make_provider!(
                crate::providers::gemini::GeminiProvider,
                cfg.base_url.clone()
            )),
            "ollama" => Some(make_provider!(
                crate::providers::ollama::OllamaProvider,
                cfg.base_url.clone()
            )),
            "azure" => Some(make_provider!(
                crate::providers::azure::AzureOpenAiProvider,
                cfg.base_url.clone()
            )),
            "deepseek" => Some(make_provider!(
                crate::providers::openai::OpenAiProvider,
                base_url_for_deepseek()
            )),
            other => {
                tracing::warn!(
                    provider_id = %cfg.id,
                    provider_type = %other,
                    "Skipping provider: unsupported type \
                    (supported: openai, openai-compat, anthropic, gemini, ollama, azure, deepseek)"
                );
                None
            }
        };

        if let Some(p) = provider {
            providers.push(p);
        }
    }

    providers
}

impl ProviderPoolImpl {
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
        self.find_entry(provider_id).map(|e| Arc::clone(&e.metrics))
    }

    /// Return metadata for all registered providers.
    ///
    /// Used by the TUI to query context window sizes and model names.
    pub fn list_metadata(&self) -> Vec<y_core::provider::ProviderMetadata> {
        self.providers
            .iter()
            .map(|e| e.provider.metadata().clone())
            .collect()
    }

    /// Return a snapshot of metrics for all providers.
    ///
    /// Each entry pairs the provider ID with its current metrics snapshot.
    /// Avoids N+1 lookups when building observability reports.
    pub fn all_metrics(&self) -> Vec<(y_core::types::ProviderId, crate::metrics::MetricsSnapshot)> {
        self.providers
            .iter()
            .map(|e| (e.provider.metadata().id.clone(), e.metrics.snapshot()))
            .collect()
    }

    /// Attach a metrics event sender to every provider's metrics tracker.
    ///
    /// Each provider fires [`MetricsEvent`](crate::metrics::MetricsEvent)
    /// values through the returned receiver. The sender tags events with the
    /// provider ID + model so the consumer can persist them without needing
    /// a back-reference to the pool.
    ///
    /// Returns a list of `(provider_id, model, receiver)` tuples.
    pub fn attach_event_senders(
        &self,
    ) -> Vec<(
        String,
        String,
        tokio::sync::mpsc::UnboundedReceiver<crate::metrics::MetricsEvent>,
    )> {
        self.providers
            .iter()
            .map(|entry| {
                let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
                entry.metrics.set_event_sender(tx);
                let meta = entry.provider.metadata();
                (meta.id.to_string(), meta.model.clone(), rx)
            })
            .collect()
    }

    /// Record token usage for a completed streaming request.
    ///
    /// Called by the service layer after a streaming response has been fully
    /// consumed and the final token counts are available. The request itself
    /// was already counted at stream start; this only adds token counts and
    /// cost.
    pub fn record_stream_completion(
        &self,
        provider_id: &ProviderId,
        input_tokens: u32,
        output_tokens: u32,
    ) {
        if let Some(entry) = self.find_entry(provider_id) {
            let meta = entry.provider.metadata();
            entry.metrics.record_stream_completion(
                input_tokens,
                output_tokens,
                meta.cost_per_1k_input,
                meta.cost_per_1k_output,
            );
        }
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

        // Acquire global semaphore permit if configured.
        let _global_permit = if let Some(ref sem) = self.global_semaphore {
            Some(sem.acquire().await.map_err(|_| ProviderError::Other {
                message: "global semaphore closed".into(),
            })?)
        } else {
            None
        };

        // Acquire per-provider semaphore permit for concurrency control.
        let _permit = entry
            .semaphore
            .acquire()
            .await
            .map_err(|_| ProviderError::Other {
                message: "semaphore closed".into(),
            })?;

        // Track active request for observability.
        entry.active_requests.fetch_add(1, Ordering::Relaxed);

        let result = entry.provider.chat_completion(request).await;

        // Decrement active counter after request completes.
        entry.active_requests.fetch_sub(1, Ordering::Relaxed);

        match result {
            Ok(mut response) => {
                let meta = entry.provider.metadata();
                entry.metrics.record_success_with_cost(
                    response.usage.input_tokens,
                    response.usage.output_tokens,
                    meta.cost_per_1k_input,
                    meta.cost_per_1k_output,
                );
                response.provider_id = Some(meta.id.clone());
                Ok(response)
            }
            Err(e) => {
                entry.metrics.record_error();
                self.report_error(&entry.provider.metadata().id, &e);
                Err(e)
            }
        }
    }

    #[instrument(skip(self, request), fields(tags = ?route.required_tags))]
    async fn chat_completion_stream(
        &self,
        request: &ChatRequest,
        route: &RouteRequest,
    ) -> Result<ChatStreamResponse, ProviderError> {
        let routable = self.routable_providers();
        let idx = self.router.select(&routable, route)?;
        let entry = &self.providers[idx];

        // Acquire global semaphore permit if configured.
        let _global_permit = if let Some(ref sem) = self.global_semaphore {
            Some(sem.acquire().await.map_err(|_| ProviderError::Other {
                message: "global semaphore closed".into(),
            })?)
        } else {
            None
        };

        let _permit = entry
            .semaphore
            .acquire()
            .await
            .map_err(|_| ProviderError::Other {
                message: "semaphore closed".into(),
            })?;

        // Track active request for observability (decremented when stream is
        // fully consumed or dropped, via ActiveRequestGuard).
        entry.active_requests.fetch_add(1, Ordering::Relaxed);
        let guard = ActiveRequestGuard(Arc::clone(&entry.active_requests));

        // Streaming metrics are tracked at the caller level when the stream
        // completes or errors. We do NOT record a premature success here
        // because the stream has not started consuming yet.

        let meta = entry.provider.metadata();
        let stream_result = entry.provider.chat_completion_stream(request).await;

        match stream_result {
            Ok(mut stream_response) => {
                stream_response.provider_id = Some(meta.id.clone());
                stream_response.model.clone_from(&meta.model);
                stream_response.context_window = meta.context_window;

                // Wrap the inner stream so `guard` is held until the stream
                // is fully consumed or dropped.
                let inner = stream_response.stream;
                stream_response.stream = Box::pin(futures::stream::unfold(
                    (inner, Some(guard)),
                    |(mut s, g)| async move {
                        use futures::StreamExt;
                        s.next().await.map(|item| (item, (s, g)))
                    },
                ));

                Ok(stream_response)
            }
            Err(e) => {
                entry.metrics.record_error();
                self.report_error(&meta.id, &e);
                // guard drops here, decrementing active_requests
                Err(e)
            }
        }
    }

    fn report_error(&self, provider_id: &ProviderId, error: &ProviderError) {
        if let Some(entry) = self.find_entry(provider_id) {
            // Use the error classifier (P1-5) for freeze decisions.
            let std_error = error_classifier::classify_provider_error(error);

            if !std_error.should_freeze() {
                tracing::debug!(
                    provider_id = %provider_id,
                    error = %error,
                    classification = ?std_error,
                    "error does not warrant provider freeze"
                );
                return;
            }

            if std_error.is_permanent() {
                entry.freeze_manager.freeze_permanent(format!("{error}"));
                tracing::warn!(
                    provider_id = %provider_id,
                    error = %error,
                    classification = ?std_error,
                    "provider permanently frozen"
                );
            } else if let Some(duration) = std_error.freeze_duration() {
                entry
                    .freeze_manager
                    .freeze(format!("{error}"), Some(duration));
                tracing::info!(
                    provider_id = %provider_id,
                    error = %error,
                    classification = ?std_error,
                    freeze_secs = duration.as_secs(),
                    "provider frozen with error-type-specific duration"
                );
            } else {
                // Fallback: adaptive freeze.
                entry.freeze_manager.freeze(format!("{error}"), None);
                tracing::info!(
                    provider_id = %provider_id,
                    error = %error,
                    classification = ?std_error,
                    "provider frozen (adaptive duration)"
                );
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
                    frozen_since: freeze_status.frozen_since.map(|inst| {
                        chrono::Utc::now()
                            - chrono::Duration::from_std(inst.elapsed()).unwrap_or_default()
                    }),
                    thaw_at: freeze_status.thaw_at.map(|inst| {
                        let now = std::time::Instant::now();
                        if inst > now {
                            chrono::Utc::now()
                                + chrono::Duration::from_std(inst - now).unwrap_or_default()
                        } else {
                            chrono::Utc::now()
                        }
                    }),
                    freeze_reason: freeze_status.reason,
                    active_requests: entry.active_requests.load(Ordering::Relaxed),
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
                    tool_calling_mode: ToolCallingMode::default(),
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
                    tool_calling_mode: ToolCallingMode::default(),
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
                reasoning_content: None,
                tool_calls: vec![],
                usage: TokenUsage {
                    input_tokens: 10,
                    output_tokens: 5,
                    cache_read_tokens: None,
                    cache_write_tokens: None,
                    ..Default::default()
                },
                finish_reason: FinishReason::Stop,
                raw_request: None,
                raw_response: None,
                provider_id: None,
            })
        }

        async fn chat_completion_stream(
            &self,
            _request: &ChatRequest,
        ) -> Result<ChatStreamResponse, ProviderError> {
            panic!("MockProvider does not support streaming -- this code path should not be reached in pool unit tests")
        }

        fn metadata(&self) -> &ProviderMetadata {
            &self.meta
        }
    }

    fn test_config() -> ProviderPoolConfig {
        ProviderPoolConfig {
            providers: vec![],
            proxy: Default::default(),
            default_freeze_duration_secs: 30,
            max_freeze_duration_secs: 3600,
            health_check_interval_secs: 60,
            selection_strategy: Default::default(),
            max_global_concurrency: None,
        }
    }

    fn test_request() -> ChatRequest {
        ChatRequest {
            messages: vec![y_core::types::Message {
                message_id: String::new(),
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
            top_p: None,
            tools: vec![],
            tool_calling_mode: ToolCallingMode::default(),
            stop: vec![],
            extra: serde_json::Value::Null,
            thinking: None,
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

        assert_eq!(
            before, after,
            "semaphore permits should be released after completion"
        );
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

        assert_eq!(
            before, after,
            "semaphore permits should be released even on error"
        );
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
        assert!(matches!(
            result,
            Err(ProviderError::NoProviderAvailable { .. })
        ));
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

    // -----------------------------------------------------------------------
    // from_config() tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_from_config_creates_providers() {
        use crate::config::{ProviderConfig, ProxyEntry};

        let config = ProviderPoolConfig {
            providers: vec![
                ProviderConfig {
                    id: "openai-1".into(),
                    provider_type: "openai".into(),
                    model: "gpt-4o".into(),
                    enabled: true,
                    tags: vec!["general".into()],
                    max_concurrency: 3,
                    context_window: 128_000,
                    cost_per_1k_input: 0.005,
                    cost_per_1k_output: 0.015,
                    api_key: Some("sk-test".into()),
                    api_key_env: None,
                    base_url: None,
                    temperature: None,
                    top_p: None,
                    tool_calling_mode: None,
                    icon: None,
                },
                ProviderConfig {
                    id: "anthropic-1".into(),
                    provider_type: "anthropic".into(),
                    model: "claude-3-opus".into(),
                    enabled: true,
                    tags: vec!["reasoning".into()],
                    max_concurrency: 3,
                    context_window: 200_000,
                    cost_per_1k_input: 0.015,
                    cost_per_1k_output: 0.075,
                    api_key: Some("sk-ant-test".into()),
                    api_key_env: None,
                    base_url: None,
                    temperature: None,
                    top_p: None,
                    tool_calling_mode: None,
                    icon: None,
                },
                ProviderConfig {
                    id: "gemini-1".into(),
                    provider_type: "gemini".into(),
                    model: "gemini-2.0-flash".into(),
                    enabled: true,
                    tags: vec!["fast".into()],
                    max_concurrency: 5,
                    context_window: 1_000_000,
                    cost_per_1k_input: 0.0,
                    cost_per_1k_output: 0.0,
                    api_key: Some("AIza-test".into()),
                    api_key_env: None,
                    base_url: None,
                    temperature: None,
                    top_p: None,
                    tool_calling_mode: None,
                    icon: None,
                },
                ProviderConfig {
                    id: "ollama-local".into(),
                    provider_type: "ollama".into(),
                    model: "llama3.1:8b".into(),
                    enabled: true,
                    tags: vec!["local".into()],
                    max_concurrency: 3,
                    context_window: 32_768,
                    cost_per_1k_input: 0.0,
                    cost_per_1k_output: 0.0,
                    api_key: Some("ollama-key".into()),
                    api_key_env: None,
                    base_url: None,
                    temperature: None,
                    top_p: None,
                    tool_calling_mode: None,
                    icon: None,
                },
                ProviderConfig {
                    id: "azure-1".into(),
                    provider_type: "azure".into(),
                    model: "gpt-4o".into(),
                    enabled: true,
                    tags: vec!["cloud".into()],
                    max_concurrency: 5,
                    context_window: 128_000,
                    cost_per_1k_input: 0.005,
                    cost_per_1k_output: 0.015,
                    api_key: Some("azure-key".into()),
                    api_key_env: None,
                    base_url: Some("https://res.openai.azure.com/openai/deployments/gpt-4o".into()),
                    temperature: None,
                    top_p: None,
                    tool_calling_mode: None,
                    icon: None,
                },
            ],
            proxy: crate::config::ProxyConfig {
                providers: {
                    let mut m = std::collections::HashMap::new();
                    m.insert(
                        "ollama-local".into(),
                        ProxyEntry {
                            url: None,
                            enabled: false,
                            auth_env: None,
                        },
                    );
                    m
                },
                ..Default::default()
            },
            ..Default::default()
        };

        let pool = ProviderPoolImpl::from_config(&config).expect("should create pool");
        assert_eq!(pool.providers.len(), 5);

        // Verify provider types via metadata.
        let ids: Vec<String> = pool
            .providers
            .iter()
            .map(|e| e.provider.metadata().id.to_string())
            .collect();
        assert_eq!(
            ids,
            vec![
                "openai-1",
                "anthropic-1",
                "gemini-1",
                "ollama-local",
                "azure-1"
            ]
        );
    }

    #[test]
    fn test_from_config_empty_fails() {
        let config = ProviderPoolConfig::default();
        let result = ProviderPoolImpl::from_config(&config);
        assert!(result.is_err());
    }

    #[test]
    fn test_from_config_unknown_type_fails() {
        use crate::config::ProviderConfig;
        let config = ProviderPoolConfig {
            providers: vec![ProviderConfig {
                id: "unknown-1".into(),
                provider_type: "supermodel".into(),
                model: "best".into(),
                enabled: true,
                tags: vec![],
                max_concurrency: 5,
                context_window: 128_000,
                cost_per_1k_input: 0.0,
                cost_per_1k_output: 0.0,
                api_key: None,
                api_key_env: None,
                base_url: None,
                temperature: None,
                top_p: None,
                tool_calling_mode: None,
                icon: None,
            }],
            ..Default::default()
        };

        let pool = ProviderPoolImpl::from_config(&config).expect("should create pool");
        // Unknown provider type is gracefully skipped, resulting in 0 providers.
        assert_eq!(pool.providers.len(), 0);
    }
}
