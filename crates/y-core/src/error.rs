//! Shared error types and classification traits.

use thiserror::Error;

/// Severity classification for errors that cross crate boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorSeverity {
    /// Transient failure, safe to retry.
    Transient,
    /// Permanent failure, do not retry.
    Permanent,
    /// Requires user action (e.g., invalid config, missing API key).
    UserActionRequired,
}

/// Trait for errors that carry classification metadata.
pub trait ClassifiedError {
    /// Whether this error is safe to retry.
    fn is_retryable(&self) -> bool;

    /// Machine-readable error code (e.g., "`PROVIDER_RATE_LIMITED`").
    fn error_code(&self) -> &str;

    /// Severity classification.
    fn severity(&self) -> ErrorSeverity;
}

/// Trait for errors that may contain sensitive data.
pub trait Redactable {
    /// Return a redacted string representation safe for logging.
    fn redacted(&self) -> String;
}

/// Top-level error type for y-core operations.
#[derive(Debug, Error)]
pub enum CoreError {
    #[error("configuration error: {message}")]
    Config { message: String },

    #[error("serialization error: {source}")]
    Serialization {
        #[from]
        source: serde_json::Error,
    },

    #[error("not found: {entity} with id {id}")]
    NotFound { entity: String, id: String },

    #[error("internal error: {message}")]
    Internal { message: String },
}
