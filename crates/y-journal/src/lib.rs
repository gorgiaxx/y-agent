//! y-journal: File journal middleware for automatic file-level change tracking.
//!
//! This crate provides:
//!
//! - [`FileJournalMiddleware`] — intercepts file-mutating tool calls and captures state
//! - [`JournalStore`] — in-memory journal entry storage with scope management
//! - [`conflict`] — hash-based conflict detection for rollback safety
//! - [`rollback`] — scope-based file restoration
//!
//! # Design
//!
//! The file journal operates as a `ToolMiddleware` that captures file state
//! before file-mutating tool calls (`FileWrite`).
//! On failure or explicit rollback, entries are replayed in reverse to
//! restore files to their pre-operation state.

pub mod conflict;
pub mod error;
pub mod middleware;
pub mod rollback;
pub mod storage;

// Re-export primary types.
pub use conflict::{detect_conflict, ConflictStatus};
pub use error::JournalError;
pub use middleware::FileJournalMiddleware;
pub use rollback::{rollback_scope, RollbackReport};
pub use storage::{
    FileOperation, JournalEntry, JournalScope, JournalStore, ScopeStatus, ScopeType,
    StorageStrategy,
};
