//! Durable dynamic-agent lifecycle orchestration.

use std::path::Path;
use std::sync::Arc;

use chrono::Utc;
use tokio::sync::Mutex;
use y_agent::agent::dynamic_agent::{
    validate_definition, CreatorPermissionSnapshot, DynamicAgentDefinition,
    DynamicAgentStoreBackend,
};
use y_agent::agent::dynamic_agent_proposal::{
    DynamicAgentCandidateDefinition, DynamicAgentEvolutionProposal, DynamicAgentProposalChange,
    DynamicAgentProposalStatus, RegressionEvidence,
};
use y_agent::agent::meta_tools::{
    agent_create, agent_deactivate, agent_search, agent_update, AgentCreateParams,
    AgentDeactivateParams, AgentSearchParams, AgentUpdateParams, MetaToolResult,
};
use y_agent::agent::persistent_dynamic_proposal_store::PersistentDynamicAgentProposalStore;
use y_agent::agent::persistent_dynamic_store::PersistentDynamicAgentStore;
use y_agent::{AgentDefinition, AgentRegistry, MultiAgentError, TrustTier};

use crate::DynamicAgentRegressionFinding;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DynamicAgentProposalDecision {
    Approve,
    Reject,
    Defer,
}

/// Coordinates durable dynamic-agent state with the live delegation registry.
pub struct DynamicAgentService {
    store: Arc<PersistentDynamicAgentStore>,
    proposal_store: Arc<PersistentDynamicAgentProposalStore>,
    registry: Arc<Mutex<AgentRegistry>>,
}

impl DynamicAgentService {
    /// Open the durable store and register every active definition for delegation.
    pub async fn open(
        path: impl AsRef<Path>,
        registry: Arc<Mutex<AgentRegistry>>,
    ) -> Result<Self, DynamicAgentServiceError> {
        let agent_path = path.as_ref().to_path_buf();
        let proposal_path = agent_path.with_file_name("dynamic-agent-proposals.jsonl");
        let store = Arc::new(
            PersistentDynamicAgentStore::open(&agent_path).map_err(|error| store_error(&error))?,
        );
        let proposal_store = Arc::new(
            PersistentDynamicAgentProposalStore::open(proposal_path)
                .map_err(|error| proposal_store_error(&error))?,
        );
        {
            let mut live_registry = registry.lock().await;
            for agent in store.list_active() {
                live_registry
                    .register_dynamic(runtime_definition(&agent))
                    .map_err(DynamicAgentServiceError::Registry)?;
            }
        }
        Ok(Self {
            store,
            proposal_store,
            registry,
        })
    }

    /// Validate, persist, and activate a dynamic agent.
    pub async fn create(
        &self,
        params: AgentCreateParams,
        creator_id: &str,
        creator_snapshot: &CreatorPermissionSnapshot,
    ) -> Result<DynamicAgentDefinition, DynamicAgentServiceError> {
        let result = agent_create(self.store.as_ref(), params, creator_id, creator_snapshot);
        let created = result_agent(result)?;
        if let Err(error) = self
            .registry
            .lock()
            .await
            .register_dynamic(runtime_definition(&created))
        {
            let _ = self.store.deactivate(
                &created.id,
                "runtime registry synchronization failed during creation",
            );
            return Err(DynamicAgentServiceError::Registry(error));
        }
        Ok(created)
    }

    /// Validate, persist, and replace an active dynamic agent definition.
    pub async fn update(
        &self,
        params: AgentUpdateParams,
    ) -> Result<DynamicAgentDefinition, DynamicAgentServiceError> {
        if params.description.is_none()
            && params.mode.is_none()
            && params.allowed_tools.is_none()
            && params.system_prompt.is_none()
        {
            return Err(DynamicAgentServiceError::Operation {
                message: "dynamic-agent update contains no changes".to_string(),
            });
        }
        let result = agent_update(self.store.as_ref(), params);
        let updated = result_agent(result)?;
        self.registry
            .lock()
            .await
            .register_or_override(runtime_definition(&updated))
            .map_err(DynamicAgentServiceError::Registry)?;
        Ok(updated)
    }

    /// Persist deactivation and remove the definition from new delegations.
    pub async fn deactivate(
        &self,
        params: &AgentDeactivateParams,
    ) -> Result<(), DynamicAgentServiceError> {
        if params.reason.trim().is_empty() {
            return Err(DynamicAgentServiceError::Operation {
                message: "dynamic-agent deactivation reason must not be blank".to_string(),
            });
        }
        result_success(agent_deactivate(self.store.as_ref(), params))?;
        match self.registry.lock().await.unregister(&params.id) {
            Ok(_) | Err(MultiAgentError::NotFound { .. }) => Ok(()),
            Err(error) => Err(DynamicAgentServiceError::Registry(error)),
        }
    }

    /// Search active durable dynamic-agent definitions.
    pub fn search(&self, params: &AgentSearchParams) -> Vec<DynamicAgentDefinition> {
        agent_search(self.store.as_ref(), params)
            .agents
            .unwrap_or_default()
    }

    /// Get the latest durable definition, including deactivated agents.
    pub fn get(&self, id: &str) -> Option<DynamicAgentDefinition> {
        self.store.get(id)
    }

    /// Get a committed historical active definition snapshot.
    pub fn get_version(
        &self,
        id: &str,
        version: u64,
    ) -> Result<Option<DynamicAgentDefinition>, DynamicAgentServiceError> {
        self.store
            .get_version(id, version)
            .map_err(|error| store_error(&error))
    }

    /// Count all durable definitions, including deactivated agents.
    pub fn count(&self) -> usize {
        self.store.count()
    }

    /// Diagnostics metadata identifying the active definition version used by
    /// a delegated dynamic-agent execution.
    pub fn execution_trace_metadata(&self, id: &str) -> serde_json::Value {
        self.store.get(id).map_or(serde_json::Value::Null, |agent| {
            serde_json::json!({
                "dynamic_agent": {
                    "id": agent.id,
                    "version": agent.version,
                    "created_by": agent.created_by,
                    "delegation_depth": agent.delegation_depth,
                }
            })
        })
    }

    /// Convert durable regression findings into idempotent pending rollback proposals.
    pub fn propose_regressions(
        &self,
        findings: &[DynamicAgentRegressionFinding],
    ) -> Result<Vec<DynamicAgentEvolutionProposal>, DynamicAgentServiceError> {
        let mut proposals = Vec::new();
        for finding in findings {
            let Some(current) = self.store.get(&finding.agent_id) else {
                continue;
            };
            if current.version != finding.current_version {
                continue;
            }
            if let Some(existing) = self
                .proposal_store
                .find_for_version(&finding.agent_id, finding.current_version)
            {
                proposals.push(existing);
                continue;
            }
            let proposal = DynamicAgentEvolutionProposal::new_regression(
                &finding.agent_id,
                finding.current_version,
                finding.baseline_version,
                RegressionEvidence {
                    baseline_samples: finding.baseline_samples,
                    current_samples: finding.current_samples,
                    baseline_success_rate: finding.baseline_success_rate,
                    current_success_rate: finding.current_success_rate,
                    success_rate_drop: finding.success_rate_drop,
                },
            );
            proposals.push(
                self.proposal_store
                    .create(proposal)
                    .map_err(|error| proposal_store_error(&error))?,
            );
        }
        Ok(proposals)
    }

    pub fn list_proposals(
        &self,
        status: Option<DynamicAgentProposalStatus>,
        agent_id: Option<&str>,
    ) -> Vec<DynamicAgentEvolutionProposal> {
        self.proposal_store
            .list()
            .into_iter()
            .filter(|proposal| status.is_none_or(|expected| proposal.status == expected))
            .filter(|proposal| agent_id.is_none_or(|expected| proposal.agent_id == expected))
            .collect()
    }

    /// Get one durable evolution proposal.
    pub fn get_proposal(&self, id: &str) -> Option<DynamicAgentEvolutionProposal> {
        self.proposal_store.get(id)
    }

    /// Validate and attach a candidate update without changing the live agent.
    pub fn attach_candidate(
        &self,
        proposal_id: &str,
        candidate: DynamicAgentCandidateDefinition,
    ) -> Result<DynamicAgentEvolutionProposal, DynamicAgentServiceError> {
        let mut proposal = self.proposal_store.get(proposal_id).ok_or_else(|| {
            DynamicAgentServiceError::Operation {
                message: format!("dynamic-agent proposal '{proposal_id}' not found"),
            }
        })?;
        if !matches!(
            proposal.status,
            DynamicAgentProposalStatus::Pending | DynamicAgentProposalStatus::Deferred
        ) {
            return invalid_proposal_transition(&proposal, "refine");
        }
        let current = self.store.get(&proposal.agent_id).ok_or_else(|| {
            DynamicAgentServiceError::Operation {
                message: format!("dynamic agent '{}' not found", proposal.agent_id),
            }
        })?;
        if current.version != proposal.current_version {
            return Err(DynamicAgentServiceError::Operation {
                message: format!(
                    "proposal is stale: expected version {}, found {}",
                    proposal.current_version, current.version
                ),
            });
        }
        validate_candidate(&current, &candidate)?;
        proposal.change = DynamicAgentProposalChange::CandidateUpdate { candidate };
        proposal.status = DynamicAgentProposalStatus::Pending;
        proposal.decision_reason = None;
        proposal.failure_message = None;
        self.proposal_store
            .update(proposal)
            .map_err(|error| proposal_store_error(&error))
    }

    pub async fn decide_proposal(
        &self,
        id: &str,
        decision: DynamicAgentProposalDecision,
        reason: Option<String>,
    ) -> Result<DynamicAgentEvolutionProposal, DynamicAgentServiceError> {
        if reason
            .as_deref()
            .is_some_and(|value| value.trim().is_empty())
        {
            return Err(DynamicAgentServiceError::Operation {
                message: "dynamic-agent proposal decision reason must not be blank".to_string(),
            });
        }
        let mut proposal =
            self.proposal_store
                .get(id)
                .ok_or_else(|| DynamicAgentServiceError::Operation {
                    message: format!("dynamic-agent proposal '{id}' not found"),
                })?;

        match decision {
            DynamicAgentProposalDecision::Approve => {
                if !matches!(
                    proposal.status,
                    DynamicAgentProposalStatus::Pending
                        | DynamicAgentProposalStatus::Deferred
                        | DynamicAgentProposalStatus::Approved
                ) {
                    return invalid_proposal_transition(&proposal, "approve");
                }
                if proposal.status != DynamicAgentProposalStatus::Approved {
                    proposal.status = DynamicAgentProposalStatus::Approved;
                    proposal.decided_at = Some(Utc::now().to_rfc3339());
                    proposal.decision_reason = reason;
                    proposal = self
                        .proposal_store
                        .update(proposal)
                        .map_err(|error| proposal_store_error(&error))?;
                }
                self.apply_approved_proposal(proposal).await
            }
            DynamicAgentProposalDecision::Reject => {
                if !matches!(
                    proposal.status,
                    DynamicAgentProposalStatus::Pending | DynamicAgentProposalStatus::Deferred
                ) {
                    return invalid_proposal_transition(&proposal, "reject");
                }
                proposal.status = DynamicAgentProposalStatus::Rejected;
                proposal.decided_at = Some(Utc::now().to_rfc3339());
                proposal.decision_reason = reason;
                self.proposal_store
                    .update(proposal)
                    .map_err(|error| proposal_store_error(&error))
            }
            DynamicAgentProposalDecision::Defer => {
                if proposal.status != DynamicAgentProposalStatus::Pending {
                    return invalid_proposal_transition(&proposal, "defer");
                }
                proposal.status = DynamicAgentProposalStatus::Deferred;
                proposal.decision_reason = reason;
                self.proposal_store
                    .update(proposal)
                    .map_err(|error| proposal_store_error(&error))
            }
        }
    }

    async fn apply_approved_proposal(
        &self,
        mut proposal: DynamicAgentEvolutionProposal,
    ) -> Result<DynamicAgentEvolutionProposal, DynamicAgentServiceError> {
        let Some(current) = self.store.get(&proposal.agent_id) else {
            return self.fail_proposal(proposal, "active agent definition not found");
        };
        if current.version != proposal.current_version {
            let message = format!(
                "proposal is stale: expected version {}, found {}",
                proposal.current_version, current.version
            );
            return self.fail_proposal(proposal, &message);
        }
        let applied = match &proposal.change {
            DynamicAgentProposalChange::Rollback { target_version } => {
                match self.store.rollback(&proposal.agent_id, *target_version) {
                    Ok(restored) => restored,
                    Err(error) => return self.fail_proposal(proposal, &error.to_string()),
                }
            }
            DynamicAgentProposalChange::CandidateUpdate { candidate } => {
                let params = AgentUpdateParams {
                    id: proposal.agent_id.clone(),
                    description: Some(candidate.description.clone()),
                    mode: Some(candidate.mode),
                    allowed_tools: Some(candidate.allowed_tools.clone()),
                    system_prompt: Some(candidate.system_prompt.clone()),
                };
                match result_agent(agent_update(self.store.as_ref(), params)) {
                    Ok(updated) => updated,
                    Err(error) => return self.fail_proposal(proposal, &error.to_string()),
                }
            }
        };
        self.registry
            .lock()
            .await
            .register_or_override(runtime_definition(&applied))
            .map_err(DynamicAgentServiceError::Registry)?;
        proposal.status = DynamicAgentProposalStatus::Applied;
        proposal.applied_version = Some(applied.version);
        proposal.failure_message = None;
        self.proposal_store
            .update(proposal)
            .map_err(|error| proposal_store_error(&error))
    }

    fn fail_proposal(
        &self,
        mut proposal: DynamicAgentEvolutionProposal,
        message: &str,
    ) -> Result<DynamicAgentEvolutionProposal, DynamicAgentServiceError> {
        proposal.status = DynamicAgentProposalStatus::Failed;
        proposal.failure_message = Some(message.to_string());
        let _ = self.proposal_store.update(proposal);
        Err(DynamicAgentServiceError::Operation {
            message: format!("failed to apply dynamic-agent proposal: {message}"),
        })
    }
}

fn validate_candidate(
    current: &DynamicAgentDefinition,
    candidate: &DynamicAgentCandidateDefinition,
) -> Result<(), DynamicAgentServiceError> {
    if candidate.description.trim().is_empty()
        || candidate.system_prompt.trim().is_empty()
        || candidate.rationale.trim().is_empty()
    {
        return Err(DynamicAgentServiceError::Operation {
            message: "dynamic-agent candidate fields must not be blank".to_string(),
        });
    }
    if current.definition.description == candidate.description
        && current.definition.mode == candidate.mode
        && current.definition.allowed_tools == candidate.allowed_tools
        && current.definition.system_prompt == candidate.system_prompt
    {
        return Err(DynamicAgentServiceError::Operation {
            message: "dynamic-agent candidate contains no definition changes".to_string(),
        });
    }

    let mut proposed = current.clone();
    proposed
        .definition
        .description
        .clone_from(&candidate.description);
    proposed.definition.mode = candidate.mode;
    proposed
        .definition
        .allowed_tools
        .clone_from(&candidate.allowed_tools);
    proposed
        .definition
        .system_prompt
        .clone_from(&candidate.system_prompt);
    proposed
        .effective_permissions
        .tools_allowed
        .retain(|tool| candidate.allowed_tools.contains(tool));
    validate_definition(&proposed).map_err(|error| DynamicAgentServiceError::Operation {
        message: format!("invalid dynamic-agent candidate: {error}"),
    })
}

fn runtime_definition(agent: &DynamicAgentDefinition) -> AgentDefinition {
    let mut definition = agent.definition.clone();
    definition.trust_tier = TrustTier::Dynamic;
    definition
        .allowed_tools
        .retain(|tool| agent.effective_permissions.tools_allowed.contains(tool));
    definition.max_iterations =
        usize::try_from(agent.effective_permissions.max_iterations).unwrap_or(usize::MAX);
    definition.max_tool_calls =
        usize::try_from(agent.effective_permissions.max_tool_calls).unwrap_or(usize::MAX);
    if agent.effective_permissions.max_tokens != u64::MAX {
        definition.max_completion_tokens =
            Some(usize::try_from(agent.effective_permissions.max_tokens).unwrap_or(usize::MAX));
    }
    definition
}

fn result_agent(
    result: MetaToolResult,
) -> Result<DynamicAgentDefinition, DynamicAgentServiceError> {
    if result.success {
        result
            .agent
            .ok_or_else(|| DynamicAgentServiceError::Operation {
                message: "dynamic-agent operation succeeded without returning a definition"
                    .to_string(),
            })
    } else {
        Err(DynamicAgentServiceError::Operation {
            message: result.message,
        })
    }
}

fn result_success(result: MetaToolResult) -> Result<(), DynamicAgentServiceError> {
    if result.success {
        Ok(())
    } else {
        Err(DynamicAgentServiceError::Operation {
            message: result.message,
        })
    }
}

fn store_error(error: &MultiAgentError) -> DynamicAgentServiceError {
    DynamicAgentServiceError::Store {
        message: error.to_string(),
    }
}

fn proposal_store_error(error: &MultiAgentError) -> DynamicAgentServiceError {
    DynamicAgentServiceError::Store {
        message: error.to_string(),
    }
}

fn invalid_proposal_transition(
    proposal: &DynamicAgentEvolutionProposal,
    action: &str,
) -> Result<DynamicAgentEvolutionProposal, DynamicAgentServiceError> {
    Err(DynamicAgentServiceError::Operation {
        message: format!(
            "cannot {action} dynamic-agent proposal '{}' in status {:?}",
            proposal.id, proposal.status
        ),
    })
}

/// Dynamic-agent lifecycle failure.
#[derive(Debug, thiserror::Error)]
pub enum DynamicAgentServiceError {
    #[error("dynamic-agent storage failed: {message}")]
    Store { message: String },
    #[error("dynamic-agent registry synchronization failed: {0}")]
    Registry(MultiAgentError),
    #[error("{message}")]
    Operation { message: String },
}
