use std::sync::Arc;

use async_trait::async_trait;
use tempfile::tempdir;
use tokio::sync::Mutex;
use y_agent::agent::dynamic_agent::CreatorPermissionSnapshot;
use y_agent::agent::dynamic_agent_proposal::DynamicAgentProposalChange;
use y_agent::agent::meta_tools::{AgentCreateParams, AgentUpdateParams};
use y_agent::{AgentMode, AgentRegistry, ContextStrategy};
use y_core::agent::{AgentDelegator, ContextStrategyHint, DelegationError, DelegationOutput};
use y_service::dynamic_agent_refinement::DynamicAgentRefinementService;
use y_service::dynamic_agent_service::DynamicAgentService;
use y_service::DynamicAgentRegressionFinding;

#[derive(Debug)]
struct MockRefiner;

#[async_trait]
impl AgentDelegator for MockRefiner {
    async fn delegate(
        &self,
        agent_name: &str,
        input: serde_json::Value,
        context_strategy: ContextStrategyHint,
        _session_id: Option<uuid::Uuid>,
    ) -> Result<DelegationOutput, DelegationError> {
        assert_eq!(agent_name, "agent-refiner");
        assert_eq!(context_strategy, ContextStrategyHint::None);
        assert_eq!(input["constraints"]["active_mutation_allowed"], false);
        assert_eq!(input["current_definition"]["version"], 2);
        assert_eq!(input["baseline_definition"]["version"], 1);
        Ok(DelegationOutput {
            text: serde_json::json!({
                "description": "Finds implementation evidence with explicit source references",
                "mode": "explore",
                "allowed_tools": ["FileRead"],
                "system_prompt": "Inspect evidence and cite source files before concluding.",
                "rationale": "Reduce unsupported conclusions and narrow the tool surface"
            })
            .to_string(),
            tokens_used: 120,
            input_tokens: 90,
            output_tokens: 30,
            model_used: "test-model".to_string(),
            duration_ms: 12,
        })
    }
}

fn creator_snapshot() -> CreatorPermissionSnapshot {
    CreatorPermissionSnapshot {
        tools_allowed: vec!["FileRead".to_string(), "SearchCode".to_string()],
        max_iterations: 30,
        max_tool_calls: 60,
        max_tokens: 8_192,
        delegation_depth: 3,
    }
}

#[tokio::test]
async fn delegated_refinement_replaces_rollback_with_a_validated_pending_candidate() {
    let dir = tempdir().unwrap();
    let registry = Arc::new(Mutex::new(AgentRegistry::new()));
    let dynamic_agents = Arc::new(
        DynamicAgentService::open(dir.path().join("dynamic-agents.jsonl"), registry)
            .await
            .unwrap(),
    );
    let created = dynamic_agents
        .create(
            AgentCreateParams {
                name: "code-scout".to_string(),
                description: "Finds implementation evidence".to_string(),
                mode: AgentMode::Explore,
                capabilities: vec!["repository-search".to_string()],
                allowed_tools: vec!["FileRead".to_string(), "SearchCode".to_string()],
                system_prompt: "Inspect before reporting.".to_string(),
                context_sharing: ContextStrategy::Summary,
            },
            "root-agent",
            &creator_snapshot(),
        )
        .await
        .unwrap();
    dynamic_agents
        .update(AgentUpdateParams {
            id: created.id.clone(),
            description: Some("A regressed definition".to_string()),
            mode: None,
            allowed_tools: None,
            system_prompt: None,
        })
        .await
        .unwrap();
    let proposal = dynamic_agents
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
    let refinement =
        DynamicAgentRefinementService::new(Arc::clone(&dynamic_agents), Arc::new(MockRefiner));

    let drafted = refinement
        .generate_candidate(&proposal.id, Some("Prefer fewer tools"), None)
        .await
        .unwrap();

    assert!(matches!(
        drafted.change,
        DynamicAgentProposalChange::CandidateUpdate { .. }
    ));
    assert_eq!(dynamic_agents.get(&created.id).unwrap().version, 2);
}
