//! Governed candidate generation for durable skill-evolution proposals.

use std::sync::Arc;

use y_core::agent::{AgentDelegator, ContextStrategyHint};
use y_skills::evolution::{EvolutionProposal, ProposalStatus};

use crate::skill_evolution_service::{PromotionResources, SkillEvolutionService};

#[derive(Debug, serde::Deserialize)]
struct SkillRefinerOutput {
    root_content: String,
    rationale: String,
}

/// Delegates candidate drafting to the tool-free `skill-refiner` and persists
/// only candidates that pass normal skill validation.
pub struct SkillEvolutionRefinementService {
    evolution: Arc<SkillEvolutionService>,
    delegator: Arc<dyn AgentDelegator>,
}

impl SkillEvolutionRefinementService {
    pub fn new(evolution: Arc<SkillEvolutionService>, delegator: Arc<dyn AgentDelegator>) -> Self {
        Self {
            evolution,
            delegator,
        }
    }

    pub async fn generate_candidate(
        &self,
        proposal_id: &str,
        instructions: Option<&str>,
        resources: PromotionResources,
        session_id: Option<uuid::Uuid>,
    ) -> Result<EvolutionProposal, SkillEvolutionRefinementError> {
        let proposal = self
            .evolution
            .get_proposal(proposal_id)
            .await
            .map_err(|error| service_error(&error))?;
        if !matches!(
            proposal.status,
            ProposalStatus::PendingApproval | ProposalStatus::Deferred
        ) {
            return Err(SkillEvolutionRefinementError::Validation {
                message: format!(
                    "skill proposal cannot be refined in status {:?}: {proposal_id}",
                    proposal.status
                ),
            });
        }
        let current = self
            .evolution
            .load_active_skill(&proposal.skill_name)
            .map_err(|error| service_error(&error))?;
        let evidence = self
            .evolution
            .recent_skill_experiences(&proposal.skill_name, 20)
            .await
            .map_err(|error| service_error(&error))?;
        let input = serde_json::json!({
            "proposal": proposal,
            "current_skill": {
                "name": current.name,
                "description": current.description,
                "version": current.version,
                "root_content": current.root_content,
                "token_estimate": current.token_estimate,
                "classification": current.classification,
                "constraints": current.constraints,
                "references": current.references,
            },
            "evidence": evidence,
            "constraints": {
                "active_mutation_allowed": false,
                "preserve_identity": true,
                "root_token_limit": 2_000,
                "instruction_only": true,
                "executable_code_allowed": false,
            },
            "reviewer_instructions": instructions,
        });
        let output = self
            .delegator
            .delegate(
                "skill-refiner",
                input,
                ContextStrategyHint::None,
                session_id,
            )
            .await
            .map_err(|error| SkillEvolutionRefinementError::Delegation {
                message: error.to_string(),
            })?;
        let candidate: SkillRefinerOutput = serde_json::from_str(
            crate::skill_ingestion::extract_json_from_response(&output.text),
        )
        .map_err(|error| SkillEvolutionRefinementError::InvalidOutput {
            message: error.to_string(),
        })?;
        self.evolution
            .attach_candidate(
                proposal_id,
                candidate.root_content,
                candidate.rationale,
                &resources,
            )
            .await
            .map_err(|error| service_error(&error))
    }
}

fn service_error(error: &y_skills::SkillModuleError) -> SkillEvolutionRefinementError {
    SkillEvolutionRefinementError::Validation {
        message: error.to_string(),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SkillEvolutionRefinementError {
    #[error("skill-refiner delegation failed: {message}")]
    Delegation { message: String },
    #[error("skill-refiner returned invalid output: {message}")]
    InvalidOutput { message: String },
    #[error("skill candidate validation failed: {message}")]
    Validation { message: String },
}
