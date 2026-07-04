//! Decision enum for exec policy rules.
//!
//! Maps to y-agent's `PermissionBehavior` vocabulary:
//! - `Allow` → allow without prompting
//! - `Ask` → prompt the user for approval
//! - `Deny` → block unconditionally

use serde::{Deserialize, Serialize};

use crate::exec_policy::error::{ExecPolicyError, ExecPolicyResult};

/// The decision a policy rule returns for a command.
///
/// Ordered from most permissive to most restrictive:
/// `Allow` < `Ask` < `Deny`.
/// When multiple rules match, the strictest wins.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecDecision {
    /// Command may run without further approval.
    Allow,
    /// Request explicit user approval.
    Ask,
    /// Command is blocked without further consideration.
    Deny,
}

impl ExecDecision {
    /// Parse a decision from its string form.
    pub fn parse(raw: &str) -> ExecPolicyResult<Self> {
        match raw {
            "allow" => Ok(Self::Allow),
            "ask" | "prompt" => Ok(Self::Ask),
            "deny" | "forbidden" => Ok(Self::Deny),
            other => Err(ExecPolicyError::InvalidDecision(other.to_string())),
        }
    }

    /// String form used in Starlark policy files.
    pub const fn as_policy_string(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Ask => "ask",
            Self::Deny => "deny",
        }
    }
}

impl std::fmt::Display for ExecDecision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_policy_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_allow() {
        assert_eq!(ExecDecision::parse("allow").unwrap(), ExecDecision::Allow);
    }

    #[test]
    fn parse_ask_aliases() {
        assert_eq!(ExecDecision::parse("ask").unwrap(), ExecDecision::Ask);
        assert_eq!(ExecDecision::parse("prompt").unwrap(), ExecDecision::Ask);
    }

    #[test]
    fn parse_deny_aliases() {
        assert_eq!(ExecDecision::parse("deny").unwrap(), ExecDecision::Deny);
        assert_eq!(
            ExecDecision::parse("forbidden").unwrap(),
            ExecDecision::Deny
        );
    }

    #[test]
    fn parse_invalid() {
        assert!(ExecDecision::parse("maybe").is_err());
    }

    #[test]
    fn strictest_wins_via_ord() {
        // Allow < Ask < Deny — max() gives the strictest.
        assert_eq!(
            [ExecDecision::Allow, ExecDecision::Deny, ExecDecision::Ask]
                .into_iter()
                .max()
                .unwrap(),
            ExecDecision::Deny
        );
    }
}
