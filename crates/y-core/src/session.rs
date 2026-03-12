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

    /// Update session metadata (title, token_count, message_count).
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

    /// List checkpoints for a session, ordered by turn_number descending.
    async fn list_by_session(
        &self,
        session_id: &SessionId,
    ) -> Result<Vec<ChatCheckpoint>, SessionError>;

    /// Get the latest non-invalidated checkpoint for a session.
    async fn latest(&self, session_id: &SessionId)
        -> Result<Option<ChatCheckpoint>, SessionError>;

    /// Invalidate all checkpoints after a given turn number.
    async fn invalidate_after(
        &self,
        session_id: &SessionId,
        turn_number: u32,
    ) -> Result<u32, SessionError>;
}

