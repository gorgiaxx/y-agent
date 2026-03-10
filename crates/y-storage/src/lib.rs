//! y-storage: Persistence layer — SQLite backends, JSONL writers, blob storage.
//!
//! This crate provides concrete implementations of the storage traits
//! defined in `y-core`:
//!
//! - [`SqliteCheckpointStorage`] — committed/pending checkpoint persistence
//! - [`SqliteSessionStore`] — session tree metadata in SQLite
//! - [`JsonlTranscriptStore`] — JSONL file-based message transcripts
//!
//! It also manages SQLite connection pools (WAL mode) and migration execution.

pub mod checkpoint;
pub mod config;
pub mod error;
pub mod migration;
pub mod pool;
pub mod repository;
pub mod session_store;
pub mod transcript;

// Re-export primary types for convenient access.
pub use checkpoint::SqliteCheckpointStorage;
pub use config::StorageConfig;
pub use error::StorageError;
pub use pool::create_pool;
pub use session_store::SqliteSessionStore;
pub use transcript::JsonlTranscriptStore;
