//! Capability gap detection and resolution middleware.
//!
//! Design reference: agent-autonomy-design.md §Capability-Gap Resolution
//!
//! The `CapabilityGapMiddleware` detects when the agent's current tool or
//! agent inventory is insufficient for a task and triggers automatic
//! resolution. It unifies tool gaps and agent gaps under a single
//! middleware, replacing the previous separate `ToolGapMiddleware`.
//!
//! Feature flag: `capability_gap_resolution`

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Gap types
// ---------------------------------------------------------------------------

/// Types of capability gaps (tool + agent, unified).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityGapType {
    // Tool gaps
    /// Required tool does not exist.
    ToolNotFound,
    /// Tool exists but parameters don't match the request.
    ParameterMismatch,
    /// Tool has hardcoded constraints that prevent the operation.
    HardcodedConstraint,

    // Agent gaps
    /// Required agent does not exist.
    AgentNotFound,
    /// Agent exists but lacks the needed capability.
    CapabilityMismatch,
    /// Agent exists but its mode is inappropriate.
    ModeInappropriate,
}

impl CapabilityGapType {
    /// Whether this is a tool gap (vs agent gap).
    pub fn is_tool_gap(&self) -> bool {
        matches!(
            self,
            Self::ToolNotFound | Self::ParameterMismatch | Self::HardcodedConstraint
        )
    }

    /// Whether this is an agent gap.
    pub fn is_agent_gap(&self) -> bool {
        !self.is_tool_gap()
    }
}

// ---------------------------------------------------------------------------
// Gap and resolution
// ---------------------------------------------------------------------------

/// A detected capability gap.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityGap {
    /// Type of gap detected.
    pub gap_type: CapabilityGapType,
    /// Name of the tool or agent involved.
    pub target_name: String,
    /// What capability was desired.
    pub desired_capability: String,
    /// Session context.
    pub session_id: String,
}

/// Result of attempting to resolve a gap.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GapResolution {
    /// Gap was resolved by creating/updating a tool or agent.
    Resolved {
        /// Name of the created/updated tool or agent.
        name: String,
    },
    /// Gap could not be resolved automatically.
    Unresolvable {
        /// Reason the gap is unresolvable.
        reason: String,
    },
    /// Resolution requires human input.
    EscalateToUser {
        /// Prompt to show the user.
        prompt: String,
    },
}

// ---------------------------------------------------------------------------
// Middleware
// ---------------------------------------------------------------------------

/// Capability gap detection and resolution middleware.
///
/// Detects gaps after tool/agent execution failures and triggers
/// resolution via specialized sub-agents (tool-engineer, agent-architect).
pub struct CapabilityGapMiddleware {
    /// Whether the middleware is active (feature flag).
    enabled: bool,
    /// History of detected gaps for this session.
    gap_history: Vec<(CapabilityGap, GapResolution)>,
}

impl CapabilityGapMiddleware {
    /// Create a new middleware instance.
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            gap_history: Vec::new(),
        }
    }

    /// Create a disabled (no-op) middleware.
    pub fn disabled() -> Self {
        Self::new(false)
    }

    /// Whether the middleware is enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Detect a capability gap from an error context.
    ///
    /// Analyzes the error to classify the gap type.
    pub fn detect_gap(
        &self,
        error_message: &str,
        target_name: &str,
        session_id: &str,
    ) -> Option<CapabilityGap> {
        if !self.enabled {
            return None;
        }

        let gap_type = if error_message.contains("not found") || error_message.contains("unknown") {
            if error_message.contains("tool") {
                CapabilityGapType::ToolNotFound
            } else if error_message.contains("agent") {
                CapabilityGapType::AgentNotFound
            } else {
                CapabilityGapType::ToolNotFound // default to tool gap
            }
        } else if error_message.contains("parameter") || error_message.contains("argument") {
            CapabilityGapType::ParameterMismatch
        } else if error_message.contains("capability") || error_message.contains("unable") {
            CapabilityGapType::CapabilityMismatch
        } else if error_message.contains("mode") {
            CapabilityGapType::ModeInappropriate
        } else {
            return None; // Cannot classify
        };

        Some(CapabilityGap {
            gap_type,
            target_name: target_name.to_string(),
            desired_capability: error_message.to_string(),
            session_id: session_id.to_string(),
        })
    }

    /// Attempt to resolve a capability gap.
    ///
    /// In production, this spawns:
    /// - `tool-engineer` for tool gaps
    /// - `agent-architect` for agent gaps
    ///
    /// This stub returns a descriptive resolution.
    pub fn resolve_gap(&mut self, gap: &CapabilityGap) -> GapResolution {
        if !self.enabled {
            let resolution = GapResolution::Unresolvable {
                reason: "middleware disabled".to_string(),
            };
            return resolution;
        }

        let resolution = if gap.gap_type.is_tool_gap() {
            // Would spawn tool-engineer in production.
            GapResolution::Resolved {
                name: format!("{}_v2", gap.target_name),
            }
        } else {
            // Would spawn agent-architect in production.
            GapResolution::Resolved {
                name: format!("{}_specialized", gap.target_name),
            }
        };

        self.gap_history.push((gap.clone(), resolution.clone()));
        resolution
    }

    /// Escalate to the user when resolution fails.
    pub fn escalate(&mut self, gap: &CapabilityGap) -> GapResolution {
        let resolution = GapResolution::EscalateToUser {
            prompt: format!(
                "Cannot resolve {} gap for '{}': {}. Please provide guidance.",
                if gap.gap_type.is_tool_gap() {
                    "tool"
                } else {
                    "agent"
                },
                gap.target_name,
                gap.desired_capability
            ),
        };
        self.gap_history.push((gap.clone(), resolution.clone()));
        resolution
    }

    /// Get the gap resolution history.
    pub fn history(&self) -> &[(CapabilityGap, GapResolution)] {
        &self.gap_history
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// T-P3-41-01: Detect `ToolNotFound` gap.
    #[test]
    fn test_detect_tool_not_found() {
        let mw = CapabilityGapMiddleware::new(true);
        let gap = mw
            .detect_gap("tool 'format_code' not found", "format_code", "sess-1")
            .unwrap();
        assert_eq!(gap.gap_type, CapabilityGapType::ToolNotFound);
        assert!(gap.gap_type.is_tool_gap());
    }

    /// T-P3-41-02: Detect `AgentNotFound` gap.
    #[test]
    fn test_detect_agent_not_found() {
        let mw = CapabilityGapMiddleware::new(true);
        let gap = mw
            .detect_gap("agent 'code-reviewer' not found", "code-reviewer", "sess-1")
            .unwrap();
        assert_eq!(gap.gap_type, CapabilityGapType::AgentNotFound);
        assert!(gap.gap_type.is_agent_gap());
    }

    /// T-P3-41-03: Resolve tool gap via tool-engineer (stub).
    #[test]
    fn test_resolve_tool_gap() {
        let mut mw = CapabilityGapMiddleware::new(true);
        let gap = CapabilityGap {
            gap_type: CapabilityGapType::ToolNotFound,
            target_name: "format_code".to_string(),
            desired_capability: "code formatting".to_string(),
            session_id: "sess-1".to_string(),
        };
        let resolution = mw.resolve_gap(&gap);
        assert_eq!(
            resolution,
            GapResolution::Resolved {
                name: "format_code_v2".to_string()
            }
        );
        assert_eq!(mw.history().len(), 1);
    }

    /// T-P3-41-04: HITL escalation when resolution fails.
    #[test]
    fn test_escalate_to_user() {
        let mut mw = CapabilityGapMiddleware::new(true);
        let gap = CapabilityGap {
            gap_type: CapabilityGapType::HardcodedConstraint,
            target_name: "deploy_tool".to_string(),
            desired_capability: "deploy to production".to_string(),
            session_id: "sess-1".to_string(),
        };
        let resolution = mw.escalate(&gap);
        assert!(matches!(resolution, GapResolution::EscalateToUser { .. }));
    }

    /// T-P3-41-05: Disabled middleware returns None for detect.
    #[test]
    fn test_disabled_middleware_noop() {
        let mw = CapabilityGapMiddleware::disabled();
        assert!(!mw.is_enabled());
        let gap = mw.detect_gap("tool not found", "x", "s");
        assert!(gap.is_none());
    }

    /// T-P3-41-06: `ParameterMismatch` detection.
    #[test]
    fn test_detect_parameter_mismatch() {
        let mw = CapabilityGapMiddleware::new(true);
        let gap = mw
            .detect_gap(
                "invalid parameter 'output_format' for tool",
                "export_data",
                "sess-1",
            )
            .unwrap();
        assert_eq!(gap.gap_type, CapabilityGapType::ParameterMismatch);
    }
}
