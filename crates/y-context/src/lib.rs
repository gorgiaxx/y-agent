//! y-context: Context assembly pipeline, compaction, memory recall.
//!
//! This crate provides:
//!
//! - [`ContextPipeline`] — ordered pipeline of context providers
//! - [`ContextWindowGuard`] — token budget monitoring with 3 trigger modes
//! - [`CompactionEngine`] — summarizes older messages to reclaim context space
//! - [`repair`] — session history repair (empty, orphan, duplicate, merge)
//! - [`RecallStore`] — memory recall via hybrid text/vector search
//!
//! The pipeline stages (`BuildSystemPrompt`, `InjectBootstrap`, `InjectMemory`,
//! `InjectSkills`, `InjectTools`, `LoadHistory`, `InjectContextStatus`) are
//! implemented as [`ContextProvider`] trait objects.

pub mod compaction;
pub mod guard;
pub mod pipeline;
pub mod recall;
pub mod repair;

// Re-export primary types.
pub use compaction::{
    CompactionConfig, CompactionEngine, CompactionResult, CompactionStrategy, IdentifierPolicy,
};
pub use guard::{ContextWindowGuard, GuardMode, GuardVerdict, TokenBudget};
pub use pipeline::{
    AssembledContext, ContextCategory, ContextItem, ContextPipeline, ContextPipelineError,
    ContextProvider,
};
pub use recall::{RecallConfig, RecallMethod, RecallStore, RecalledMemory};
pub use repair::{repair_history, HistoryMessage, RepairReport};
