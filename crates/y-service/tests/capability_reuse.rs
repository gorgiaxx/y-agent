use tempfile::tempdir;
use y_core::skill::{SkillManifest, SkillVersion};
use y_core::types::{now, SkillId};
use y_service::capability_reuse::{CapabilityAssetType, CapabilityReusePlanner};
use y_service::workflow_service::{CreateWorkflowRequest, WorkflowService};
use y_service::{ServiceConfig, ServiceContainer};

#[tokio::test]
async fn planner_bounds_cross_asset_recommendations_and_sets_creation_guard() {
    let dir = tempdir().unwrap();
    let mut config = ServiceConfig::default();
    config.storage = y_storage::StorageConfig {
        db_path: ":memory:".to_string(),
        pool_size: 1,
        wal_enabled: false,
        transcript_dir: dir.path().join("transcripts"),
        ..y_storage::StorageConfig::default()
    };
    let container = ServiceContainer::from_config(&config).await.unwrap();
    let timestamp = now();
    container.skill_search.write().await.index(SkillManifest {
        id: SkillId::from_string("skill-rust-review"),
        name: "rust-review".to_string(),
        description: "Review Rust files for correctness".to_string(),
        version: SkillVersion("v1".to_string()),
        tags: vec!["rust".to_string(), "review".to_string()],
        trigger_patterns: vec!["review rust files".to_string()],
        knowledge_bases: vec![],
        root_content: "Review Rust files.".to_string(),
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
    });
    container
        .agent_registry
        .lock()
        .await
        .register_agent_from_toml(
            r#"
id = "rust-reviewer"
name = "rust-reviewer"
description = "Reviews Rust files for correctness"
mode = "plan"
trust_tier = "user_defined"
capabilities = ["rust", "review"]
allowed_tools = ["FileRead"]
system_prompt = "Review Rust files."
user_callable = true
"#,
        )
        .unwrap();
    WorkflowService::create(
        &container.workflow_store,
        &CreateWorkflowRequest {
            name: "rust-review-pipeline".to_string(),
            definition: "inspect >> review >> summarize".to_string(),
            format: "expression_dsl".to_string(),
            description: Some("Review Rust files and summarize findings".to_string()),
            tags: Some("rust,review".to_string()),
        },
    )
    .await
    .unwrap();

    let decision =
        CapabilityReusePlanner::recommend(&container, "Review these Rust files", &[]).await;

    assert!(decision.reuse_before_create);
    assert!(decision.recommendations.len() <= 4);
    for asset_type in [
        CapabilityAssetType::Skill,
        CapabilityAssetType::Agent,
        CapabilityAssetType::Workflow,
    ] {
        assert_eq!(
            decision
                .recommendations
                .iter()
                .filter(|recommendation| recommendation.asset_type == asset_type)
                .count(),
            1
        );
    }
}
