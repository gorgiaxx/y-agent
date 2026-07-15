use std::sync::Arc;

use tempfile::tempdir;
use tokio::sync::Mutex;
use y_agent::agent::dynamic_agent::{AgentStatus, CreatorPermissionSnapshot};
use y_agent::agent::dynamic_agent_proposal::DynamicAgentProposalStatus;
use y_agent::agent::dynamic_agent_proposal::{
    DynamicAgentCandidateDefinition, DynamicAgentProposalChange,
};
use y_agent::agent::meta_tools::{AgentCreateParams, AgentDeactivateParams, AgentUpdateParams};
use y_agent::{AgentMode, AgentRegistry, ContextStrategy};
use y_service::dynamic_agent_service::{DynamicAgentProposalDecision, DynamicAgentService};
use y_service::DynamicAgentRegressionFinding;

fn creator_snapshot() -> CreatorPermissionSnapshot {
    CreatorPermissionSnapshot {
        tools_allowed: vec!["FileRead".to_string(), "SearchCode".to_string()],
        max_iterations: 30,
        max_tool_calls: 60,
        max_tokens: 8_192,
        delegation_depth: 3,
    }
}

fn create_params() -> AgentCreateParams {
    AgentCreateParams {
        name: "code-scout".to_string(),
        description: "Finds implementation and test evidence".to_string(),
        mode: AgentMode::Explore,
        capabilities: vec!["repository-search".to_string()],
        allowed_tools: vec!["FileRead".to_string(), "SearchCode".to_string()],
        system_prompt: "Inspect before reporting.".to_string(),
        context_sharing: ContextStrategy::Summary,
    }
}

#[tokio::test]
async fn lifecycle_changes_stay_in_sync_with_the_live_registry() {
    let dir = tempdir().unwrap();
    let journal = dir.path().join("dynamic-agents.jsonl");
    let registry = Arc::new(Mutex::new(AgentRegistry::new()));
    let service = DynamicAgentService::open(&journal, Arc::clone(&registry))
        .await
        .unwrap();

    let created = service
        .create(create_params(), "root-agent", &creator_snapshot())
        .await
        .unwrap();
    assert_eq!(created.effective_permissions.delegation_depth, 2);
    assert!(registry.lock().await.get(&created.id).is_some());

    let updated = service
        .update(AgentUpdateParams {
            id: created.id.clone(),
            description: Some("Finds architecture, implementation, and test evidence".to_string()),
            mode: None,
            allowed_tools: None,
            system_prompt: None,
        })
        .await
        .unwrap();
    assert_eq!(updated.version, 2);
    assert_eq!(
        service.execution_trace_metadata(&created.id)["dynamic_agent"]["version"],
        2
    );
    assert_eq!(
        registry.lock().await.get(&created.id).unwrap().description,
        updated.definition.description
    );

    service
        .deactivate(&AgentDeactivateParams {
            id: created.id.clone(),
            reason: "superseded".to_string(),
        })
        .await
        .unwrap();
    assert!(registry.lock().await.get(&created.id).is_none());
    assert_eq!(
        service.get(&created.id).unwrap().status,
        AgentStatus::Deactivated
    );
}

#[tokio::test]
async fn reopening_rehydrates_active_agents_only() {
    let dir = tempdir().unwrap();
    let journal = dir.path().join("dynamic-agents.jsonl");
    let first_registry = Arc::new(Mutex::new(AgentRegistry::new()));
    let first = DynamicAgentService::open(&journal, first_registry)
        .await
        .unwrap();
    let active = first
        .create(create_params(), "root-agent", &creator_snapshot())
        .await
        .unwrap();

    let mut inactive_params = create_params();
    inactive_params.name = "temporary-scout".to_string();
    let inactive = first
        .create(inactive_params, "root-agent", &creator_snapshot())
        .await
        .unwrap();
    first
        .deactivate(&AgentDeactivateParams {
            id: inactive.id.clone(),
            reason: "temporary task complete".to_string(),
        })
        .await
        .unwrap();
    drop(first);

    let reopened_registry = Arc::new(Mutex::new(AgentRegistry::new()));
    let reopened = DynamicAgentService::open(&journal, Arc::clone(&reopened_registry))
        .await
        .unwrap();

    assert_eq!(reopened.count(), 2);
    let registry = reopened_registry.lock().await;
    assert!(registry.get(&active.id).is_some());
    assert!(registry.get(&inactive.id).is_none());
}

#[tokio::test]
async fn rejects_noop_updates_and_blank_deactivation_reasons() {
    let dir = tempdir().unwrap();
    let journal = dir.path().join("dynamic-agents.jsonl");
    let registry = Arc::new(Mutex::new(AgentRegistry::new()));
    let service = DynamicAgentService::open(&journal, registry).await.unwrap();
    let created = service
        .create(create_params(), "root-agent", &creator_snapshot())
        .await
        .unwrap();

    let update = service
        .update(AgentUpdateParams {
            id: created.id.clone(),
            description: None,
            mode: None,
            allowed_tools: None,
            system_prompt: None,
        })
        .await;
    assert!(update.unwrap_err().to_string().contains("no changes"));

    let deactivate = service
        .deactivate(&AgentDeactivateParams {
            id: created.id,
            reason: "  ".to_string(),
        })
        .await;
    assert!(deactivate.unwrap_err().to_string().contains("reason"));
}

#[tokio::test]
async fn regression_proposal_approval_rolls_back_and_persists_decision() {
    let dir = tempdir().unwrap();
    let journal = dir.path().join("dynamic-agents.jsonl");
    let registry = Arc::new(Mutex::new(AgentRegistry::new()));
    let service = DynamicAgentService::open(&journal, Arc::clone(&registry))
        .await
        .unwrap();
    let created = service
        .create(create_params(), "root-agent", &creator_snapshot())
        .await
        .unwrap();
    let updated = service
        .update(AgentUpdateParams {
            id: created.id.clone(),
            description: Some("A regressed definition".to_string()),
            mode: None,
            allowed_tools: None,
            system_prompt: None,
        })
        .await
        .unwrap();

    let finding = DynamicAgentRegressionFinding {
        agent_id: created.id.clone(),
        baseline_version: 1,
        current_version: 2,
        baseline_samples: 5,
        current_samples: 5,
        baseline_success_rate: 1.0,
        current_success_rate: 0.2,
        success_rate_drop: 0.8,
        recommendation: "rollback".to_string(),
    };
    let first = service.propose_regressions(&[finding.clone()]).unwrap();
    let repeated = service.propose_regressions(&[finding]).unwrap();
    assert_eq!(first.len(), 1);
    assert_eq!(repeated[0].id, first[0].id);
    assert_eq!(service.list_proposals(None, None).len(), 1);

    let applied = service
        .decide_proposal(
            &first[0].id,
            DynamicAgentProposalDecision::Approve,
            Some("Repeated regression is sufficient evidence".to_string()),
        )
        .await
        .unwrap();
    assert_eq!(applied.status, DynamicAgentProposalStatus::Applied);
    assert_eq!(applied.applied_version, Some(3));
    let active = service.get(&created.id).unwrap();
    assert_eq!(active.version, 3);
    assert_eq!(
        active.definition.description,
        created.definition.description
    );
    assert_ne!(
        active.definition.description,
        updated.definition.description
    );
    assert_eq!(
        registry.lock().await.get(&created.id).unwrap().description,
        created.definition.description
    );
    drop(service);

    let reopened_registry = Arc::new(Mutex::new(AgentRegistry::new()));
    let reopened = DynamicAgentService::open(&journal, reopened_registry)
        .await
        .unwrap();
    assert_eq!(reopened.get(&created.id).unwrap().version, 3);
    assert_eq!(
        reopened.list_proposals(None, None)[0].status,
        DynamicAgentProposalStatus::Applied
    );
}

#[tokio::test]
async fn rejected_regression_proposal_does_not_mutate_the_agent() {
    let dir = tempdir().unwrap();
    let journal = dir.path().join("dynamic-agents.jsonl");
    let registry = Arc::new(Mutex::new(AgentRegistry::new()));
    let service = DynamicAgentService::open(&journal, registry).await.unwrap();
    let created = service
        .create(create_params(), "root-agent", &creator_snapshot())
        .await
        .unwrap();
    service
        .update(AgentUpdateParams {
            id: created.id.clone(),
            description: Some("Keep this version".to_string()),
            mode: None,
            allowed_tools: None,
            system_prompt: None,
        })
        .await
        .unwrap();
    let proposals = service
        .propose_regressions(&[DynamicAgentRegressionFinding {
            agent_id: created.id.clone(),
            baseline_version: 1,
            current_version: 2,
            baseline_samples: 5,
            current_samples: 5,
            baseline_success_rate: 1.0,
            current_success_rate: 0.4,
            success_rate_drop: 0.6,
            recommendation: "review".to_string(),
        }])
        .unwrap();

    let rejected = service
        .decide_proposal(
            &proposals[0].id,
            DynamicAgentProposalDecision::Reject,
            Some("The changed task distribution explains the result".to_string()),
        )
        .await
        .unwrap();

    assert_eq!(rejected.status, DynamicAgentProposalStatus::Rejected);
    assert_eq!(service.get(&created.id).unwrap().version, 2);
}

#[tokio::test]
async fn validated_candidate_stays_pending_until_approval_then_becomes_a_new_version() {
    let dir = tempdir().unwrap();
    let journal = dir.path().join("dynamic-agents.jsonl");
    let registry = Arc::new(Mutex::new(AgentRegistry::new()));
    let service = DynamicAgentService::open(&journal, Arc::clone(&registry))
        .await
        .unwrap();
    let created = service
        .create(create_params(), "root-agent", &creator_snapshot())
        .await
        .unwrap();
    let regressed = service
        .update(AgentUpdateParams {
            id: created.id.clone(),
            description: Some("A regressed definition".to_string()),
            mode: None,
            allowed_tools: None,
            system_prompt: None,
        })
        .await
        .unwrap();
    let proposal = service
        .propose_regressions(&[DynamicAgentRegressionFinding {
            agent_id: created.id.clone(),
            baseline_version: 1,
            current_version: 2,
            baseline_samples: 5,
            current_samples: 5,
            baseline_success_rate: 1.0,
            current_success_rate: 0.2,
            success_rate_drop: 0.8,
            recommendation: "refine".to_string(),
        }])
        .unwrap()
        .remove(0);

    let drafted = service
        .attach_candidate(
            &proposal.id,
            DynamicAgentCandidateDefinition {
                description: "Finds implementation evidence with explicit source references"
                    .to_string(),
                mode: AgentMode::Explore,
                allowed_tools: vec!["FileRead".to_string()],
                system_prompt: "Inspect evidence and cite source files before concluding."
                    .to_string(),
                rationale: "Narrow the tool surface and require grounded conclusions".to_string(),
            },
        )
        .unwrap();
    assert!(matches!(
        drafted.change,
        DynamicAgentProposalChange::CandidateUpdate { .. }
    ));
    assert_eq!(service.get(&created.id).unwrap().version, regressed.version);

    let applied = service
        .decide_proposal(
            &proposal.id,
            DynamicAgentProposalDecision::Approve,
            Some("Candidate is permission-safe and addresses the evidence".to_string()),
        )
        .await
        .unwrap();
    assert_eq!(applied.status, DynamicAgentProposalStatus::Applied);
    assert_eq!(applied.applied_version, Some(3));
    let active = service.get(&created.id).unwrap();
    assert_eq!(active.version, 3);
    assert_eq!(active.definition.allowed_tools, vec!["FileRead"]);
    assert!(active.definition.description.contains("source references"));
    assert_eq!(
        registry
            .lock()
            .await
            .get(&created.id)
            .unwrap()
            .allowed_tools,
        vec!["FileRead"]
    );
}

#[tokio::test]
async fn candidate_cannot_expand_the_agents_effective_permissions() {
    let dir = tempdir().unwrap();
    let journal = dir.path().join("dynamic-agents.jsonl");
    let registry = Arc::new(Mutex::new(AgentRegistry::new()));
    let service = DynamicAgentService::open(&journal, registry).await.unwrap();
    let created = service
        .create(create_params(), "root-agent", &creator_snapshot())
        .await
        .unwrap();
    let updated = service
        .update(AgentUpdateParams {
            id: created.id.clone(),
            description: Some("A regressed definition".to_string()),
            mode: None,
            allowed_tools: None,
            system_prompt: None,
        })
        .await
        .unwrap();
    let proposal = service
        .propose_regressions(&[DynamicAgentRegressionFinding {
            agent_id: created.id,
            baseline_version: 1,
            current_version: updated.version,
            baseline_samples: 5,
            current_samples: 5,
            baseline_success_rate: 1.0,
            current_success_rate: 0.2,
            success_rate_drop: 0.8,
            recommendation: "refine".to_string(),
        }])
        .unwrap()
        .remove(0);

    let result = service.attach_candidate(
        &proposal.id,
        DynamicAgentCandidateDefinition {
            description: "Attempts to add shell access".to_string(),
            mode: AgentMode::Build,
            allowed_tools: vec!["FileRead".to_string(), "ShellExec".to_string()],
            system_prompt: "Inspect and execute commands.".to_string(),
            rationale: "Shell access would be convenient".to_string(),
        },
    );

    assert!(result.unwrap_err().to_string().contains("ShellExec"));
    assert_eq!(service.get(&proposal.agent_id).unwrap().version, 2);
}
