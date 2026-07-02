//! Security verdict types for skill content screening.
//!
//! These types are produced by the agent-based security screening pipeline
//! and consumed by the filter gate. See [`SecurityVerdict`] and
//! [`SecurityFinding`].

use serde::{Deserialize, Serialize};

/// Type of security finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecurityFindingType {
    /// Attempt to override system instructions.
    PromptInjection,
    /// Attempt to gain unauthorized access.
    PrivilegeEscalation,
    /// Unconstrained delegation to sub-agents.
    UnconstrainedDelegation,
    /// Attempt to exfiltrate data.
    DataExfiltration,
    /// Overly broad permissions or unconstrained behavior.
    ExcessiveFreedom,
}

impl std::fmt::Display for SecurityFindingType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::PromptInjection => "prompt_injection",
            Self::PrivilegeEscalation => "privilege_escalation",
            Self::UnconstrainedDelegation => "unconstrained_delegation",
            Self::DataExfiltration => "data_exfiltration",
            Self::ExcessiveFreedom => "excessive_freedom",
        };
        f.write_str(s)
    }
}

/// A single security finding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityFinding {
    /// Type of issue found.
    pub finding_type: SecurityFindingType,
    /// Human-readable description.
    pub description: String,
    /// Severity: 1 (low) to 5 (critical).
    pub severity: u8,
    /// Line number where the pattern was found (if applicable).
    pub line: Option<usize>,
}

/// Overall security verdict.
#[derive(Debug, Clone)]
pub enum SecurityVerdict {
    /// Content passed all security checks.
    Pass,
    /// Content blocked due to security issues.
    Blocked {
        /// Primary reason for blocking.
        reason: String,
        /// Type of the most severe finding.
        finding_type: SecurityFindingType,
        /// All findings.
        findings: Vec<SecurityFinding>,
    },
}

impl SecurityVerdict {
    /// Returns true if the verdict is `Pass`.
    pub fn is_pass(&self) -> bool {
        matches!(self, Self::Pass)
    }
}
