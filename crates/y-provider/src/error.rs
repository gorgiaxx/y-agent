//! Crate-level error types for y-provider.

/// Errors from the provider pool layer.
///
/// These are distinct from `y_core::provider::ProviderError` which represents
/// individual provider failures. `ProviderPoolError` represents pool-level
/// issues (config, routing decisions, pool management).
#[derive(Debug, thiserror::Error)]
pub enum ProviderPoolError {
    #[error("provider pool configuration error: {message}")]
    Config { message: String },

    #[error("provider '{id}' not found in pool")]
    ProviderNotFound { id: String },

    #[error("duplicate provider id: {id}")]
    DuplicateProvider { id: String },

    #[error("provider '{id}' is frozen: {reason}")]
    ProviderFrozen { id: String, reason: String },

    #[error("health check failed for '{id}': {message}")]
    HealthCheckFailed { id: String, message: String },

    #[error("{message}")]
    Other { message: String },
}
