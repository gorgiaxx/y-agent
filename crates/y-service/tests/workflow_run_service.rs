use tempfile::tempdir;
use y_service::workflow_run_service::WorkflowRunService;
use y_service::workflow_service::{CreateWorkflowRequest, WorkflowService};
use y_service::{ServiceConfig, ServiceContainer};

#[tokio::test]
async fn runs_an_existing_workflow_by_name_with_parameters() {
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
    let workflow = WorkflowService::create(
        &container.workflow_store,
        &CreateWorkflowRequest {
            name: "research-pipeline".to_string(),
            definition: "search >> summarize".to_string(),
            format: "expression_dsl".to_string(),
            description: Some("Research and summarize a topic".to_string()),
            tags: Some("research,summary".to_string()),
        },
    )
    .await
    .unwrap();

    let execution = WorkflowRunService::run(
        &container,
        "research-pipeline",
        serde_json::json!({"topic": "hybrid retrieval"}),
    )
    .await
    .unwrap();

    assert_eq!(execution.request_summary["workflow_id"], workflow.id);
    assert_eq!(
        execution.request_summary["parameters"]["topic"],
        "hybrid retrieval"
    );
}
