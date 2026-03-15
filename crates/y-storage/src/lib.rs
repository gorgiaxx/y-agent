//! y-storage: Persistence layer — `SQLite` backends, optional `PostgreSQL`,
//! JSONL writers, blob storage.
//!
//! This crate provides concrete implementations of the storage traits
//! defined in `y-core`:
//!
//! - [`SqliteCheckpointStorage`] — committed/pending checkpoint persistence
//! - [`SqliteSessionStore`] — session tree metadata in `SQLite`
//! - [`JsonlTranscriptStore`] — JSONL file-based message transcripts
//!
//! It also manages `SQLite` connection pools (WAL mode) and migration execution.
//! `PostgreSQL` support for diagnostics is available behind the `diagnostics_pg`
//! feature flag.

pub mod checkpoint;
pub mod checkpoint_chat;
pub mod chat_message;
pub mod config;
pub mod error;
pub mod migration;
pub mod pg_pool;
pub mod pool;
pub mod repository;
pub mod schedule_store;
pub mod session_store;
pub mod transcript;
pub mod transcript_display;
pub mod workflow_store;

// Re-export primary types for convenient access.
pub use checkpoint::SqliteCheckpointStorage;
pub use checkpoint_chat::SqliteChatCheckpointStore;
pub use chat_message::SqliteChatMessageStore;
pub use config::StorageConfig;
pub use error::StorageError;
pub use pool::create_pool;
pub use schedule_store::SqliteScheduleStore;
pub use session_store::SqliteSessionStore;
pub use transcript::JsonlTranscriptStore;
pub use transcript_display::JsonlDisplayTranscriptStore;
pub use workflow_store::SqliteWorkflowStore;

#[cfg(feature = "diagnostics_pg")]
pub use pg_pool::create_pg_pool;
