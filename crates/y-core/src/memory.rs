//! Memory client traits and associated types.
//!
//! Design reference: memory-architecture-design.md
//!
//! Three-tier memory system:
//! - Long-Term Memory (LTM): persistent, vector store backed, workspace-scoped
//! - Short-Term Memory (STM): session-scoped, `SQLite` backed
//! - Working Memory (WM): pipeline-scoped, in-memory blackboard
//!
//! Access is via dual protocol: gRPC (high-performance internal) and
//! MCP (third-party integration). Both expose the same `MemoryClient` trait.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::types::{MemoryId, SessionId, SkillId, Timestamp};

// ---------------------------------------------------------------------------
// Memory types
// ---------------------------------------------------------------------------

/// A single memory entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    pub id: MemoryId,
    pub memory_type: MemoryType,
    /// Scopes this memory belongs to (workspace, project, etc.).
    pub scopes: Vec<String>,
    /// Description of when this memory is useful (used as embedding target).
    pub when_to_use: String,
    /// The actual memory content.
    pub content: String,
    /// Importance score (0.0 - 1.0), subject to time decay.
    pub importance: f32,
    /// Number of times this memory has been recalled.
    pub access_count: u32,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

/// Memory category.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryType {
    /// Personal user preferences and patterns.
    Personal,
    /// Task-specific learned information.
    Task,
    /// Tool usage patterns and tips.
    Tool,
    /// Experience records from completed tasks.
    Experience,
}

/// Query for retrieving memories.
#[derive(Debug, Clone)]
pub struct MemoryQuery {
    /// Natural language query (used for semantic search).
    pub query: String,
    /// Filter by memory type.
    pub memory_type: Option<MemoryType>,
    /// Filter by scope.
    pub scope: Option<String>,
    /// Maximum number of results.
    pub limit: usize,
    /// Minimum importance score.
    pub min_importance: Option<f32>,
}

/// Result of a memory query.
#[derive(Debug, Clone)]
pub struct MemoryResult {
    pub memory: Memory,
    /// Semantic similarity score (0.0 - 1.0).
    pub relevance: f32,
}

// ---------------------------------------------------------------------------
// Experience store types (STM enhancement)
// ---------------------------------------------------------------------------

/// A compressed experience record stored in the Experience Store.
///
/// The Experience Store is a session-scoped STM enhancement that provides
/// indexed archival of task experiences. Agents control archival via
/// `compress_experience` and retrieval via `read_experience`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperienceRecord {
    pub session_id: SessionId,
    /// Stable index for agent dereference (e.g., "see experience #3").
    pub slot_index: u32,
    /// Compressed summary of the experience.
    pub summary: String,
    /// Provenance tag for evidence reliability.
    pub evidence_type: EvidenceType,
    /// Associated skill (None for skillless experiences).
    pub skill_id: Option<SkillId>,
    /// Estimated token count for budget awareness.
    pub token_estimate: u32,
    pub created_at: Timestamp,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

/// Provenance classification for experience evidence.
///
/// `AgentObservation` requires corroboration from user evidence
/// (hard rule in Pattern Extraction, not a soft weight).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceType {
    /// Explicitly stated by the user.
    UserStated,
    /// User corrected a previous behavior.
    UserCorrection,
    /// Outcome of a completed task (success/failure).
    TaskOutcome,
    /// Agent's own observation (requires corroboration).
    AgentObservation,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors from memory operations.
#[derive(Debug, thiserror::Error)]
pub enum MemoryError {
    #[error("memory not found: {id}")]
    NotFound { id: String },

    #[error("storage error: {message}")]
    StorageError { message: String },

    #[error("embedding error: {message}")]
    EmbeddingError { message: String },

    #[error("vector store error: {message}")]
    VectorStoreError { message: String },

    #[error("{message}")]
    Other { message: String },
}

// ---------------------------------------------------------------------------
// Traits
// ---------------------------------------------------------------------------

/// Client interface for the memory system.
///
/// Abstracts over three backends: Local, gRPC, and MCP.
/// All return the same types regardless of transport.
#[async_trait]
pub trait MemoryClient: Send + Sync {
    /// Store a new memory or update an existing one.
    async fn remember(&self, memory: Memory) -> Result<MemoryId, MemoryError>;

    /// Recall memories relevant to a query.
    async fn recall(&self, query: MemoryQuery) -> Result<Vec<MemoryResult>, MemoryError>;

    /// Delete a memory by ID.
    async fn forget(&self, id: &MemoryId) -> Result<(), MemoryError>;

    /// Get a specific memory by ID.
    async fn get(&self, id: &MemoryId) -> Result<Memory, MemoryError>;
}

/// Session-scoped experience store for indexed archival.
#[async_trait]
pub trait ExperienceStore: Send + Sync {
    /// Compress and archive an experience, returning the assigned slot index.
    async fn compress(
        &self,
        session_id: &SessionId,
        summary: String,
        evidence_type: EvidenceType,
        skill_id: Option<SkillId>,
    ) -> Result<u32, MemoryError>;

    /// Read an experience by session and slot index.
    async fn read(
        &self,
        session_id: &SessionId,
        slot_index: u32,
    ) -> Result<ExperienceRecord, MemoryError>;

    /// List all experiences in a session.
    async fn list(&self, session_id: &SessionId) -> Result<Vec<ExperienceRecord>, MemoryError>;
}
