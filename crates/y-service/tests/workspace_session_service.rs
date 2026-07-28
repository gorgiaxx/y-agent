use y_core::session::{CreateSessionOptions, SessionType};
use y_service::config_types::StorageConfig;
use y_service::{ServiceConfig, ServiceContainer, SessionService, WorkspaceService};

async fn container() -> ServiceContainer {
    let config = ServiceConfig {
        storage: StorageConfig::in_memory(),
        ..ServiceConfig::default()
    };
    ServiceContainer::from_config(&config).await.unwrap()
}

#[tokio::test]
async fn resumable_sessions_are_strictly_scoped_to_the_canonical_workspace() {
    let container = container().await;
    let workspace_a = tempfile::tempdir().unwrap();
    let workspace_b = tempfile::tempdir().unwrap();

    let session_a = SessionService::create_session(
        &container.session_manager,
        CreateSessionOptions {
            parent_id: None,
            session_type: SessionType::Main,
            agent_id: None,
            title: Some("a".into()),
        },
        workspace_a.path(),
    )
    .await
    .unwrap();
    SessionService::create_session(
        &container.session_manager,
        CreateSessionOptions {
            parent_id: None,
            session_type: SessionType::Main,
            agent_id: None,
            title: Some("b".into()),
        },
        workspace_b.path(),
    )
    .await
    .unwrap();
    container
        .session_manager
        .create_session(CreateSessionOptions {
            parent_id: None,
            session_type: SessionType::Main,
            agent_id: None,
            title: Some("legacy unassigned".into()),
        })
        .await
        .unwrap();

    let listed = SessionService::list_resumable_sessions(
        &container.session_manager,
        workspace_a.path(),
        None,
    )
    .await
    .unwrap();

    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, session_a.id);
}

#[tokio::test]
async fn resume_target_resolution_does_not_escape_the_current_workspace() {
    let container = container().await;
    let workspace_a = tempfile::tempdir().unwrap();
    let workspace_b = tempfile::tempdir().unwrap();
    let session_b = SessionService::create_session(
        &container.session_manager,
        CreateSessionOptions {
            parent_id: None,
            session_type: SessionType::Main,
            agent_id: None,
            title: Some("workspace-b".into()),
        },
        workspace_b.path(),
    )
    .await
    .unwrap();

    let resolved = SessionService::resolve_resume_target(
        &container.session_manager,
        workspace_a.path(),
        None,
        &session_b.id.to_string()[..8],
    )
    .await
    .unwrap();

    assert!(resolved.is_none());
}

#[cfg(unix)]
#[tokio::test]
async fn canonical_workspace_aliases_share_resume_history() {
    let container = container().await;
    let temp = tempfile::tempdir().unwrap();
    let real = temp.path().join("real");
    std::fs::create_dir(&real).unwrap();
    let alias = temp.path().join("alias");
    std::os::unix::fs::symlink(&real, &alias).unwrap();

    let created = SessionService::create_session(
        &container.session_manager,
        CreateSessionOptions {
            parent_id: None,
            session_type: SessionType::Main,
            agent_id: None,
            title: None,
        },
        &alias,
    )
    .await
    .unwrap();
    let listed = SessionService::list_resumable_sessions(&container.session_manager, &real, None)
        .await
        .unwrap();

    assert_eq!(listed.first().map(|session| &session.id), Some(&created.id));
}

#[tokio::test]
async fn legacy_workspace_assignments_backfill_only_missing_sqlite_identity() {
    let container = container().await;
    let config_dir = tempfile::tempdir().unwrap();
    let workspace_dir = tempfile::tempdir().unwrap();
    let workspaces = WorkspaceService::new(config_dir.path());
    let workspace = workspaces
        .create(
            "legacy".into(),
            workspace_dir.path().to_string_lossy().into_owned(),
        )
        .unwrap();
    let session = container
        .session_manager
        .create_session(CreateSessionOptions {
            parent_id: None,
            session_type: SessionType::Main,
            agent_id: None,
            title: None,
        })
        .await
        .unwrap();
    workspaces
        .assign_session(workspace.id, session.id.to_string())
        .unwrap();

    let updated =
        SessionService::backfill_legacy_assignments(&container.session_manager, &workspaces)
            .await
            .unwrap();
    let persisted = container
        .session_manager
        .get_session(&session.id)
        .await
        .unwrap();

    assert_eq!(updated, 1);
    assert_eq!(
        persisted.workspace_path,
        Some(
            std::fs::canonicalize(workspace_dir.path())
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        )
    );
}
