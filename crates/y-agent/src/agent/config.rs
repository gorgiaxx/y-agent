//! Multi-agent configuration.
//!
//! Design reference: multi-agent-design.md §Agent Pool Configuration

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultiAgentConfig {
    /// Maximum registered agent definitions (active + inactive).
    #[serde(default = "default_max_agents")]
    pub max_agents: usize,

    /// Maximum concurrent agent instances across all delegations.
    /// Design: default 5.
    #[serde(default = "default_max_concurrent_agents")]
    pub max_concurrent_agents: usize,

    /// Maximum parallel agents spawned per single delegation.
    /// Design: default 3.
    #[serde(default = "default_max_agents_per_delegation")]
    pub max_agents_per_delegation: usize,

    /// Default delegation timeout in milliseconds.
    #[serde(default = "default_delegation_timeout_ms")]
    pub delegation_timeout_ms: u64,

    /// Maximum delegation depth (prevents circular A→B→A).
    /// Design: default 3.
    #[serde(default = "default_max_delegation_depth")]
    pub max_delegation_depth: usize,
}

impl Default for MultiAgentConfig {
    fn default() -> Self {
        Self {
            max_agents: default_max_agents(),
            max_concurrent_agents: default_max_concurrent_agents(),
            max_agents_per_delegation: default_max_agents_per_delegation(),
            delegation_timeout_ms: default_delegation_timeout_ms(),
            max_delegation_depth: default_max_delegation_depth(),
        }
    }
}

const fn default_max_agents() -> usize {
    10
}
const fn default_max_concurrent_agents() -> usize {
    5
}
const fn default_max_agents_per_delegation() -> usize {
    3
}
const fn default_delegation_timeout_ms() -> u64 {
    60_000
}
const fn default_max_delegation_depth() -> usize {
    3
}
