use serde::{Deserialize, Serialize};

/// Strict versioned manifest parsed from `capability-pack.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityPackManifest {
    pub pack: CapabilityPackMetadata,
    #[serde(default)]
    pub resources: Vec<CapabilityResourceDeclaration>,
}

/// Pack-level identity and schema metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityPackMetadata {
    pub schema_version: u32,
    pub id: String,
    pub version: String,
    #[serde(default)]
    pub description: Option<String>,
}

/// One explicitly declared capability resource.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityResourceDeclaration {
    pub kind: CapabilityResourceKind,
    pub id: String,
    pub path: String,
    pub sha256: String,
}

/// Capability owner selected for a declared resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityResourceKind {
    Skill,
    Agent,
    Workflow,
    Mcp,
    Hook,
    Lsp,
}

impl CapabilityResourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Skill => "skill",
            Self::Agent => "agent",
            Self::Workflow => "workflow",
            Self::Mcp => "mcp",
            Self::Hook => "hook",
            Self::Lsp => "lsp",
        }
    }

    pub fn expects_directory(self) -> bool {
        matches!(self, Self::Skill)
    }
}
