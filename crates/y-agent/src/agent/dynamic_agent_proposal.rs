//! Governed evolution proposal types for dynamic agents.

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::agent::definition::AgentMode;

/// Durable lifecycle state of a dynamic-agent evolution proposal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DynamicAgentProposalStatus {
    Pending,
    Approved,
    Rejected,
    Deferred,
    Applied,
    Failed,
}

impl DynamicAgentProposalStatus {
    pub fn is_open(self) -> bool {
        matches!(self, Self::Pending | Self::Approved | Self::Deferred)
    }
}

/// Repeated execution evidence supporting a regression proposal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegressionEvidence {
    pub baseline_samples: usize,
    pub current_samples: usize,
    pub baseline_success_rate: f64,
    pub current_success_rate: f64,
    pub success_rate_drop: f64,
}

/// Immutable-field-safe candidate definition drafted for a dynamic agent.
///
/// Identity, creator metadata, trust, budgets, and delegation depth are not
/// represented here, so refinement cannot alter them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DynamicAgentCandidateDefinition {
    pub description: String,
    pub mode: AgentMode,
    pub allowed_tools: Vec<String>,
    pub system_prompt: String,
    pub rationale: String,
}

/// Proposed reversible mutation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DynamicAgentProposalChange {
    Rollback {
        target_version: u64,
    },
    CandidateUpdate {
        candidate: DynamicAgentCandidateDefinition,
    },
}

/// Append-only state snapshot for one governed dynamic-agent proposal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DynamicAgentEvolutionProposal {
    pub id: String,
    pub agent_id: String,
    pub current_version: u64,
    pub baseline_version: u64,
    pub evidence: RegressionEvidence,
    pub change: DynamicAgentProposalChange,
    pub status: DynamicAgentProposalStatus,
    pub revision: u64,
    pub created_at: String,
    pub decided_at: Option<String>,
    pub decision_reason: Option<String>,
    pub applied_version: Option<u64>,
    pub failure_message: Option<String>,
}

impl DynamicAgentEvolutionProposal {
    pub fn new_regression(
        agent_id: impl Into<String>,
        current_version: u64,
        baseline_version: u64,
        evidence: RegressionEvidence,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            agent_id: agent_id.into(),
            current_version,
            baseline_version,
            evidence,
            change: DynamicAgentProposalChange::Rollback {
                target_version: baseline_version,
            },
            status: DynamicAgentProposalStatus::Pending,
            revision: 1,
            created_at: Utc::now().to_rfc3339(),
            decided_at: None,
            decision_reason: None,
            applied_version: None,
            failure_message: None,
        }
    }
}
