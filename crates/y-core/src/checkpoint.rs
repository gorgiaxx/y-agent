//! Checkpoint storage traits for workflow state persistence.
//!
//! Design reference: orchestrator-design.md
//!
//! The checkpoint system uses committed/pending write separation for
//! cancellation safety. Pending writes are visible only to the current
//! workflow step; committed writes survive crashes.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::types::{SessionId, Timestamp, WorkflowId};

// ---------------------------------------------------------------------------
// Checkpoint types
// ---------------------------------------------------------------------------

/// A workflow checkpoint capturing the full execution state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowCheckpoint {
    pub workflow_id: WorkflowId,
    pub session_id: SessionId,
    /// Monotonically increasing step counter.
    pub step_number: u64,
    pub status: CheckpointStatus,
    /// Committed channel state and task outputs (survives crashes).
    pub committed_state: serde_json::Value,
    /// Pending writes not yet committed (lost on crash).
    pub pending_state: Option<serde_json::Value>,
    /// Interrupt metadata (present when status = Interrupted).
    pub interrupt_data: Option<serde_json::Value>,
    /// Version tracking for stale checkpoint detection.
    pub versions_seen: serde_json::Value,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// Checkpoint lifecycle status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointStatus {
    Running,
    Completed,
    Failed,
    Interrupted,
    Compensating,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors from checkpoint operations.
#[derive(Debug, thiserror::Error)]
pub enum CheckpointError {
    #[error("checkpoint not found for workflow {workflow_id}")]
    NotFound { workflow_id: String },

    #[error("stale checkpoint: expected version {expected}, found {found}")]
    StaleCheckpoint { expected: u64, found: u64 },

    #[error("storage error: {message}")]
    StorageError { message: String },

    #[error("{message}")]
    Other { message: String },
}

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Persistent storage for workflow checkpoints.
///
/// Uses `SQLite` WAL mode for crash safety. The committed/pending
/// separation ensures that a crash during a step leaves the system
/// in the last committed state (not a partial step).
#[async_trait]
pub trait CheckpointStorage: Send + Sync {
    /// Write pending state for the current step (not yet durable).
    async fn write_pending(
        &self,
        workflow_id: &WorkflowId,
        session_id: &SessionId,
        step_number: u64,
        state: &serde_json::Value,
    ) -> Result<(), CheckpointError>;

    /// Commit the pending state, making it the new committed checkpoint.
    async fn commit(
        &self,
        workflow_id: &WorkflowId,
        step_number: u64,
    ) -> Result<(), CheckpointError>;

    /// Read the latest committed checkpoint for a workflow.
    async fn read_committed(
        &self,
        workflow_id: &WorkflowId,
    ) -> Result<Option<WorkflowCheckpoint>, CheckpointError>;

    /// Mark a checkpoint as interrupted (for HITL resume).
    async fn set_interrupted(
        &self,
        workflow_id: &WorkflowId,
        interrupt_data: serde_json::Value,
    ) -> Result<(), CheckpointError>;

    /// Mark a checkpoint as completed.
    async fn set_completed(&self, workflow_id: &WorkflowId) -> Result<(), CheckpointError>;

    /// Mark a checkpoint as failed.
    async fn set_failed(
        &self,
        workflow_id: &WorkflowId,
        error: &str,
    ) -> Result<(), CheckpointError>;

    /// Delete checkpoints older than the given step number (cleanup).
    async fn prune(
        &self,
        workflow_id: &WorkflowId,
        keep_after_step: u64,
    ) -> Result<u64, CheckpointError>;
}
