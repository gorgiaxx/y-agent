//! Orchestrator: DAG engine, typed channels, checkpointing.
//!
//! This module provides the core execution engine for y-agent workflows:
//!
//! - [`TaskDag`] — DAG-based task dependency resolution and scheduling
//! - [`Channel`] / [`WorkflowContext`] — typed state channels with configurable reducers
//! - [`CheckpointStore`] — workflow state persistence for task-level recovery
//! - [`InterruptManager`] — interrupt/resume protocol for human-in-the-loop
//! - [`WorkflowExecutor`] — orchestrates DAG execution with checkpointing
//! - [`TaskExecutor`] — trait for type-dispatched async task execution
//! - [`FailureStrategy`] / [`RetryConfig`] -- failure handling and retry configuration
//! - [`ConcurrencyController`] -- global and per-resource concurrency limits

pub mod channel;
pub mod checkpoint;
pub mod concurrency;
pub mod dag;
pub mod executor;
pub mod executors;
pub mod expression_dsl;
pub mod failure;
pub mod interrupt;
pub mod io_mapping;
pub mod micro_pipeline;
pub mod task_executor;
pub mod toml_parser;
pub mod workflow_meta;

// Re-export primary types.
pub use channel::{Channel, ChannelType, WorkflowContext};
pub use checkpoint::{ChannelSnapshot, CheckpointStore, TaskOutput, WorkflowCheckpoint};
pub use concurrency::{ConcurrencyController, ResourceType};
pub use dag::{DagError, TaskDag, TaskId, TaskNode, TaskPriority, TaskType};
pub use executor::{
    ExecutionConfig, StreamMode, WorkflowExecuteError, WorkflowExecutor, WorkflowState,
};
pub use failure::{BackoffStrategy, FailureStrategy, RetryConfig};
pub use interrupt::{InterruptManager, InterruptState, ResumeCommand, WorkflowInterrupt};
pub use io_mapping::{InputMapping, InputResolveError, OutputMapping};
pub use task_executor::{TaskExecuteError, TaskExecutor};
pub use toml_parser::{ParsedWorkflow, TomlParseError, WorkflowDefinition};
