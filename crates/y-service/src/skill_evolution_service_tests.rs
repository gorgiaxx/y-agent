use super::*;
use y_core::skill::{SkillManifest, SkillVersion};
use y_core::types::{now, SkillId};
use y_skills::evolution::ProposalStatus;
use y_skills::experience::{
    EvidenceEntry, EvidenceProvenance, ExperienceOutcome, TokenUsage, ToolCallRecord,
};
use y_skills::FilesystemSkillStore;

#[tokio::test]
async fn test_record_turn_persists_complete_versioned_experience() {
    let temp = tempfile::TempDir::new().unwrap();
    let skills_dir = temp.path().join("skills");
    save_skill(&skills_dir, "review-rust", "v1");
    let service = SkillEvolutionService::open(temp.path().join("evolution"), Some(skills_dir))
        .await
        .unwrap();

    service.record_turn(failed_turn()).await.unwrap();
    let records = service.load_experiences().await.unwrap();

    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert_eq!(record.skill_id.as_deref(), Some("review-rust"));
    assert_eq!(record.skill_version.as_deref(), Some("v1"));
    assert_eq!(record.task_description, "Review the ownership module");
    assert_eq!(
        record.trajectory_summary,
        "Compilation failed after the review edit"
    );
    assert_eq!(record.key_decisions, vec!["Changed the borrow strategy"]);
    assert_eq!(record.evidence.len(), 2);
    assert_eq!(record.tool_calls.len(), 1);
    assert_eq!(
        record.error_messages,
        vec!["borrowed value does not live long enough"]
    );
    assert_eq!(record.token_usage.total, 150);
}

#[tokio::test]
async fn test_repeated_failures_create_one_pending_evidence_backed_proposal() {
    let temp = tempfile::TempDir::new().unwrap();
    let skills_dir = temp.path().join("skills");
    save_skill(&skills_dir, "review-rust", "v1");
    let service = SkillEvolutionService::open(temp.path().join("evolution"), Some(skills_dir))
        .await
        .unwrap();

    assert!(service.record_turn(failed_turn()).await.unwrap().is_empty());
    assert!(service.record_turn(failed_turn()).await.unwrap().is_empty());
    let created = service.record_turn(failed_turn()).await.unwrap();
    let duplicate = service.record_turn(failed_turn()).await.unwrap();

    assert_eq!(created.len(), 1);
    assert_eq!(created[0].status, ProposalStatus::PendingApproval);
    assert_eq!(created[0].skill_name, "review-rust");
    assert_eq!(created[0].current_version, "v1");
    assert!(!created[0].patterns_referenced.is_empty());
    assert!(duplicate.is_empty());
    assert_eq!(service.load_proposals().await.unwrap().len(), 1);
}

#[tokio::test]
async fn test_update_proposal_status_persists_supervised_decision() {
    let temp = tempfile::TempDir::new().unwrap();
    let skills_dir = temp.path().join("skills");
    save_skill(&skills_dir, "review-rust", "v1");
    let service = SkillEvolutionService::open(temp.path().join("evolution"), Some(skills_dir))
        .await
        .unwrap();
    for _ in 0..3 {
        service.record_turn(failed_turn()).await.unwrap();
    }
    let proposal = service.load_proposals().await.unwrap().remove(0);

    let approved = service
        .update_proposal_status(&proposal.id, ProposalStatus::Approved)
        .await
        .unwrap();
    let latest = service.load_proposals().await.unwrap();

    assert_eq!(approved.status, ProposalStatus::Approved);
    assert_eq!(latest.len(), 1);
    assert_eq!(latest[0].status, ProposalStatus::Approved);
}

#[tokio::test]
async fn test_approved_proposal_promotes_validated_version_and_rolls_back() {
    let temp = tempfile::TempDir::new().unwrap();
    let skills_dir = temp.path().join("skills");
    save_skill(&skills_dir, "review-rust", "v1");
    let service =
        SkillEvolutionService::open(temp.path().join("evolution"), Some(skills_dir.clone()))
            .await
            .unwrap();
    for _ in 0..3 {
        service.record_turn(failed_turn()).await.unwrap();
    }
    let proposal = service.load_proposals().await.unwrap().remove(0);
    service
        .update_proposal_status(&proposal.id, ProposalStatus::Approved)
        .await
        .unwrap();

    let promoted = service
        .promote_approved_proposal(
            &proposal.id,
            "Review ownership, temporary lifetimes, and borrow extension before proposing edits.",
            PromotionResources::default(),
        )
        .await
        .unwrap();
    let promoted_manifest = FilesystemSkillStore::new(&skills_dir)
        .unwrap()
        .load_skill("review-rust")
        .unwrap();

    assert_eq!(promoted.status, ProposalStatus::Promoted);
    assert_eq!(promoted_manifest.version.0.len(), 64);
    assert!(promoted_manifest
        .root_content
        .contains("temporary lifetimes"));

    let rolled_back = service
        .rollback_promoted_proposal(&proposal.id)
        .await
        .unwrap();
    let restored = FilesystemSkillStore::new(&skills_dir)
        .unwrap()
        .load_skill("review-rust")
        .unwrap();

    assert_eq!(rolled_back.status, ProposalStatus::RolledBack);
    assert_eq!(
        restored.root_content,
        "Review ownership and borrowing carefully."
    );
}

#[tokio::test]
async fn test_promotion_rejects_invalid_candidate_without_mutating_active_skill() {
    let temp = tempfile::TempDir::new().unwrap();
    let skills_dir = temp.path().join("skills");
    save_skill(&skills_dir, "review-rust", "v1");
    let service =
        SkillEvolutionService::open(temp.path().join("evolution"), Some(skills_dir.clone()))
            .await
            .unwrap();
    for _ in 0..3 {
        service.record_turn(failed_turn()).await.unwrap();
    }
    let proposal = service.load_proposals().await.unwrap().remove(0);
    service
        .update_proposal_status(&proposal.id, ProposalStatus::Approved)
        .await
        .unwrap();

    let error = service
        .promote_approved_proposal(
            &proposal.id,
            &"oversized instruction ".repeat(10_000),
            PromotionResources::default(),
        )
        .await
        .unwrap_err();
    let active = FilesystemSkillStore::new(&skills_dir)
        .unwrap()
        .load_skill("review-rust")
        .unwrap();

    assert!(error.to_string().contains("root_token_limit"));
    assert_eq!(active.version.0, "v1");
    assert_eq!(
        active.root_content,
        "Review ownership and borrowing carefully."
    );
}

#[tokio::test]
async fn test_promoted_version_regression_triggers_automatic_rollback() {
    let temp = tempfile::TempDir::new().unwrap();
    let skills_dir = temp.path().join("skills");
    save_skill(&skills_dir, "review-rust", "v1");
    let service =
        SkillEvolutionService::open(temp.path().join("evolution"), Some(skills_dir.clone()))
            .await
            .unwrap();
    for _ in 0..5 {
        service
            .record_turn(successful_corrected_turn())
            .await
            .unwrap();
    }
    let proposal = service.load_proposals().await.unwrap().remove(0);
    service
        .update_proposal_status(&proposal.id, ProposalStatus::Approved)
        .await
        .unwrap();
    service
        .promote_approved_proposal(
            &proposal.id,
            "Review ownership with the new candidate instructions.",
            PromotionResources::default(),
        )
        .await
        .unwrap();
    for _ in 0..5 {
        service.record_turn(failed_turn()).await.unwrap();
    }

    let active = FilesystemSkillStore::new(&skills_dir)
        .unwrap()
        .load_skill("review-rust")
        .unwrap();
    let latest = service.load_proposals().await.unwrap();

    assert_eq!(
        active.root_content,
        "Review ownership and borrowing carefully."
    );
    assert_eq!(
        latest
            .iter()
            .find(|candidate| candidate.id == proposal.id)
            .unwrap()
            .status,
        ProposalStatus::RolledBack
    );
}

#[tokio::test]
async fn test_infrastructure_failures_do_not_create_skill_proposals() {
    let temp = tempfile::TempDir::new().unwrap();
    let service = SkillEvolutionService::open(temp.path().join("evolution"), None)
        .await
        .unwrap();
    let failure = TurnExperienceInput {
        skills: vec!["review-rust".to_string()],
        task_description: "Review a Rust module".to_string(),
        outcome: ExperienceOutcome::Failure,
        trajectory_summary: "Provider request timed out".to_string(),
        key_decisions: vec![],
        evidence: vec![EvidenceEntry {
            content: "Provider request timed out".to_string(),
            provenance: EvidenceProvenance::TaskOutcome,
        }],
        tool_calls: vec![],
        error_messages: vec!["provider timeout".to_string()],
        duration_ms: 100,
        token_usage: TokenUsage::default(),
    };

    for _ in 0..4 {
        assert!(service
            .record_turn(failure.clone())
            .await
            .unwrap()
            .is_empty());
    }

    assert!(service.load_proposals().await.unwrap().is_empty());
    assert_eq!(service.load_experiences().await.unwrap().len(), 4);
}

#[test]
fn test_agent_observation_requires_user_corroboration() {
    let evidence = SkillEvolutionService::sanitize_evidence(vec![
        EvidenceEntry {
            content: "uncorroborated guess".to_string(),
            provenance: EvidenceProvenance::AgentObservation,
        },
        EvidenceEntry {
            content: "shared correction".to_string(),
            provenance: EvidenceProvenance::AgentObservation,
        },
        EvidenceEntry {
            content: "shared correction".to_string(),
            provenance: EvidenceProvenance::UserCorrection,
        },
    ]);

    assert_eq!(evidence.len(), 2);
    assert!(evidence
        .iter()
        .all(|entry| entry.content == "shared correction"));
}

#[tokio::test]
async fn test_user_feedback_is_idempotent_and_preserves_correction_provenance() {
    let temp = tempfile::TempDir::new().unwrap();
    let service = SkillEvolutionService::open(temp.path().join("evolution"), None)
        .await
        .unwrap();
    let feedback_id = uuid::Uuid::new_v4();
    let trace_id = uuid::Uuid::new_v4();

    let first = service
        .record_user_feedback(
            feedback_id,
            trace_id,
            &["review-rust".to_string()],
            "Review the ownership module",
            0.0,
            "The review missed the unsafe aliasing path",
        )
        .await
        .unwrap();
    let repeated = service
        .record_user_feedback(
            feedback_id,
            trace_id,
            &["review-rust".to_string()],
            "Review the ownership module",
            0.0,
            "The review missed the unsafe aliasing path",
        )
        .await
        .unwrap();

    assert_eq!(first, 1);
    assert_eq!(repeated, 0);
    let records = service.load_experiences().await.unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].outcome, ExperienceOutcome::Failure);
    assert!(records[0].evidence.iter().any(|entry| {
        entry.provenance == EvidenceProvenance::UserCorrection
            && entry.content.contains("unsafe aliasing")
    }));
    assert!(records[0]
        .key_decisions
        .iter()
        .any(|entry| entry.contains(&trace_id.to_string())));
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

fn successful_corrected_turn() -> TurnExperienceInput {
    TurnExperienceInput {
        skills: vec!["review-rust".to_string()],
        task_description: "Review the ownership module".to_string(),
        outcome: ExperienceOutcome::Success,
        trajectory_summary: "Review completed and tests passed".to_string(),
        key_decisions: vec![],
        evidence: vec![EvidenceEntry {
            content: "Clarify temporary lifetime checks".to_string(),
            provenance: EvidenceProvenance::UserCorrection,
        }],
        tool_calls: vec![],
        error_messages: vec![],
        duration_ms: 80,
        token_usage: TokenUsage::new(80, 20),
    }
}

fn save_skill(path: &std::path::Path, name: &str, version: &str) {
    let timestamp = now();
    let store = FilesystemSkillStore::new(path).unwrap();
    store
        .save_skill(&SkillManifest {
            id: SkillId::from_string(format!("skill-{name}")),
            name: name.to_string(),
            description: "Reviews Rust ownership".to_string(),
            version: SkillVersion(version.to_string()),
            tags: vec!["rust".to_string()],
            trigger_patterns: vec![],
            knowledge_bases: vec![],
            root_content: "Review ownership and borrowing carefully.".to_string(),
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
