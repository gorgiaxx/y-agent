use std::sync::Arc;

use async_trait::async_trait;
use tempfile::tempdir;
use y_core::agent::{AgentDelegator, ContextStrategyHint, DelegationError, DelegationOutput};
use y_core::skill::{SkillManifest, SkillVersion};
use y_core::types::{now, SkillId};
use y_service::skill_evolution_refinement::SkillEvolutionRefinementService;
use y_service::skill_evolution_service::{
    PromotionResources, SkillEvolutionService, SkillProposalDecision, TurnExperienceInput,
};
use y_skills::evolution::EvolutionProposal;
use y_skills::evolution::ProposalStatus;
use y_skills::experience::{
    EvidenceEntry, EvidenceProvenance, ExperienceOutcome, TokenUsage, ToolCallRecord,
};
use y_skills::FilesystemSkillStore;

#[derive(Debug)]
struct MockSkillRefiner;

#[async_trait]
impl AgentDelegator for MockSkillRefiner {
    async fn delegate(
        &self,
        agent_name: &str,
        input: serde_json::Value,
        context_strategy: ContextStrategyHint,
        _session_id: Option<uuid::Uuid>,
    ) -> Result<DelegationOutput, DelegationError> {
        assert_eq!(agent_name, "skill-refiner");
        assert_eq!(context_strategy, ContextStrategyHint::None);
        assert_eq!(input["constraints"]["active_mutation_allowed"], false);
        assert_eq!(input["current_skill"]["version"], "v1");
        assert!(input["evidence"]
            .as_array()
            .is_some_and(|items| !items.is_empty()));
        Ok(DelegationOutput {
            text: serde_json::json!({
                "root_content": "Review ownership, temporary lifetimes, and borrow extension before proposing edits.",
                "rationale": "The candidate directly addresses the repeated lifetime failure evidence."
            })
            .to_string(),
            tokens_used: 120,
            input_tokens: 90,
            output_tokens: 30,
            model_used: "test-model".to_string(),
            duration_ms: 12,
            workspace_isolation: None,
        })
    }
}

#[tokio::test]
async fn refinement_persists_candidate_without_mutation_then_approval_promotes_it() {
    let dir = tempdir().unwrap();
    let skills_dir = dir.path().join("skills");
    save_skill(&skills_dir);
    let evolution_dir = dir.path().join("evolution");
    let service = Arc::new(
        SkillEvolutionService::open(&evolution_dir, Some(skills_dir.clone()))
            .await
            .unwrap(),
    );
    let proposal = create_proposal(&service).await;
    let refinement =
        SkillEvolutionRefinementService::new(Arc::clone(&service), Arc::new(MockSkillRefiner));

    let drafted = refinement
        .generate_candidate(
            &proposal.id,
            Some("Keep the skill atomic"),
            PromotionResources::default(),
            None,
        )
        .await
        .unwrap();

    assert_eq!(drafted.status, ProposalStatus::PendingApproval);
    assert!(drafted
        .candidate_root_content
        .as_deref()
        .is_some_and(|content| content.contains("temporary lifetimes")));
    assert!(drafted.diff_preview.contains("temporary lifetimes"));
    assert_eq!(active_content(&skills_dir), "Review ownership carefully.");

    drop(refinement);
    drop(service);
    let reopened = SkillEvolutionService::open(&evolution_dir, Some(skills_dir.clone()))
        .await
        .unwrap();
    let persisted = reopened.load_proposals().await.unwrap().remove(0);
    assert_eq!(
        persisted.candidate_root_content,
        drafted.candidate_root_content
    );

    let promoted = reopened
        .decide_proposal(
            &proposal.id,
            SkillProposalDecision::Approve,
            Some("Validated against repeated user corrections".to_string()),
            PromotionResources::default(),
        )
        .await
        .unwrap();

    assert_eq!(promoted.status, ProposalStatus::Promoted);
    assert!(active_content(&skills_dir).contains("temporary lifetimes"));
    assert_ne!(promoted.proposed_version.as_deref(), Some("v1"));
}

#[tokio::test]
async fn reject_and_defer_decisions_never_mutate_the_active_skill() {
    for decision in [SkillProposalDecision::Reject, SkillProposalDecision::Defer] {
        let dir = tempdir().unwrap();
        let skills_dir = dir.path().join("skills");
        save_skill(&skills_dir);
        let service = Arc::new(
            SkillEvolutionService::open(dir.path().join("evolution"), Some(skills_dir.clone()))
                .await
                .unwrap(),
        );
        let proposal = create_proposal(&service).await;

        let decided = service
            .decide_proposal(
                &proposal.id,
                decision,
                Some("Reviewer requested no activation".to_string()),
                PromotionResources::default(),
            )
            .await
            .unwrap();

        let expected = match decision {
            SkillProposalDecision::Reject => ProposalStatus::Rejected,
            SkillProposalDecision::Defer => ProposalStatus::Deferred,
            SkillProposalDecision::Approve => unreachable!(),
        };
        assert_eq!(decided.status, expected);
        assert_eq!(active_content(&skills_dir), "Review ownership carefully.");
    }
}

async fn create_proposal(service: &SkillEvolutionService) -> EvolutionProposal {
    for _ in 0..3 {
        service.record_turn(failed_turn()).await.unwrap();
    }
    service.load_proposals().await.unwrap().remove(0)
}

fn failed_turn() -> TurnExperienceInput {
    TurnExperienceInput {
        skills: vec!["review-rust".to_string()],
        task_description: "Review the ownership module".to_string(),
        outcome: ExperienceOutcome::Failure,
        trajectory_summary: "Compilation failed after the review edit".to_string(),
        key_decisions: vec!["Changed the borrow strategy".to_string()],
        evidence: vec![
            EvidenceEntry {
                content: "The task failed its compilation check".to_string(),
                provenance: EvidenceProvenance::TaskOutcome,
            },
            EvidenceEntry {
                content: "Do not extend the temporary borrow".to_string(),
                provenance: EvidenceProvenance::UserCorrection,
            },
        ],
        tool_calls: vec![ToolCallRecord {
            name: "ShellExec".to_string(),
            success: false,
            duration_ms: 25,
        }],
        error_messages: vec!["borrowed value does not live long enough".to_string()],
        duration_ms: 100,
        token_usage: TokenUsage::new(100, 50),
    }
}

fn save_skill(path: &std::path::Path) {
    let timestamp = now();
    FilesystemSkillStore::new(path)
        .unwrap()
        .save_skill(&SkillManifest {
            id: SkillId::from_string("skill-review-rust"),
            name: "review-rust".to_string(),
            description: "Reviews Rust ownership".to_string(),
            version: SkillVersion("v1".to_string()),
            tags: vec!["rust".to_string()],
            trigger_patterns: vec![],
            knowledge_bases: vec![],
            root_content: "Review ownership carefully.".to_string(),
            sub_documents: vec![],
            token_estimate: 10,
            created_at: timestamp,
            updated_at: timestamp,
            classification: None,
            constraints: None,
            security: None,
            references: None,
            author: None,
            source_format: None,
            source_hash: None,
            state: None,
            root_path: None,
        })
        .unwrap();
}

fn active_content(skills_dir: &std::path::Path) -> String {
    FilesystemSkillStore::new(skills_dir)
        .unwrap()
        .load_skill("review-rust")
        .unwrap()
        .root_content
}
