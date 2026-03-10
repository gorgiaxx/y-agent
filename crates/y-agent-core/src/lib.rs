//! y-agent-core: Agent Orchestrator — DAG engine, typed channels, checkpointing.
//!
//! This crate provides the core execution engine for y-agent workflows:
//!
//! - [`TaskDag`] — DAG-based task dependency resolution and scheduling
//! - [`Channel`] / [`WorkflowContext`] — typed state channels with configurable reducers
//! - [`CheckpointStore`] — workflow state persistence for task-level recovery
//! - [`InterruptManager`] — interrupt/resume protocol for human-in-the-loop
//! - [`WorkflowExecutor`] — orchestrates DAG execution with checkpointing

pub mod channel;
pub mod checkpoint;
pub mod dag;
pub mod executor;
pub mod interrupt;

// Re-export primary types.
pub use channel::{Channel, ChannelType, WorkflowContext};
pub use checkpoint::{ChannelSnapshot, CheckpointStore, TaskOutput, WorkflowCheckpoint};
pub use dag::{DagError, TaskDag, TaskId, TaskNode, TaskPriority};
pub use executor::{
    ExecutionConfig, StreamMode, WorkflowExecuteError, WorkflowExecutor, WorkflowState,
};
pub use interrupt::{InterruptManager, InterruptState, ResumeCommand, WorkflowInterrupt};
