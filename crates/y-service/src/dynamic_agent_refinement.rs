//! Governed candidate generation for dynamic-agent evolution proposals.

use std::sync::Arc;

use y_agent::agent::dynamic_agent_proposal::{
    DynamicAgentCandidateDefinition, DynamicAgentEvolutionProposal,
};
use y_core::agent::{AgentDelegator, ContextStrategyHint};

use crate::dynamic_agent_service::DynamicAgentService;

/// Delegates candidate drafting to the read-only `agent-refiner` and stores
/// only candidates that pass the dynamic-agent validation pipeline.
pub struct DynamicAgentRefinementService {
    dynamic_agents: Arc<DynamicAgentService>,
    delegator: Arc<dyn AgentDelegator>,
}

impl DynamicAgentRefinementService {
    pub fn new(
        dynamic_agents: Arc<DynamicAgentService>,
        delegator: Arc<dyn AgentDelegator>,
    ) -> Self {
        Self {
            dynamic_agents,
            delegator,
        }
    }

    pub async fn generate_candidate(
        &self,
        proposal_id: &str,
        instructions: Option<&str>,
        session_id: Option<uuid::Uuid>,
    ) -> Result<DynamicAgentEvolutionProposal, DynamicAgentRefinementError> {
        let proposal = self
            .dynamic_agents
            .get_proposal(proposal_id)
            .ok_or_else(|| DynamicAgentRefinementError::NotFound {
                message: format!("dynamic-agent proposal '{proposal_id}' not found"),
            })?;
        let current = self.dynamic_agents.get(&proposal.agent_id).ok_or_else(|| {
            DynamicAgentRefinementError::NotFound {
                message: format!("dynamic agent '{}' not found", proposal.agent_id),
            }
        })?;
        let baseline = self
            .dynamic_agents
            .get_version(&proposal.agent_id, proposal.baseline_version)
            .map_err(|error| service_error(&error))?
            .ok_or_else(|| DynamicAgentRefinementError::NotFound {
                message: format!(
                    "dynamic agent '{}@{}' not found",
                    proposal.agent_id, proposal.baseline_version
                ),
            })?;
        let input = serde_json::json!({
            "task": "Draft a validated candidate update for this regressed dynamic agent. Return JSON only.",
            "proposal": proposal,
            "current_definition": versioned_definition(&current),
            "baseline_definition": versioned_definition(&baseline),
            "constraints": {
                "allowed_tools": current.effective_permissions.tools_allowed,
                "immutable_fields": [
                    "id", "name", "trust_tier", "source", "created_by",
                    "delegation_depth", "effective_permissions", "version", "status"
                ],
                "active_mutation_allowed": false,
            },
            "reviewer_instructions": instructions,
        });
        let output = self
            .delegator
            .delegate(
                "agent-refiner",
                input,
                ContextStrategyHint::None,
                session_id,
            )
            .await
            .map_err(|error| DynamicAgentRefinementError::Delegation {
                message: error.to_string(),
            })?;
        let candidate: DynamicAgentCandidateDefinition = serde_json::from_str(
            crate::skill_ingestion::extract_json_from_response(&output.text),
        )
        .map_err(|error| DynamicAgentRefinementError::InvalidOutput {
            message: error.to_string(),
        })?;
        self.dynamic_agents
            .attach_candidate(proposal_id, candidate)
            .map_err(|error| service_error(&error))
    }
}

fn versioned_definition(
    agent: &y_agent::agent::dynamic_agent::DynamicAgentDefinition,
) -> serde_json::Value {
    serde_json::json!({
        "version": agent.version,
        "description": agent.definition.description,
        "mode": agent.definition.mode,
        "allowed_tools": agent.definition.allowed_tools,
        "system_prompt": agent.definition.system_prompt,
    })
}

fn service_error(
    error: &crate::dynamic_agent_service::DynamicAgentServiceError,
) -> DynamicAgentRefinementError {
    DynamicAgentRefinementError::Validation {
        message: error.to_string(),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DynamicAgentRefinementError {
    #[error("{message}")]
    NotFound { message: String },
    #[error("dynamic-agent refinement delegation failed: {message}")]
    Delegation { message: String },
    #[error("agent-refiner returned invalid output: {message}")]
    InvalidOutput { message: String },
    #[error("dynamic-agent candidate validation failed: {message}")]
    Validation { message: String },
}
