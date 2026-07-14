//! Context pruning: message-level branch pruning for attention quality.
//!
//! Architecture reference: `docs/guides/ARCHITECTURE.md`
//!
//! Two built-in strategies:
//! - **`RetryPruning`**: removes failed tool call branches (zero LLM cost)
//! - **`ProgressivePruning`**: replaces completed multi-step sequences with
//!   LLM-generated rolling summaries

pub mod config;
pub mod detector;
pub mod engine;
pub mod intra_turn;
pub mod patterns;
pub mod progressive;
pub mod report;
pub mod retry;
pub mod strategy;
pub mod superseded;
pub use config::{PruningConfig, PruningStrategyMode};
pub use detector::PruningDetector;
pub use engine::PruningEngine;
pub use intra_turn::{IntraTurnPruner, IntraTurnPruningReport};
pub use progressive::ProgressivePruning;
pub use report::PruningReport;
pub use retry::RetryPruning;
pub use strategy::{PruningCandidate, PruningReason, PruningStrategy};
pub use superseded::{
    prune_tool_outputs, supersede_key, ToolOutputPruneConfig, ToolOutputPruneResult,
};
