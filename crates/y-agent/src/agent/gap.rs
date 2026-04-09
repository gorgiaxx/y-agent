//! Capability gap detection and resolution.
//!
//! Design reference: multi-agent-design.md §`CapabilityGap` Middleware
//!
//! When a delegation cannot find a suitable agent, the gap detector
//! classifies the gap type and attempts auto-resolution via the
//! `agent-architect` built-in agent.

use crate::agent::definition::AgentMode;
use crate::agent::registry::AgentRegistry;

// ---------------------------------------------------------------------------
// Gap types
// ---------------------------------------------------------------------------

/// Types of capability gaps detected during delegation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentGapType {
    /// No agent with the requested ID exists in the registry.
    AgentNotFound { agent_id: String },
    /// Agent exists but lacks required capabilities for the task.
    CapabilityMismatch {
        agent_id: String,
        required_capabilities: Vec<String>,
        available_capabilities: Vec<String>,
    },
    /// Agent exists but its mode is inappropriate for the task.
    ModeInappropriate {
        agent_id: String,
        requested_mode: AgentMode,
        agent_mode: AgentMode,
    },
}

// ---------------------------------------------------------------------------
// Gap resolution
// ---------------------------------------------------------------------------

/// Result of a gap resolution attempt.
#[derive(Debug, Clone)]
pub enum GapResolution {
    /// Gap resolved: use this agent ID for the delegation.
    Resolved { agent_id: String },
    /// Gap requires human-in-the-loop intervention.
    HitlRequired { gap: AgentGapType, reason: String },
}

// ---------------------------------------------------------------------------
// Gap detector
// ---------------------------------------------------------------------------

/// Detects and classifies agent capability gaps.
pub struct AgentGapDetector;

impl AgentGapDetector {
    /// Detect if there's a gap for the requested agent and task.
    pub fn detect(
        registry: &AgentRegistry,
        agent_id: &str,
        required_capabilities: &[String],
        requested_mode: Option<AgentMode>,
    ) -> Option<AgentGapType> {
        let Some(definition) = registry.get(agent_id) else {
            return Some(AgentGapType::AgentNotFound {
                agent_id: agent_id.to_string(),
            });
        };

        // Check capabilities
        if !required_capabilities.is_empty() {
            let missing: Vec<String> = required_capabilities
                .iter()
                .filter(|cap| !definition.capabilities.contains(cap))
                .cloned()
                .collect();

            if !missing.is_empty() {
                return Some(AgentGapType::CapabilityMismatch {
                    agent_id: agent_id.to_string(),
                    required_capabilities: missing,
                    available_capabilities: definition.capabilities.clone(),
                });
            }
        }

        // Check mode compatibility
        if let Some(requested) = requested_mode {
            // If requested mode requires write tools but agent is plan/explore only
            let mode_compatible = !matches!(
                (requested, definition.mode),
                (AgentMode::Build, AgentMode::Plan | AgentMode::Explore)
            );

            if !mode_compatible {
                return Some(AgentGapType::ModeInappropriate {
                    agent_id: agent_id.to_string(),
                    requested_mode: requested,
                    agent_mode: definition.mode,
                });
            }
        }

        None
    }

    /// Attempt to resolve a capability gap.
    ///
    /// For `AgentNotFound` gaps, triggers `agent-architect` to design
    /// a new definition. For other gaps, falls back to HITL.
    pub fn resolve(registry: &AgentRegistry, gap: &AgentGapType) -> GapResolution {
        match gap {
            AgentGapType::AgentNotFound { agent_id } => {
                // Check if agent-architect is available for auto-resolution
                if registry.get("agent-architect").is_some() {
                    // In a full implementation, this would:
                    // 1. Spawn agent-architect
                    // 2. Have it design a new agent definition
                    // 3. Register the new definition
                    // 4. Return the new agent_id
                    //
                    // For now, signal that HITL is needed since we can't
                    // actually run the agent loop here.
                    GapResolution::HitlRequired {
                        gap: gap.clone(),
                        reason: format!(
                            "Agent '{agent_id}' not found. agent-architect available for \
                             auto-design, but requires orchestrator integration to execute."
                        ),
                    }
                } else {
                    GapResolution::HitlRequired {
                        gap: gap.clone(),
                        reason: format!(
                            "Agent '{agent_id}' not found and agent-architect not available."
                        ),
                    }
                }
            }
            AgentGapType::CapabilityMismatch {
                agent_id,
                required_capabilities,
                ..
            } => GapResolution::HitlRequired {
                gap: gap.clone(),
                reason: format!(
                    "Agent '{agent_id}' missing capabilities: {}. \
                         Consider creating a specialized agent.",
                    required_capabilities.join(", ")
                ),
            },
            AgentGapType::ModeInappropriate {
                agent_id,
                requested_mode,
                agent_mode,
            } => GapResolution::HitlRequired {
                gap: gap.clone(),
                reason: format!(
                    "Agent '{agent_id}' is in {agent_mode:?} mode but {requested_mode:?} was requested. \
                         Use mode_override in the delegation instead."
                ),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::definition::{AgentDefinition, ContextStrategy};
    use crate::agent::trust::TrustTier;

    fn registry_with_agent() -> AgentRegistry {
        let mut registry = AgentRegistry::new();
        let def = AgentDefinition {
            id: "code-reviewer".to_string(),
            name: "Code Reviewer".to_string(),
            description: "Reviews code".to_string(),
            mode: AgentMode::Plan,
            trust_tier: TrustTier::UserDefined,
            capabilities: vec!["code_review".to_string(), "static_analysis".to_string()],
            allowed_tools: vec!["FileRead".to_string(), "SearchCode".to_string()],
            denied_tools: vec![],
            system_prompt: "Review code.".to_string(),
            skills: vec![],
            preferred_models: vec![],
            fallback_models: vec![],
            provider_tags: vec![],
            temperature: None,
            top_p: None,
            max_iterations: 20,
            max_tool_calls: 50,
            timeout_secs: 300,
            context_sharing: ContextStrategy::Summary,
            max_context_tokens: 4096,
            max_completion_tokens: None,
            user_callable: false,
            prune_tool_history: false,
            auto_update: true,
        };
        registry.register(def).unwrap();
        registry
    }

    /// T-MA-R5-01: `AgentNotFound` gap triggers potential agent-architect.
    #[test]
    fn test_agent_not_found_gap() {
        let registry = registry_with_agent();

        let gap = AgentGapDetector::detect(&registry, "nonexistent-agent", &[], None);
        assert!(gap.is_some());
        assert!(matches!(gap.unwrap(), AgentGapType::AgentNotFound { .. }));
    }

    /// T-MA-R5-02: `ModeInappropriate` gap classified correctly.
    #[test]
    fn test_mode_inappropriate_gap() {
        let registry = registry_with_agent();

        // code-reviewer is Plan mode; requesting Build should fail
        let gap = AgentGapDetector::detect(&registry, "code-reviewer", &[], Some(AgentMode::Build));
        assert!(gap.is_some());
        match gap.unwrap() {
            AgentGapType::ModeInappropriate {
                agent_id,
                requested_mode,
                agent_mode,
            } => {
                assert_eq!(agent_id, "code-reviewer");
                assert_eq!(requested_mode, AgentMode::Build);
                assert_eq!(agent_mode, AgentMode::Plan);
            }
            other => panic!("expected ModeInappropriate, got: {other:?}"),
        }
    }

    /// T-MA-R5-03: Gap resolution failure falls back to HITL.
    #[test]
    fn test_gap_resolution_hitl_fallback() {
        let registry = registry_with_agent();

        let gap = AgentGapType::AgentNotFound {
            agent_id: "missing-agent".to_string(),
        };
        let resolution = AgentGapDetector::resolve(&registry, &gap);

        assert!(matches!(resolution, GapResolution::HitlRequired { .. }));
    }

    /// Capability mismatch detected when required capabilities are missing.
    #[test]
    fn test_capability_mismatch() {
        let registry = registry_with_agent();

        let gap = AgentGapDetector::detect(
            &registry,
            "code-reviewer",
            &["ml_training".to_string()],
            None,
        );
        assert!(gap.is_some());
        match gap.unwrap() {
            AgentGapType::CapabilityMismatch {
                required_capabilities,
                ..
            } => {
                assert!(required_capabilities.contains(&"ml_training".to_string()));
            }
            other => panic!("expected CapabilityMismatch, got: {other:?}"),
        }
    }

    /// No gap when agent matches requirements.
    #[test]
    fn test_no_gap_when_matching() {
        let registry = registry_with_agent();

        let gap = AgentGapDetector::detect(
            &registry,
            "code-reviewer",
            &["code_review".to_string()],
            Some(AgentMode::Plan),
        );
        assert!(gap.is_none());
    }
}
