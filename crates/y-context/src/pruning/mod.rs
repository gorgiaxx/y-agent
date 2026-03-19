//! Context pruning: message-level branch pruning for attention quality.
//!
//! Design reference: context-pruning-design.md
//!
//! Two built-in strategies:
//! - **RetryPruning**: removes failed tool call branches (zero LLM cost)
//! - **ProgressivePruning**: replaces completed multi-step sequences with
//!   LLM-generated rolling summaries

pub mod config;
pub mod detector;
pub mod engine;
pub mod progressive;
pub mod report;
pub mod retry;
pub mod strategy;

pub use config::{PruningConfig, PruningStrategyMode};
pub use detector::PruningDetector;
pub use engine::PruningEngine;
pub use progressive::ProgressivePruning;
pub use report::PruningReport;
pub use retry::RetryPruning;
pub use strategy::{PruningCandidate, PruningReason, PruningStrategy};
