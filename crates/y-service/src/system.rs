//! System status service.

use y_core::provider::ProviderPool;
use y_core::runtime::RuntimeAdapter;

use crate::container::ServiceContainer;

/// Status report for the system.
#[derive(Debug, Clone, serde::Serialize)]
pub struct StatusReport {
    /// Application version.
    pub version: String,
    /// Number of registered LLM providers.
    pub providers_registered: usize,
    /// Number of registered tools.
    pub tools_registered: usize,
    /// Runtime backend identifier.
    pub runtime_backend: String,
    /// Storage connection status.
    pub storage_status: String,
}

/// Health report combining diagnostics and system status.
#[derive(Debug, Clone, serde::Serialize)]
pub struct HealthReport {
    /// System status.
    pub status: StatusReport,
    /// Diagnostics health.
    pub diagnostics: crate::diagnostics::HealthCheckResult,
}

/// System-level service for status and health reporting.
pub struct SystemService;

impl SystemService {
    /// Gather system status report.
    pub async fn status(container: &ServiceContainer, version: &str) -> StatusReport {
        let provider_count = container.provider_pool.provider_statuses().await.len();
        let tool_count = container.tool_registry.len().await;
        let runtime_backend = format!("{:?}", container.runtime_manager.backend());

        StatusReport {
            version: version.to_string(),
            providers_registered: provider_count,
            tools_registered: tool_count,
            runtime_backend,
            storage_status: "connected".to_string(),
        }
    }

    /// Full health report (system + diagnostics).
    pub async fn health(container: &ServiceContainer, version: &str) -> HealthReport {
        let status = Self::status(container, version).await;
        let diagnostics = crate::DiagnosticsService::health_check(container).await;
        HealthReport {
            status,
            diagnostics,
        }
    }
}
