//! Trust tiers for permission scoping across agents.
//!
//! Design reference: agent-autonomy-design.md §Trust Model
//!
//! The trust hierarchy enforces `BuiltIn > UserDefined > Dynamic`.
//! Each tier determines what operations an agent can perform:
//! - `BuiltIn` agents have full access and can manage other agents.
//! - `UserDefined` agents have standard access and write-capable tools.
//! - `Dynamic` agents are restricted to only their creator-granted permissions.

use serde::{Deserialize, Serialize};

/// Trust tier assigned to an agent, determining its capabilities.
///
/// Ordered so that `BuiltIn > UserDefined > Dynamic`.
/// `PartialOrd`/`Ord` are derived based on discriminant values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustTier {
    /// Created by agents at runtime (lowest trust).
    Dynamic = 0,
    /// Defined by users in TOML config.
    UserDefined = 1,
    /// Shipped with the framework (highest trust).
    BuiltIn = 2,
}

impl PartialOrd for TrustTier {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for TrustTier {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (*self as u8).cmp(&(*other as u8))
    }
}

impl TrustTier {
    /// Whether this tier can manage (create/delete) other agents.
    pub fn can_manage_agents(self) -> bool {
        matches!(self, Self::BuiltIn)
    }

    /// Whether this tier can use write-capable tools.
    pub fn can_write(self) -> bool {
        matches!(self, Self::BuiltIn | Self::UserDefined)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// T-MA-R1-01: `TrustTier` ordering: `BuiltIn` > `UserDefined` > Dynamic.
    #[test]
    fn test_trust_tier_ordering() {
        assert!(TrustTier::BuiltIn > TrustTier::UserDefined);
        assert!(TrustTier::UserDefined > TrustTier::Dynamic);
        assert!(TrustTier::BuiltIn > TrustTier::Dynamic);
    }

    #[test]
    fn test_trust_tier_can_manage() {
        assert!(TrustTier::BuiltIn.can_manage_agents());
        assert!(!TrustTier::UserDefined.can_manage_agents());
        assert!(!TrustTier::Dynamic.can_manage_agents());
    }

    #[test]
    fn test_trust_tier_can_write() {
        assert!(TrustTier::BuiltIn.can_write());
        assert!(TrustTier::UserDefined.can_write());
        assert!(!TrustTier::Dynamic.can_write());
    }

    #[test]
    fn test_trust_tier_serde_roundtrip() {
        let tier = TrustTier::UserDefined;
        let json = serde_json::to_string(&tier).unwrap();
        assert_eq!(json, "\"user_defined\"");
        let parsed: TrustTier = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, tier);
    }
}
