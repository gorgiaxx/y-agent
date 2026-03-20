//! y-storage: Persistence layer — `SQLite` backends, JSONL writers, blob
//! storage.
//!
//! This crate provides concrete implementations of the storage traits
//! defined in `y-core`:
//!
//! - [`SqliteCheckpointStorage`] — committed/pending checkpoint persistence
//! - [`SqliteSessionStore`] — session tree metadata in `SQLite`
//! - [`JsonlTranscriptStore`] — JSONL file-based message transcripts
//!
//! It also manages `SQLite` connection pools (WAL mode) and migration execution.

pub mod chat_message;
pub mod checkpoint;
pub mod checkpoint_chat;
pub mod config;
pub mod error;
pub mod migration;
pub mod pool;
pub mod repository;
pub mod schedule_store;
pub mod session_store;
pub mod transcript;
pub mod transcript_display;
pub mod workflow_store;

// Re-export primary types for convenient access.
pub use chat_message::SqliteChatMessageStore;
pub use checkpoint::SqliteCheckpointStorage;
pub use checkpoint_chat::SqliteChatCheckpointStore;
pub use config::StorageConfig;
pub use error::StorageError;
pub use pool::create_pool;
pub use schedule_store::SqliteScheduleStore;
pub use session_store::SqliteSessionStore;
pub use sqlx::SqlitePool;
pub use transcript::JsonlTranscriptStore;
pub use transcript_display::JsonlDisplayTranscriptStore;
pub use workflow_store::SqliteWorkflowStore;
