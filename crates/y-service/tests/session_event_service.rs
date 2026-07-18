use y_core::session_event::{SessionEventKind, SessionEventRetention};
use y_core::types::SessionId;
use y_service::chat_types::{PendingInteraction, PendingPermission, PendingPlanReview, TurnEvent};
use y_service::{ServiceConfig, ServiceContainer};

async fn setup() -> ServiceContainer {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.keep();
    let mut config = ServiceConfig::default();
    config.storage = y_storage::StorageConfig {
        db_path: root.join("state.db").display().to_string(),
        pool_size: 1,
        wal_enabled: true,
        transcript_dir: root.join("transcripts"),
        ..y_storage::StorageConfig::default()
    };
    ServiceContainer::from_config(&config).await.unwrap()
}

#[tokio::test]
async fn durable_turn_events_persist_before_delivery_and_ephemeral_deltas_do_not() {
    let container = setup().await;
    let session_id = SessionId("session-1".into());
    let durable = container
        .session_event_service
        .publish_turn_event(
            &session_id,
            "run-1",
            &TurnEvent::ToolStart {
                name: "FileRead".into(),
                input_preview: "{}".into(),
                agent_name: "root".into(),
            },
            None,
        )
        .await
        .unwrap();
    let ephemeral = container
        .session_event_service
        .publish_turn_event(
            &session_id,
            "run-1",
            &TurnEvent::StreamDelta {
                content: "hello".into(),
                agent_name: "root".into(),
            },
            None,
        )
        .await
        .unwrap();

    assert!(durable.is_some());
    assert!(ephemeral.is_none());
    let replay = container
        .session_event_service
        .replay_after(0, Some(&session_id), 100)
        .await
        .unwrap();
    assert_eq!(replay.len(), 1);
    assert_eq!(replay[0].kind, SessionEventKind::ChatProgress);
}

#[tokio::test]
async fn unresolved_pending_requests_are_queryable_for_late_subscribers() {
    let container = setup().await;
    let session_id = SessionId("session-1".into());
    let (sender, _receiver) = tokio::sync::oneshot::channel();
    container
        .session_state
        .pending_permissions
        .lock()
        .await
        .insert(
            "permission-1".into(),
            PendingPermission::new(session_id.clone(), sender),
        );
    let (interaction_sender, _interaction_receiver) = tokio::sync::oneshot::channel();
    container
        .session_state
        .pending_interactions
        .lock()
        .await
        .insert(
            "interaction-1".into(),
            PendingInteraction::new(session_id.clone(), interaction_sender),
        );
    let (review_sender, _review_receiver) = tokio::sync::oneshot::channel();
    container
        .session_state
        .pending_plan_reviews
        .lock()
        .await
        .insert(
            "review-1".into(),
            PendingPlanReview::new(session_id.clone(), review_sender),
        );
    container
        .session_event_service
        .publish(
            &session_id,
            SessionEventKind::PermissionRequest,
            serde_json::json!({
                "run_id": "run-1",
                "session_id": session_id.as_str(),
                "request_id": "permission-1",
                "tool_name": "ShellExec",
                "action_description": "run command",
                "reason": "dangerous",
                "content_preview": "git status"
            }),
            SessionEventRetention::Durable,
            Some("permission-1"),
        )
        .await
        .unwrap();
    container
        .session_event_service
        .publish(
            &session_id,
            SessionEventKind::AskUser,
            serde_json::json!({
                "run_id": "run-1",
                "session_id": session_id.as_str(),
                "interaction_id": "interaction-1",
                "questions": [],
            }),
            SessionEventRetention::Durable,
            Some("interaction-1"),
        )
        .await
        .unwrap();
    container
        .session_event_service
        .publish(
            &session_id,
            SessionEventKind::PlanReviewRequest,
            serde_json::json!({
                "run_id": "run-1",
                "session_id": session_id.as_str(),
                "review_id": "review-1",
                "plan": {},
            }),
            SessionEventRetention::Durable,
            Some("review-1"),
        )
        .await
        .unwrap();
    container
        .session_event_service
        .publish_turn_event(
            &session_id,
            "run-1",
            &TurnEvent::PermissionRequest {
                request_id: "permission-1".into(),
                tool_name: "ShellExec".into(),
                action_description: "run command".into(),
                reason: "dangerous".into(),
                content_preview: Some("git status".into()),
            },
            None,
        )
        .await
        .unwrap();

    let pending = container
        .session_event_service
        .pending_events(&container.session_state, &session_id)
        .await
        .unwrap();

    assert_eq!(pending.len(), 3);
    assert_eq!(pending[0].kind, SessionEventKind::PermissionRequest);
    assert_eq!(pending[0].correlation_id.as_deref(), Some("permission-1"));
    assert_eq!(pending[1].kind, SessionEventKind::AskUser);
    assert_eq!(pending[2].kind, SessionEventKind::PlanReviewRequest);
}
