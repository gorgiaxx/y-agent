//! y-agent: Unified Agent Framework.
//!
//! Combines the orchestrator (DAG engine, typed channels, checkpointing) with
//! agent lifecycle management (definitions, registry, pool, delegation) into
//! a single crate.
//!
//! # Modules
//!
//! - [`orchestrator`] — DAG engine, typed channels, checkpointing, interrupt/resume
//! - [`agent`] — Agent definitions, registry, pool, delegation, patterns

pub mod agent;
pub mod orchestrator;

// ── Convenience re-exports from agent ────────────────────────────────────
pub use agent::config::MultiAgentConfig;
pub use agent::definition::{AgentDefinition, AgentMode, ContextStrategy};
pub use agent::delegation::DelegationProtocol;
pub use agent::error::MultiAgentError;
pub use agent::pool::{AgentPool, DelegationTracker};
pub use agent::registry::AgentRegistry;
pub use agent::trust::TrustTier;
