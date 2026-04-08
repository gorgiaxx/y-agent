//! Session store traits and associated types.
//!
//! Design reference: context-session-design.md
//!
//! Sessions form a tree structure supporting branching conversations.
//! Metadata is stored in `SQLite`; message transcripts in JSONL files.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::types::{AgentId, Message, SessionId, Timestamp};

// ---------------------------------------------------------------------------
// Session types
// ---------------------------------------------------------------------------

/// A node in the session tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionNode {
    pub id: SessionId,
    pub parent_id: Option<SessionId>,
    pub root_id: SessionId,
    pub depth: u32,
    /// Materialized path from root to this node.
    pub path: Vec<SessionId>,
    pub session_type: SessionType,
    pub state: SessionState,
    /// Agent that owns this session (if any).
    pub agent_id: Option<AgentId>,
    pub title: Option<String>,
    /// Channel identifier (e.g., "cli", "tui", "api").
    pub channel: Option<String>,
    /// User-defined label for categorization.
    pub label: Option<String>,
    pub token_count: u32,
    pub message_count: u32,
    /// When the last compaction was performed.
    pub last_compaction: Option<Timestamp>,
    /// Number of compactions performed on this session.
    pub compaction_count: u32,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// Session type within the tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionType {
    /// Top-level user session.
    Main,
    /// Child session created by the agent (e.g., sub-agent delegation).
    Child,
    /// Branch from an existing session (conversation fork).
    Branch,
    /// Temporary session that is not persisted long-term.
    Ephemeral,
    /// Cross-channel canonical session.
    Canonical,
}

/// Session lifecycle state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Active,
    Paused,
    Archived,
    Merged,
    /// Soft-deleted; kept for referential integrity.
    Tombstone,
}

/// Options for creating a new session.
#[derive(Debug, Clone)]
pub struct CreateSessionOptions {
    pub parent_id: Option<SessionId>,
    pub session_type: SessionType,
    pub agent_id: Option<AgentId>,
    pub title: Option<String>,
}

/// Filter for listing sessions.
#[derive(Debug, Clone, Default)]
pub struct SessionFilter {
    pub state: Option<SessionState>,
    pub session_type: Option<SessionType>,
    pub agent_id: Option<AgentId>,
    pub root_id: Option<SessionId>,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors from session operations.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("session not found: {id}")]
    NotFound { id: String },

    #[error("invalid session state transition: {from:?} -> {to:?}")]
    InvalidStateTransition {
        from: SessionState,
        to: SessionState,
    },

    #[error("storage error: {message}")]
    StorageError { message: String },

    #[error("transcript error: {message}")]
    TranscriptError { message: String },

    #[error("{message}")]
    Other { message: String },
}

// ---------------------------------------------------------------------------
// Traits
// ---------------------------------------------------------------------------

/// Persistent storage for session tree metadata.
#[async_trait]
pub trait SessionStore: Send + Sync {
    /// Create a new session, returning the created node.
    async fn create(&self, options: CreateSessionOptions) -> Result<SessionNode, SessionError>;

    /// Get a session by ID.
    async fn get(&self, id: &SessionId) -> Result<SessionNode, SessionError>;

    /// List sessions matching the given filter.
    async fn list(&self, filter: &SessionFilter) -> Result<Vec<SessionNode>, SessionError>;

    /// Update session state (e.g., Active -> Paused).
    async fn set_state(&self, id: &SessionId, state: SessionState) -> Result<(), SessionError>;

    /// Update session metadata (title, `token_count`, `message_count`).
    async fn update_metadata(
        &self,
        id: &SessionId,
        title: Option<String>,
        token_count: u32,
        message_count: u32,
    ) -> Result<(), SessionError>;

    /// Get all children of a session.
    async fn children(&self, id: &SessionId) -> Result<Vec<SessionNode>, SessionError>;

    /// Get the full ancestor path from root to this session.
    async fn ancestors(&self, id: &SessionId) -> Result<Vec<SessionNode>, SessionError>;

    /// Update only the session title.
    async fn set_title(&self, id: &SessionId, title: String) -> Result<(), SessionError>;

    /// Hard-delete a session and all its data from storage.
    ///
    /// This permanently removes the session metadata row. Transcript files
    /// must be removed separately by the caller (they are not in the database).
    async fn delete(&self, id: &SessionId) -> Result<(), SessionError>;

    /// Get the context reset index for a session.
    ///
    /// Returns `None` if no reset has been set (full context is used).
    async fn get_context_reset_index(&self, id: &SessionId) -> Result<Option<u32>, SessionError>;

    /// Set or clear the context reset index for a session.
    ///
    /// When set, the LLM context is trimmed to only include messages after
    /// this index. Pass `None` to clear (use full context).
    async fn set_context_reset_index(
        &self,
        id: &SessionId,
        index: Option<u32>,
    ) -> Result<(), SessionError>;

    /// Get the custom system prompt for a session.
    ///
    /// Returns `None` if no custom prompt has been set (global prompt is used).
    async fn get_custom_system_prompt(
        &self,
        id: &SessionId,
    ) -> Result<Option<String>, SessionError>;

    /// Set or clear the custom system prompt for a session.
    ///
    /// When set, the custom prompt replaces the built-in identity/behavioral
    /// sections while preserving dynamic/functional sections (tool protocol,
    /// plan mode, datetime, environment). Pass `None` to revert to the
    /// global prompt.
    async fn set_custom_system_prompt(
        &self,
        id: &SessionId,
        prompt: Option<String>,
    ) -> Result<(), SessionError>;
}

/// Read/write interface for session message transcripts (JSONL).
#[async_trait]
pub trait TranscriptStore: Send + Sync {
    /// Append a message to the session transcript.
    async fn append(&self, session_id: &SessionId, message: &Message) -> Result<(), SessionError>;

    /// Read all messages for a session.
    async fn read_all(&self, session_id: &SessionId) -> Result<Vec<Message>, SessionError>;

    /// Read the last N messages for a session.
    async fn read_last(
        &self,
        session_id: &SessionId,
        count: usize,
    ) -> Result<Vec<Message>, SessionError>;

    /// Count messages in a session transcript.
    async fn message_count(&self, session_id: &SessionId) -> Result<usize, SessionError>;

    /// Truncate the transcript, keeping only the first `keep_count` messages.
    /// Returns the number of messages removed.
    async fn truncate(
        &self,
        session_id: &SessionId,
        keep_count: usize,
    ) -> Result<usize, SessionError>;
}

/// Append-only transcript for GUI display.
///
/// This store mirrors the context transcript but is **never compacted**.
/// The GUI reads exclusively from this store so users always see the
/// full, uncompacted conversation history.  Only truncated during
/// undo/rollback operations.
///
/// Design reference: `GUI_SESSION_SEPARATION_PLAN.md` §3.1
#[async_trait]
pub trait DisplayTranscriptStore: Send + Sync {
    /// Append a message to the display transcript.
    async fn append(&self, session_id: &SessionId, message: &Message) -> Result<(), SessionError>;

    /// Read all messages from the display transcript.
    async fn read_all(&self, session_id: &SessionId) -> Result<Vec<Message>, SessionError>;

    /// Count messages in the display transcript.
    async fn message_count(&self, session_id: &SessionId) -> Result<usize, SessionError>;

    /// Truncate the display transcript, keeping only the first `keep_count` messages.
    /// Returns the number of messages removed.
    async fn truncate(
        &self,
        session_id: &SessionId,
        keep_count: usize,
    ) -> Result<usize, SessionError>;
}

// ---------------------------------------------------------------------------
// Chat checkpoint types
// ---------------------------------------------------------------------------

/// A checkpoint record linking a chat turn to a File Journal scope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCheckpoint {
    /// Unique checkpoint identifier.
    pub checkpoint_id: String,
    /// Session this checkpoint belongs to.
    pub session_id: SessionId,
    /// Turn number (1-indexed, incremented per user message).
    pub turn_number: u32,
    /// Number of messages in transcript BEFORE this turn started.
    /// Truncating to this count restores the pre-turn state.
    pub message_count_before: u32,
    /// File Journal scope ID associated with this turn.
    pub journal_scope_id: String,
    /// Whether this checkpoint has been invalidated (by a rollback past it).
    pub invalidated: bool,
    /// Timestamp when checkpoint was created.
    pub created_at: Timestamp,
}

/// Persistent storage for chat checkpoints.
#[async_trait]
pub trait ChatCheckpointStore: Send + Sync {
    /// Save a checkpoint record.
    async fn save(&self, checkpoint: &ChatCheckpoint) -> Result<(), SessionError>;

    /// Load a checkpoint by ID.
    async fn load(&self, checkpoint_id: &str) -> Result<ChatCheckpoint, SessionError>;

    /// List checkpoints for a session, ordered by `turn_number` descending.
    async fn list_by_session(
        &self,
        session_id: &SessionId,
    ) -> Result<Vec<ChatCheckpoint>, SessionError>;

    /// Get the latest non-invalidated checkpoint for a session.
    async fn latest(&self, session_id: &SessionId) -> Result<Option<ChatCheckpoint>, SessionError>;

    /// Invalidate all checkpoints after a given turn number.
    async fn invalidate_after(
        &self,
        session_id: &SessionId,
        turn_number: u32,
    ) -> Result<u32, SessionError>;
}

// ---------------------------------------------------------------------------
// Chat message store (Phase 2 — session history tree)
// ---------------------------------------------------------------------------

/// Status of a chat message in the history tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatMessageStatus {
    Active,
    Tombstone,
    /// Removed by pruning engine for attention quality optimization.
    /// Recoverable via `restore_pruned()`.
    Pruned,
}

/// A persisted chat message record stored in `SQLite` (Phase 2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessageRecord {
    pub id: String,
    pub session_id: SessionId,
    pub role: String,
    pub content: String,
    pub status: ChatMessageStatus,
    pub checkpoint_id: Option<String>,
    pub model: Option<String>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cost_usd: Option<f64>,
    pub context_window: Option<i64>,
    /// Parent message in the session message tree. NULL for root-level
    /// and pre-migration messages.
    pub parent_message_id: Option<String>,
    /// Logical grouping identifier for batch pruning operations.
    pub pruning_group_id: Option<String>,
    pub created_at: Timestamp,
}

/// Persistent storage for chat messages supporting soft-delete and branch recovery.
#[async_trait]
pub trait ChatMessageStore: Send + Sync {
    /// Insert a new message.
    async fn insert(&self, record: &ChatMessageRecord) -> Result<(), SessionError>;

    /// List all messages for a session (both active and tombstoned), ordered by `created_at`.
    async fn list_by_session(
        &self,
        session_id: &SessionId,
    ) -> Result<Vec<ChatMessageRecord>, SessionError>;

    /// List only active messages for a session, ordered by `created_at`.
    async fn list_active(
        &self,
        session_id: &SessionId,
    ) -> Result<Vec<ChatMessageRecord>, SessionError>;

    /// Tombstone (soft-delete) all messages after a given checkpoint.
    /// Returns the number of messages tombstoned.
    async fn tombstone_after(
        &self,
        session_id: &SessionId,
        checkpoint_id: &str,
    ) -> Result<u32, SessionError>;

    /// Restore previously tombstoned messages that belong to a given checkpoint.
    /// Returns the number of messages restored.
    async fn restore_tombstoned(
        &self,
        session_id: &SessionId,
        checkpoint_id: &str,
    ) -> Result<u32, SessionError>;

    /// Swap branches: tombstone currently active messages after the checkpoint,
    /// and restore the tombstoned ones. Returns (`tombstoned_count`, `restored_count`).
    async fn swap_branches(
        &self,
        session_id: &SessionId,
        checkpoint_id: &str,
    ) -> Result<(u32, u32), SessionError>;

    /// Update the status of a single message (used by pruning engine).
    async fn set_status(
        &self,
        session_id: &SessionId,
        message_id: &str,
        status: ChatMessageStatus,
    ) -> Result<(), SessionError>;

    /// Batch-update status for multiple messages (used by pruning engine).
    async fn set_status_batch(
        &self,
        session_id: &SessionId,
        message_ids: &[String],
        status: ChatMessageStatus,
    ) -> Result<u32, SessionError>;

    /// Restore all pruned messages in a session back to active.
    async fn restore_pruned(&self, session_id: &SessionId) -> Result<u32, SessionError>;
}
