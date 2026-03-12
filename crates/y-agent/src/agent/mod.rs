//! Agent lifecycle: definitions, registry, pool, delegation, patterns.
//!
//! Enables multi-agent collaboration through:
//! - Agent definitions (TOML-based mode/capability descriptors)
//! - Agent behavioral modes (Build/Plan/Explore/General)
//! - Agent registry for unified definition management (BuiltIn/UserDefined/Dynamic)
//! - Agent pool for runtime instance lifecycle management
//! - Mode overlay and context injection for delegations
//! - Delegation protocol with context strategies and depth tracking
//! - Capability gap detection and resolution
//! - Agent executor for full delegation lifecycle
//! - Built-in `task` tool for in-conversation delegation
//! - Collaboration patterns (Sequential, Hierarchical)
//! - Trust tiers for permission scoping (`BuiltIn` > `UserDefined` > Dynamic)
//! - Meta-tools for dynamic agent lifecycle management

pub mod config;
pub mod context;
pub mod definition;
pub mod delegation;
pub mod dynamic_agent;
pub mod error;
pub mod executor;
pub mod gap;
pub mod meta_tools;
pub mod mode;
pub mod patterns;
pub mod pool;
pub mod registry;
pub mod task_tool;
pub mod trust;

pub use config::MultiAgentConfig;
pub use definition::{AgentDefinition, AgentMode, ContextStrategy};
pub use delegation::DelegationProtocol;
pub use error::MultiAgentError;
pub use pool::AgentPool;
pub use registry::AgentRegistry;
pub use trust::TrustTier;
