//! Health checker for frozen providers.

use std::sync::Arc;
use std::time::Duration;

use tracing::instrument;

use y_core::provider::{ChatRequest, LlmProvider, ProviderError, ToolCallingMode};
use y_core::types::Message;

use crate::freeze::FreezeManager;

/// Performs health checks on frozen providers to determine if they can be thawed.
#[derive(Debug)]
pub struct HealthChecker {
    /// Timeout for health check requests.
    timeout: Duration,
}

impl HealthChecker {
    /// Create a new health checker.
    pub fn new(timeout: Duration) -> Self {
        Self { timeout }
    }

    /// Check if a provider is healthy by sending a minimal request.
    ///
    /// Returns `Ok(())` if the provider responds successfully,
    /// or an error describing why it is still unhealthy.
    #[instrument(skip(self, provider), fields(provider_id = %provider.metadata().id))]
    pub async fn check(&self, provider: &Arc<dyn LlmProvider>) -> Result<(), ProviderError> {
        let request = ChatRequest {
            messages: vec![Message {
                message_id: y_core::types::generate_message_id(),
                role: y_core::types::Role::User,
                content: "ping".to_string(),
                tool_call_id: None,
                tool_calls: vec![],
                timestamp: chrono::Utc::now(),
                metadata: serde_json::Value::Null,
            }],
            model: None,
            max_tokens: Some(1),
            temperature: Some(0.0),
            top_p: None,
            tools: vec![],
            tool_calling_mode: ToolCallingMode::default(),
            stop: vec![],
            extra: serde_json::Value::Null,
            thinking: None,
            response_format: None,
        };

        let result = tokio::time::timeout(self.timeout, provider.chat_completion(&request)).await;

        match result {
            Ok(Ok(_)) => {
                tracing::info!(
                    provider_id = %provider.metadata().id,
                    "health check passed"
                );
                Ok(())
            }
            Ok(Err(e)) => {
                tracing::warn!(
                    provider_id = %provider.metadata().id,
                    error = %e,
                    "health check failed"
                );
                Err(e)
            }
            Err(_) => {
                tracing::warn!(
                    provider_id = %provider.metadata().id,
                    timeout_ms = u64::try_from(self.timeout.as_millis()).unwrap_or(u64::MAX),
                    "health check timed out"
                );
                Err(ProviderError::NetworkError {
                    message: "health check timed out".into(),
                })
            }
        }
    }

    /// Attempt to thaw a frozen provider by running a health check.
    ///
    /// If the health check passes, the provider is thawed.
    /// If it fails, the provider remains frozen with a new freeze schedule.
    #[instrument(skip(self, provider, freeze_manager), fields(provider_id = %provider.metadata().id))]
    pub async fn check_and_thaw(
        &self,
        provider: &Arc<dyn LlmProvider>,
        freeze_manager: &FreezeManager,
    ) -> Result<(), ProviderError> {
        match self.check(provider).await {
            Ok(()) => {
                freeze_manager.thaw();
                tracing::info!(
                    provider_id = %provider.metadata().id,
                    "provider thawed after health check"
                );
                Ok(())
            }
            Err(e) => {
                // Re-freeze with adaptive duration.
                freeze_manager.freeze(
                    format!("health check failed: {e}"),
                    None, // Use adaptive duration.
                );
                Err(e)
            }
        }
    }
}

impl Default for HealthChecker {
    fn default() -> Self {
        Self::new(Duration::from_secs(10))
    }
}
