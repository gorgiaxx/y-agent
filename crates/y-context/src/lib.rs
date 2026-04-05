//! y-context: Context assembly pipeline, compaction, memory recall.
//!
//! This crate provides:
//!
//! - [`ContextPipeline`] — ordered pipeline of context providers
//! - [`ContextWindowGuard`] — token budget monitoring with 3 trigger modes
//! - [`CompactionEngine`] — summarizes older messages to reclaim context space
//! - [`repair`] — session history repair (empty, orphan, duplicate, merge)
//! - [`RecallStore`] — memory recall via hybrid text/vector search
//! - [`ContextMiddlewareAdapter`] — bridges `ContextProvider` to y-hooks Middleware
//! - [`InjectContextStatus`] — pipeline stage for context budget reporting
//!
//! The pipeline stages (`BuildSystemPrompt`, `InjectBootstrap`, `InjectMemory`,
//! `InjectSkills`, `InjectTools`, `LoadHistory`, `InjectContextStatus`) are
//! implemented as [`ContextProvider`] trait objects.

pub mod compaction;
pub mod context_manager;
pub mod context_status;
pub mod enrichment;
pub mod guard;
pub mod inject_bootstrap;
pub mod inject_memory;
pub mod inject_skills;
pub mod inject_tools;
pub mod knowledge_provider;
pub mod load_history;
pub mod memory;
pub mod middleware_adapter;
pub mod pipeline;
pub mod pruning;
pub mod recall;
pub mod repair;
pub mod system_prompt;
pub mod working_memory;

// Re-export primary types.
pub use compaction::{
    CompactionConfig, CompactionEngine, CompactionLlm, CompactionResult, CompactionStrategy,
    IdentifierPolicy,
};
pub use context_manager::{ContextManager, PreparedContext};
pub use context_status::InjectContextStatus;
pub use guard::{ContextWindowGuard, GuardMode, GuardVerdict, TokenBudget};
pub use inject_bootstrap::{BootstrapEntry, InjectBootstrap};
pub use inject_memory::InjectMemory;
pub use inject_skills::{InjectSkills, InjectSkillsStatic, SkillSummary, SkillTemplateVars};
pub use inject_tools::InjectTools;
pub use knowledge_provider::KnowledgeContextProvider;
pub use load_history::LoadHistory;
pub use middleware_adapter::{stage_priorities, ContextMiddlewareAdapter};
pub use pipeline::{
    AssembledContext, ContextCategory, ContextItem, ContextPipeline, ContextPipelineError,
    ContextProvider, ContextRequest,
};
pub use pruning::{
    PruningCandidate, PruningConfig, PruningEngine, PruningReport, PruningStrategy,
    PruningStrategyMode,
};
pub use recall::{RecallConfig, RecallMethod, RecallStore, RecalledMemory};
pub use repair::{repair_history, HistoryMessage, RepairReport};
pub use system_prompt::{
    BuildSystemPromptProvider, BunVenvPromptInfo, PythonVenvPromptInfo, SystemPromptConfig,
    VenvPromptInfo,
};
