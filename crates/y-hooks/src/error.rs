//! Crate-level error types for y-hooks.

/// Errors from the hook/middleware/event system.
#[derive(Debug, thiserror::Error)]
pub enum HookError {
    #[error("middleware '{name}' not found")]
    MiddlewareNotFound { name: String },

    #[error("middleware '{name}' already registered")]
    MiddlewareAlreadyRegistered { name: String },

    #[error("hook handler registration failed: {message}")]
    RegistrationError { message: String },

    #[error("event bus error: {message}")]
    EventBusError { message: String },

    #[error("chain execution error: {message}")]
    ChainError { message: String },

    #[error("hook handler error ({handler_type}): {message}")]
    HookHandlerError {
        handler_type: String,
        message: String,
    },

    #[error("hook handler timeout ({handler_type}): exceeded {timeout_ms}ms")]
    HookHandlerTimeout {
        handler_type: String,
        timeout_ms: u64,
    },

    #[error("hook handler validation error: {message}")]
    HookHandlerValidation { message: String },

    #[error("{message}")]
    Other { message: String },
}
