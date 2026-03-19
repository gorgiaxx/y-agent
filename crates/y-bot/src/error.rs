//! Bot platform error types.

/// Errors that can occur in bot platform operations.
#[derive(Debug, thiserror::Error)]
pub enum BotError {
    /// Webhook signature verification failed.
    #[error("signature verification failed")]
    SignatureInvalid,

    /// The event type is not supported or not relevant.
    #[error("unsupported event type: {0}")]
    UnsupportedEvent(String),

    /// Failed to parse the inbound event payload.
    #[error("parse error: {0}")]
    ParseError(String),

    /// Remote API call failed.
    #[error("API error: {0}")]
    ApiError(String),

    /// The platform is not yet implemented.
    #[error("not implemented: {0}")]
    NotImplemented(String),

    /// HTTP transport error.
    #[error("HTTP error: {0}")]
    HttpError(#[from] reqwest::Error),
}
