//! Service-backed handlers for dynamic-agent lifecycle signal tools.

use y_core::trust::TrustTier;
use y_core::types::ToolCallRequest;

use crate::container::ServiceContainer;
use crate::dynamic_agent_service::DynamicAgentProposalDecision;

use super::AgentExecutionConfig;

#[derive(serde::Deserialize)]
#[serde(default)]
struct AgentEvaluateParams {
    agent_id: Option<String>,
    min_samples: usize,
    max_success_rate_drop: f64,
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
struct AgentProposalListParams {
    agent_id: Option<String>,
    status: Option<y_agent::agent::dynamic_agent_proposal::DynamicAgentProposalStatus>,
}

#[derive(serde::Deserialize)]
struct AgentProposalDecideParams {
    proposal_id: String,
    decision: DynamicAgentProposalDecision,
    reason: Option<String>,
}

#[derive(serde::Deserialize)]
struct AgentProposalRefineParams {
    proposal_id: String,
    instructions: Option<String>,
}

impl Default for AgentEvaluateParams {
    fn default() -> Self {
        Self {
            agent_id: None,
            min_samples: 5,
            max_success_rate_drop: 0.25,
        }
    }
}

pub(super) async fn handle(
    container: &ServiceContainer,
    config: &AgentExecutionConfig,
    tc: &ToolCallRequest,
    session_id: &y_core::types::SessionId,
) -> Result<y_core::tool::ToolOutput, y_core::tool::ToolError> {
    use y_agent::agent::meta_tools::{
        AgentCreateParams, AgentDeactivateParams, AgentSearchParams, AgentUpdateParams,
    };

    let content = match tc.name.as_str() {
        "AgentCreate" => {
            let params: AgentCreateParams = parse_arguments(tc)?;
            let snapshot = creator_permission_snapshot(container, config).await?;
            let created = container
                .dynamic_agent_service
                .create(params, &config.agent_name, &snapshot)
                .await
                .map_err(|error| tool_error(&tc.name, &error))?;
            serde_json::json!({
                "message": format!("Agent '{}' created successfully", created.definition.name),
                "agent": created
            })
        }
        "AgentUpdate" => {
            let params: AgentUpdateParams = parse_arguments(tc)?;
            let updated = container
                .dynamic_agent_service
                .update(params)
                .await
                .map_err(|error| tool_error(&tc.name, &error))?;
            serde_json::json!({
                "message": format!("Agent '{}' updated to version {}", updated.id, updated.version),
                "agent": updated
            })
        }
        "AgentDeactivate" => {
            let params: AgentDeactivateParams = parse_arguments(tc)?;
            container
                .dynamic_agent_service
                .deactivate(&params)
                .await
                .map_err(|error| tool_error(&tc.name, &error))?;
            serde_json::json!({
                "message": format!("Agent '{}' deactivated", params.id),
                "id": params.id,
                "status": "deactivated"
            })
        }
        "AgentSearch" => {
            let params: AgentSearchParams = parse_arguments(tc)?;
            let agents = container.dynamic_agent_service.search(&params);
            serde_json::json!({
                "count": agents.len(),
                "agents": agents
            })
        }
        "AgentEvaluate" => {
            let params: AgentEvaluateParams = parse_arguments(tc)?;
            let store = container.diagnostics.store();
            let mut metrics =
                crate::diagnostics::DiagnosticsService::dynamic_agent_version_metrics(
                    store.clone(),
                    None,
                    10_000,
                )
                .await
                .map_err(evaluation_error)?;
            let mut regressions =
                crate::diagnostics::DiagnosticsService::dynamic_agent_regressions(
                    store,
                    None,
                    10_000,
                    params.min_samples,
                    params.max_success_rate_drop,
                )
                .await
                .map_err(evaluation_error)?;
            if let Some(agent_id) = params.agent_id.as_deref() {
                metrics.retain(|metric| metric.agent_id == agent_id);
                regressions.retain(|finding| finding.agent_id == agent_id);
            }
            let regression_count = regressions.len();
            let proposals = container
                .dynamic_agent_service
                .propose_regressions(&regressions)
                .map_err(|error| tool_error(&tc.name, &error))?;
            let proposal_count = proposals.len();
            serde_json::json!({
                "metrics": metrics,
                "regressions": regressions,
                "regression_count": regression_count,
                "proposals": proposals,
                "proposal_count": proposal_count,
                "active_agent_mutation_performed": false,
            })
        }
        "AgentProposalList" => {
            let params: AgentProposalListParams = parse_arguments(tc)?;
            let proposals = container
                .dynamic_agent_service
                .list_proposals(params.status, params.agent_id.as_deref());
            let count = proposals.len();
            serde_json::json!({
                "count": count,
                "proposals": proposals,
            })
        }
        "AgentProposalRefine" => {
            let params: AgentProposalRefineParams = parse_arguments(tc)?;
            if params
                .instructions
                .as_deref()
                .is_some_and(|value| value.trim().is_empty())
            {
                return Err(y_core::tool::ToolError::ValidationError {
                    message: "'instructions' must not be blank when provided".to_string(),
                });
            }
            let refinement = crate::dynamic_agent_refinement::DynamicAgentRefinementService::new(
                std::sync::Arc::clone(&container.dynamic_agent_service),
                std::sync::Arc::clone(&container.agent_delegator),
            );
            let proposal = refinement
                .generate_candidate(
                    &params.proposal_id,
                    params.instructions.as_deref(),
                    uuid::Uuid::parse_str(session_id.as_str()).ok(),
                )
                .await
                .map_err(|error| y_core::tool::ToolError::RuntimeError {
                    name: tc.name.clone(),
                    message: error.to_string(),
                })?;
            serde_json::json!({
                "proposal": proposal,
                "active_agent_mutation_performed": false,
            })
        }
        "AgentProposalDecide" => {
            let params: AgentProposalDecideParams = parse_arguments(tc)?;
            let proposal = container
                .dynamic_agent_service
                .decide_proposal(&params.proposal_id, params.decision, params.reason)
                .await
                .map_err(|error| tool_error(&tc.name, &error))?;
            let status = proposal.status;
            let active_agent_mutation_performed = proposal.applied_version.is_some();
            serde_json::json!({
                "proposal": proposal,
                "status": status,
                "active_agent_mutation_performed": active_agent_mutation_performed,
            })
        }
        _ => unreachable!("dynamic-agent tool names are matched before dispatch"),
    };

    Ok(y_core::tool::ToolOutput {
        success: true,
        content,
        warnings: vec![],
        metadata: serde_json::json!({ "action": tc.name }),
    })
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

async fn creator_permission_snapshot(
    container: &ServiceContainer,
    config: &AgentExecutionConfig,
) -> Result<y_agent::agent::dynamic_agent::CreatorPermissionSnapshot, y_core::tool::ToolError> {
    use y_agent::agent::dynamic_agent::CreatorPermissionSnapshot;

    let current_tools = execution_tool_names(config);
    if config.trust_tier == Some(TrustTier::Dynamic) {
        let creator = container
            .dynamic_agent_service
            .get(&config.agent_name)
            .ok_or_else(|| y_core::tool::ToolError::RuntimeError {
                name: "AgentCreate".to_string(),
                message: format!(
                    "dynamic creator '{}' is missing from the durable agent store",
                    config.agent_name
                ),
            })?;
        let tools_allowed = creator
            .effective_permissions
            .tools_allowed
            .iter()
            .filter(|tool| current_tools.contains(tool))
            .cloned()
            .collect();
        return Ok(CreatorPermissionSnapshot {
            tools_allowed,
            max_iterations: creator
                .effective_permissions
                .max_iterations
                .min(u32::try_from(config.max_iterations).unwrap_or(u32::MAX)),
            max_tool_calls: creator
                .effective_permissions
                .max_tool_calls
                .min(u32::try_from(config.max_tool_calls).unwrap_or(u32::MAX)),
            max_tokens: creator
                .effective_permissions
                .max_tokens
                .min(config.max_tokens.map_or(u64::MAX, u64::from)),
            delegation_depth: creator.effective_permissions.delegation_depth,
        });
    }

    let delegation_depth =
        u32::try_from(container.agent_pool.read().await.max_delegation_depth()).unwrap_or(u32::MAX);
    Ok(CreatorPermissionSnapshot {
        tools_allowed: current_tools,
        max_iterations: u32::try_from(config.max_iterations).unwrap_or(u32::MAX),
        max_tool_calls: u32::try_from(config.max_tool_calls).unwrap_or(u32::MAX),
        max_tokens: config.max_tokens.map_or(u64::MAX, u64::from),
        delegation_depth,
    })
}

fn execution_tool_names(config: &AgentExecutionConfig) -> Vec<String> {
    let mut names = if config.trust_tier.is_some() {
        config.agent_allowed_tools.clone()
    } else {
        config
            .tool_definitions
            .iter()
            .filter_map(|definition| {
                definition
                    .pointer("/function/name")
                    .or_else(|| definition.get("name"))
                    .and_then(serde_json::Value::as_str)
                    .map(ToOwned::to_owned)
            })
            .collect()
    };
    names.sort();
    names.dedup();
    names
}

fn tool_error(
    name: &str,
    error: &crate::dynamic_agent_service::DynamicAgentServiceError,
) -> y_core::tool::ToolError {
    y_core::tool::ToolError::RuntimeError {
        name: name.to_string(),
        message: error.to_string(),
    }
}

fn evaluation_error(message: String) -> y_core::tool::ToolError {
    y_core::tool::ToolError::RuntimeError {
        name: "AgentEvaluate".to_string(),
        message,
    }
}
