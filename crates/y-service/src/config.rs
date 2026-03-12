//! Service configuration: the subset of config needed by the service layer.
//!
//! `ConfigLoader` and CLI-specific fields (`log_level`, `output_format`, `log_dir`)
//! stay in `y-cli`. This struct holds only the domain-relevant configuration
//! that `ServiceContainer` needs for construction.

use serde::Deserialize;

use y_guardrails::GuardrailConfig;
use y_hooks::HookConfig;
use y_provider::ProviderPoolConfig;
use y_runtime::RuntimeConfig;
use y_session::SessionConfig;
use y_storage::StorageConfig;
use y_tools::ToolRegistryConfig;

/// Configuration for constructing a [`ServiceContainer`](crate::ServiceContainer).
///
/// Contains all domain-relevant sub-configs. Presentation-specific fields
/// (log level, output format, log dir) are NOT included — they belong
/// in the presentation layer.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct ServiceConfig {
    /// Provider pool configuration.
    pub providers: ProviderPoolConfig,

    /// Storage configuration (`SQLite` + `PostgreSQL`).
    pub storage: StorageConfig,

    /// Session lifecycle configuration.
    pub session: SessionConfig,

    /// Tool execution runtime configuration.
    pub runtime: RuntimeConfig,

    /// Hook system configuration.
    pub hooks: HookConfig,

    /// Tool registry configuration.
    pub tools: ToolRegistryConfig,

    /// Guardrail/safety configuration.
    pub guardrails: GuardrailConfig,
}

