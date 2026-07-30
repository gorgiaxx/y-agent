use y_core::provider::{ThinkingConfig, ThinkingEffort};
use y_service::chat_types::OperationMode;
use y_service::{
    AutomationRunRequest, AutomationRunService, ServiceConfig, ServiceContainer, SessionService,
};

async fn container(temp: &tempfile::TempDir) -> ServiceContainer {
    let mut storage = y_storage::StorageConfig::in_memory();
    storage.transcript_dir = temp.path().join("transcripts");
    ServiceContainer::from_config(&ServiceConfig {
        storage,
        ..ServiceConfig::default()
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn new_agent_run_is_bound_and_exposes_public_session_reference() {
    let temp = tempfile::tempdir().unwrap();
    let container = container(&temp).await;

    let prepared = AutomationRunService::prepare(
        &container,
        AutomationRunRequest {
            agent_id: Some("general-purpose".into()),
            session_name: Some("CVE-2026 nightly analysis".into()),
            user_input: "analyze the CVE".into(),
            workspace: temp.path().to_path_buf(),
            ..AutomationRunRequest::default()
        },
    )
    .await
    .unwrap();

    assert!(!prepared.resumed);
    assert_eq!(
        prepared.session_reference,
        SessionService::public_session_reference(&prepared.turn.session_id)
    );
    assert_eq!(
        prepared
            .turn
            .agent_config
            .as_ref()
            .map(|config| config.agent_id.as_str()),
        Some("general-purpose")
    );
    let stored = container
        .session_manager
        .get_session(&prepared.turn.session_id)
        .await
        .unwrap();
    assert_eq!(stored.title.as_deref(), Some("CVE-2026 nightly analysis"));
}

#[tokio::test]
async fn unnamed_run_derives_a_searchable_title_from_the_prompt() {
    let temp = tempfile::tempdir().unwrap();
    let container = container(&temp).await;

    let prepared = AutomationRunService::prepare(
        &container,
        AutomationRunRequest {
            user_input: "Analyze CVE-2026-0001 and produce remediation guidance".into(),
            workspace: temp.path().to_path_buf(),
            ..AutomationRunRequest::default()
        },
    )
    .await
    .unwrap();
    let stored = container
        .session_manager
        .get_session(&prepared.turn.session_id)
        .await
        .unwrap();

    assert_eq!(
        stored.title.as_deref(),
        Some("Analyze CVE-2026-0001 and produce remediation guidance")
    );
}

#[tokio::test]
async fn resumed_run_accepts_public_reference_and_restores_agent_binding() {
    let temp = tempfile::tempdir().unwrap();
    let container = container(&temp).await;
    let first = AutomationRunService::prepare(
        &container,
        AutomationRunRequest {
            agent_id: Some("general-purpose".into()),
            user_input: "first turn".into(),
            workspace: temp.path().to_path_buf(),
            ..AutomationRunRequest::default()
        },
    )
    .await
    .unwrap();

    let resumed = AutomationRunService::prepare(
        &container,
        AutomationRunRequest {
            session_target: Some(first.session_reference.clone()),
            user_input: "continue".into(),
            workspace: temp.path().to_path_buf(),
            ..AutomationRunRequest::default()
        },
    )
    .await
    .unwrap();

    assert!(resumed.resumed);
    assert_eq!(resumed.turn.session_id, first.turn.session_id);
    assert_eq!(
        resumed
            .turn
            .agent_config
            .as_ref()
            .map(|config| config.agent_id.as_str()),
        Some("general-purpose")
    );
}

#[tokio::test]
async fn run_rejects_internal_agent() {
    let temp = tempfile::tempdir().unwrap();
    let container = container(&temp).await;

    let error = AutomationRunService::prepare(
        &container,
        AutomationRunRequest {
            agent_id: Some("title-generator".into()),
            user_input: "not allowed".into(),
            workspace: temp.path().to_path_buf(),
            ..AutomationRunRequest::default()
        },
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("not user-callable"));
}

#[tokio::test]
async fn request_overrides_are_forwarded_to_the_turn_contract() {
    let temp = tempfile::tempdir().unwrap();
    let container = container(&temp).await;

    let prepared = AutomationRunService::prepare(
        &container,
        AutomationRunRequest {
            user_input: "analyze".into(),
            workspace: temp.path().to_path_buf(),
            provider_id: Some("openai".into()),
            model: Some("gpt-5".into()),
            skills: Some(vec!["cve-triage".into()]),
            knowledge_collections: Some(vec!["vulnerabilities".into()]),
            thinking: Some(ThinkingConfig {
                effort: ThinkingEffort::High,
            }),
            plan_mode: Some("loop".into()),
            operation_mode: Some(OperationMode::FullAccess),
            ..AutomationRunRequest::default()
        },
    )
    .await
    .unwrap();
    let input = prepared.turn.as_turn_input();

    assert_eq!(input.provider_id.as_deref(), Some("openai"));
    assert_eq!(input.preferred_models, ["gpt-5"]);
    assert_eq!(input.skills, ["cve-triage"]);
    assert_eq!(input.knowledge_collections, ["vulnerabilities"]);
    assert_eq!(
        input.thinking.as_ref().map(|config| config.effort),
        Some(ThinkingEffort::High)
    );
    assert_eq!(input.plan_mode.as_deref(), Some("loop"));
    assert_eq!(input.operation_mode, OperationMode::FullAccess);
    let canonical_workspace = std::fs::canonicalize(temp.path()).unwrap();
    assert_eq!(
        input.working_directory.as_deref(),
        canonical_workspace.to_str()
    );
}
