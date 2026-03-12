//! Configuration for the knowledge module.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeConfig {
    /// Maximum tokens per L0 chunk (summary).
    #[serde(default = "default_l0_max_tokens")]
    pub l0_max_tokens: u32,

    /// Maximum tokens per L1 chunk (section).
    #[serde(default = "default_l1_max_tokens")]
    pub l1_max_tokens: u32,

    /// Maximum tokens per L2 chunk (paragraph).
    #[serde(default = "default_l2_max_tokens")]
    pub l2_max_tokens: u32,
}

impl Default for KnowledgeConfig {
    fn default() -> Self {
        Self {
            l0_max_tokens: 200,
            l1_max_tokens: 500,
            l2_max_tokens: 1000,
        }
    }
}

const fn default_l0_max_tokens() -> u32 {
    200
}
const fn default_l1_max_tokens() -> u32 {
    500
}
const fn default_l2_max_tokens() -> u32 {
    1000
}
