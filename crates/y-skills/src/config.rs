//! Configuration for the skills module.

use serde::{Deserialize, Serialize};

/// Skill module configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillConfig {
    /// Maximum tokens allowed in a skill root document.
    #[serde(default = "default_max_root_tokens")]
    pub max_root_tokens: u32,

    /// Base directory for the content-addressable version store.
    ///
    /// Relative paths are resolved against the XDG data directory
    /// (e.g., `~/.local/state/y-agent/data/`). Absolute paths are used as-is.
    #[serde(default = "default_store_path")]
    pub store_path: String,
}

impl Default for SkillConfig {
    fn default() -> Self {
        Self {
            max_root_tokens: 2000,
            store_path: "skills".to_string(),
        }
    }
}

const fn default_max_root_tokens() -> u32 {
    2000
}

fn default_store_path() -> String {
    "skills".to_string()
}
