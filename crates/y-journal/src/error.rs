//! Journal-specific error types.

use thiserror::Error;

/// Errors from the file journal system.
#[derive(Debug, Error)]
pub enum JournalError {
    /// Failed to capture file state before tool execution.
    #[error("capture failed for {path}: {message}")]
    CaptureFailed { path: String, message: String },

    /// Rollback conflict: file was modified by a third party.
    #[error("conflict on {path}: expected hash {expected}, found {actual}")]
    Conflict {
        path: String,
        expected: String,
        actual: String,
    },

    /// Journal entry not found.
    #[error("entry not found: {id}")]
    EntryNotFound { id: u64 },

    /// Scope not found.
    #[error("scope not found: {scope_id}")]
    ScopeNotFound { scope_id: String },

    /// Scope in invalid state for the requested operation.
    #[error("scope {scope_id} is in state {state}, cannot {operation}")]
    InvalidScopeState {
        scope_id: String,
        state: String,
        operation: String,
    },

    /// Storage I/O error.
    #[error("storage error: {message}")]
    StorageError { message: String },

    /// Generic error.
    #[error("{message}")]
    Other { message: String },
}
