//! Durable journal for governed skill-evolution proposals.

use std::collections::BTreeMap;
use std::path::Path;

use crate::error::SkillModuleError;
use crate::evolution::EvolutionProposal;
use crate::jsonl_journal::JsonlJournal;

/// Append-only proposal journal where later records supersede earlier states.
#[derive(Debug, Clone)]
pub struct ProposalJournal {
    inner: JsonlJournal,
}

impl ProposalJournal {
    /// Open or create a proposal journal at `path`.
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, SkillModuleError> {
        Ok(Self {
            inner: JsonlJournal::open(path, "evolution proposal").await?,
        })
    }

    /// Append a proposal state snapshot and sync it to durable storage.
    pub async fn append(&self, proposal: &EvolutionProposal) -> Result<(), SkillModuleError> {
        self.inner.append(proposal).await
    }

    /// Load every proposal state snapshot in append order.
    pub async fn load_all(&self) -> Result<Vec<EvolutionProposal>, SkillModuleError> {
        self.inner.load_all().await
    }

    /// Load the latest state of each proposal, ordered by proposal ID.
    pub async fn load_latest(&self) -> Result<Vec<EvolutionProposal>, SkillModuleError> {
        let mut latest = BTreeMap::new();
        for proposal in self.load_all().await? {
            latest.insert(proposal.id.clone(), proposal);
        }
        Ok(latest.into_values().collect())
    }

    /// Filesystem path backing this journal.
    pub fn path(&self) -> &Path {
        self.inner.path()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evolution::{EvolutionProposal, PatternType, ProposalStatus};

    #[tokio::test]
    async fn test_proposal_journal_latest_state_survives_reopen() {
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join("evolution/proposals.jsonl");
        let journal = ProposalJournal::open(&path).await.unwrap();
        let mut proposal = proposal();
        journal.append(&proposal).await.unwrap();
        proposal.status = ProposalStatus::Approved;
        journal.append(&proposal).await.unwrap();
        drop(journal);

        let reopened = ProposalJournal::open(&path).await.unwrap();
        let latest = reopened.load_latest().await.unwrap();

        assert_eq!(latest.len(), 1);
        assert_eq!(latest[0].id, "proposal-1");
        assert_eq!(latest[0].status, ProposalStatus::Approved);
    }

    fn proposal() -> EvolutionProposal {
        EvolutionProposal {
            id: "proposal-1".to_string(),
            skill_name: "review-rust".to_string(),
            current_version: "v1".to_string(),
            proposed_changes: "Handle repeated ownership errors".to_string(),
            patterns: vec![PatternType::CommonError],
            status: ProposalStatus::PendingApproval,
            proposed_version: None,
            baseline_version: None,
            change_type: None,
            patterns_referenced: vec!["pattern-1".to_string()],
            diff_preview: String::new(),
            candidate_root_content: None,
            candidate_rationale: None,
            decision_reason: None,
            deferred_until: None,
        }
    }
}
