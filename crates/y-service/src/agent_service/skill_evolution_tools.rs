//! Service-backed handlers for governed skill-evolution signal tools.

use std::collections::HashSet;

use y_core::types::ToolCallRequest;
use y_skills::evolution::{EvolutionProposal, ProposalStatus};

use crate::container::ServiceContainer;
use crate::skill_evolution_service::{PromotionResources, SkillProposalDecision};

#[derive(serde::Deserialize)]
#[serde(default)]
struct SkillProposalListParams {
    skill_name: Option<String>,
    status: Option<ProposalStatus>,
    limit: usize,
}

impl Default for SkillProposalListParams {
    fn default() -> Self {
        Self {
            skill_name: None,
            status: None,
            limit: 20,
        }
    }
}

#[derive(serde::Deserialize)]
struct SkillProposalRefineParams {
    proposal_id: String,
    instructions: Option<String>,
}

#[derive(serde::Deserialize)]
struct SkillProposalDecideParams {
    proposal_id: String,
    decision: SkillProposalDecision,
    reason: Option<String>,
}

pub(super) async fn handle(
    container: &ServiceContainer,
    tc: &ToolCallRequest,
    session_id: &y_core::types::SessionId,
) -> Result<y_core::tool::ToolOutput, y_core::tool::ToolError> {
    let content = match tc.name.as_str() {
        "SkillProposalList" => {
            let params: SkillProposalListParams = parse_arguments(tc)?;
            let mut proposals = container
                .skill_evolution_service
                .load_proposals()
                .await
                .map_err(|error| tool_error(&tc.name, &error))?;
            if let Some(skill_name) = normalized(params.skill_name.as_deref()) {
                proposals.retain(|proposal| proposal.skill_name == skill_name);
            }
            if let Some(status) = params.status {
                proposals.retain(|proposal| proposal.status == status);
            }
            proposals.truncate(params.limit.clamp(1, 100));
            let summaries: Vec<_> = proposals.iter().map(proposal_summary).collect();
            serde_json::json!({
                "count": summaries.len(),
                "proposals": summaries,
            })
        }
        "SkillProposalRefine" => {
            let params: SkillProposalRefineParams = parse_arguments(tc)?;
            if params
                .instructions
                .as_deref()
                .is_some_and(|value| value.trim().is_empty())
            {
                return Err(y_core::tool::ToolError::ValidationError {
                    message: "'instructions' must not be blank when provided".to_string(),
                });
            }
            let refinement =
                crate::skill_evolution_refinement::SkillEvolutionRefinementService::new(
                    std::sync::Arc::clone(&container.skill_evolution_service),
                    std::sync::Arc::clone(&container.agent_delegator),
                );
            let proposal = refinement
                .generate_candidate(
                    &params.proposal_id,
                    params.instructions.as_deref(),
                    promotion_resources(container).await,
                    uuid::Uuid::parse_str(session_id.as_str()).ok(),
                )
                .await
                .map_err(|error| y_core::tool::ToolError::RuntimeError {
                    name: tc.name.clone(),
                    message: error.to_string(),
                })?;
            serde_json::json!({
                "proposal": proposal,
                "active_skill_mutation_performed": false,
            })
        }
        "SkillProposalDecide" => {
            let params: SkillProposalDecideParams = parse_arguments(tc)?;
            let proposal = container
                .skill_evolution_service
                .decide_proposal(
                    &params.proposal_id,
                    params.decision,
                    params.reason,
                    promotion_resources(container).await,
                )
                .await
                .map_err(|error| tool_error(&tc.name, &error))?;
            let active_skill_mutation_performed = proposal.status == ProposalStatus::Promoted;
            if active_skill_mutation_performed {
                container.refresh_skill_search().await;
            }
            serde_json::json!({
                "proposal": proposal,
                "active_skill_mutation_performed": active_skill_mutation_performed,
            })
        }
        _ => unreachable!("skill-evolution tool names are matched before dispatch"),
    };

    Ok(y_core::tool::ToolOutput {
        success: true,
        content,
        warnings: vec![],
        metadata: serde_json::json!({ "action": tc.name }),
    })
}

async fn promotion_resources(container: &ServiceContainer) -> PromotionResources {
    let registered_tools = container
        .tool_registry
        .get_all_definitions()
        .await
        .into_iter()
        .map(|definition| definition.name.as_str().to_string())
        .collect::<HashSet<_>>();
    let registered_knowledge = container
        .knowledge_service
        .lock()
        .await
        .list_collections()
        .into_iter()
        .map(|collection| collection.name.clone())
        .collect();
    PromotionResources {
        registered_tools,
        registered_knowledge,
    }
}

fn proposal_summary(proposal: &EvolutionProposal) -> serde_json::Value {
    serde_json::json!({
        "id": proposal.id,
        "skill_name": proposal.skill_name,
        "current_version": proposal.current_version,
        "proposed_changes": proposal.proposed_changes,
        "patterns": proposal.patterns,
        "status": proposal.status,
        "has_candidate": proposal.candidate_root_content.is_some(),
        "candidate_rationale": proposal.candidate_rationale,
        "diff_preview": proposal.diff_preview,
        "proposed_version": proposal.proposed_version,
        "baseline_version": proposal.baseline_version,
        "decision_reason": proposal.decision_reason,
    })
}

fn normalized(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn parse_arguments<T: serde::de::DeserializeOwned>(
    tc: &ToolCallRequest,
) -> Result<T, y_core::tool::ToolError> {
    serde_json::from_value(tc.arguments.clone()).map_err(|error| {
        y_core::tool::ToolError::ValidationError {
            message: format!("invalid {} arguments: {error}", tc.name),
        }
    })
}

fn tool_error(name: &str, error: &y_skills::SkillModuleError) -> y_core::tool::ToolError {
    y_core::tool::ToolError::RuntimeError {
        name: name.to_string(),
        message: error.to_string(),
    }
}
