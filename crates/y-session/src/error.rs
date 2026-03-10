//! Crate-level error types for y-session.

/// Errors from session management.
#[derive(Debug, thiserror::Error)]
pub enum SessionManagerError {
    #[error("session not found: {id}")]
    NotFound { id: String },

    #[error("invalid state transition from {from:?} to {to:?}")]
    InvalidTransition { from: String, to: String },

    #[error("storage error: {message}")]
    Storage { message: String },

    #[error("transcript error: {message}")]
    Transcript { message: String },

    #[error("session configuration error: {message}")]
    Config { message: String },

    #[error("{message}")]
    Other { message: String },
}

impl From<y_core::session::SessionError> for SessionManagerError {
    fn from(err: y_core::session::SessionError) -> Self {
        Self::Storage {
            message: err.to_string(),
        }
    }
}
