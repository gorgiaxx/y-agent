use tempfile::tempdir;
use y_agent::agent::dynamic_agent_proposal::{
    DynamicAgentCandidateDefinition, DynamicAgentEvolutionProposal, DynamicAgentProposalChange,
    DynamicAgentProposalStatus, RegressionEvidence,
};
use y_agent::agent::persistent_dynamic_proposal_store::PersistentDynamicAgentProposalStore;
use y_agent::AgentMode;

fn proposal() -> DynamicAgentEvolutionProposal {
    DynamicAgentEvolutionProposal::new_regression(
        "dyn-code-scout",
        2,
        1,
        RegressionEvidence {
            baseline_samples: 5,
            current_samples: 5,
            baseline_success_rate: 1.0,
            current_success_rate: 0.2,
            success_rate_drop: 0.8,
        },
    )
}

#[test]
fn proposal_state_replays_latest_append_only_revision() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("dynamic-agent-proposals.jsonl");
    let store = PersistentDynamicAgentProposalStore::open(&path).unwrap();

    let created = store.create(proposal()).unwrap();
    assert_eq!(created.revision, 1);
    assert!(store
        .find_open(&created.agent_id, created.current_version)
        .is_some());

    let mut approved = created.clone();
    approved.status = DynamicAgentProposalStatus::Approved;
    approved.decision_reason = Some("Regression evidence is sufficient".to_string());
    let approved = store.update(approved).unwrap();
    assert_eq!(approved.revision, 2);
    drop(store);

    let reopened = PersistentDynamicAgentProposalStore::open(&path).unwrap();
    let replayed = reopened.get(&created.id).unwrap();
    assert_eq!(replayed.status, DynamicAgentProposalStatus::Approved);
    assert_eq!(replayed.revision, 2);
    assert_eq!(reopened.list().len(), 1);
}

#[test]
fn applied_proposals_are_not_returned_as_open() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("dynamic-agent-proposals.jsonl");
    let store = PersistentDynamicAgentProposalStore::open(&path).unwrap();
    let created = store.create(proposal()).unwrap();

    let mut applied = created.clone();
    applied.status = DynamicAgentProposalStatus::Applied;
    applied.applied_version = Some(3);
    store.update(applied).unwrap();

    assert!(store
        .find_open(&created.agent_id, created.current_version)
        .is_none());
}

#[test]
fn candidate_update_replays_with_the_latest_proposal_revision() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("dynamic-agent-proposals.jsonl");
    let store = PersistentDynamicAgentProposalStore::open(&path).unwrap();
    let mut created = store.create(proposal()).unwrap();
    created.change = DynamicAgentProposalChange::CandidateUpdate {
        candidate: DynamicAgentCandidateDefinition {
            description: "Find implementation evidence with explicit citations".to_string(),
            mode: AgentMode::Explore,
            allowed_tools: vec!["FileRead".to_string()],
            system_prompt: "Inspect evidence and cite the relevant files.".to_string(),
            rationale: "Reduce unsupported conclusions after the observed regression".to_string(),
        },
    };
    let updated = store.update(created).unwrap();
    drop(store);

    let reopened = PersistentDynamicAgentProposalStore::open(&path).unwrap();
    assert_eq!(reopened.get(&updated.id).unwrap().change, updated.change);
}
