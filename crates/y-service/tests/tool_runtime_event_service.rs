use y_core::runtime::{RuntimeBackend, ToolRuntimeEvent, ToolRuntimeEventKind, ToolRuntimeStream};
use y_core::session_event::{SessionEventKind, SessionEventRetention};
use y_core::types::SessionId;
use y_service::{ServiceConfig, ServiceContainer, ToolRuntimeEventService};

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

fn runtime_event(kind: ToolRuntimeEventKind) -> ToolRuntimeEvent {
    ToolRuntimeEvent::new(
        SessionId("session-1".into()),
        "process-1",
        "ShellExec",
        Some(RuntimeBackend::Native),
        kind,
    )
}

#[tokio::test]
async fn terminal_events_are_durable_before_live_publication() {
    let container = setup().await;
    let service = ToolRuntimeEventService::new(container.session_event_service.clone()).0;
    let mut live = service.subscribe();

    let published = service
        .publish(runtime_event(ToolRuntimeEventKind::ProcessCompleted {
            exit_code: 0,
            duration_ms: 125,
        }))
        .await
        .unwrap();
    let delivered = live.recv().await.unwrap();

    assert_eq!(delivered.event_id, published.event_id);
    let replay = container
        .session_event_service
        .replay_after(0, Some(&SessionId("session-1".into())), 100)
        .await
        .unwrap();
    assert_eq!(replay.len(), 1);
    assert_eq!(replay[0].kind, SessionEventKind::ToolRuntime);
    assert_eq!(replay[0].retention, SessionEventRetention::Durable);
}

#[tokio::test]
async fn output_chunks_use_short_lived_retention() {
    let container = setup().await;
    let service = ToolRuntimeEventService::new(container.session_event_service.clone()).0;

    service
        .publish(runtime_event(ToolRuntimeEventKind::OutputChunk {
            stream: ToolRuntimeStream::Stderr,
            content: "warning\n".into(),
        }))
        .await
        .unwrap();

    let replay = container
        .session_event_service
        .replay_after(0, Some(&SessionId("session-1".into())), 100)
        .await
        .unwrap();
    assert_eq!(replay[0].retention, SessionEventRetention::ShortLived);
}

#[tokio::test]
async fn runtime_sink_preserves_started_output_terminal_order() {
    let container = setup().await;
    let (service, sink) = ToolRuntimeEventService::new(container.session_event_service.clone());
    let mut live = service.subscribe();

    sink.publish(runtime_event(ToolRuntimeEventKind::ProcessStarted {
        command: "printf ready".into(),
        working_dir: None,
    }));
    sink.publish(runtime_event(ToolRuntimeEventKind::OutputChunk {
        stream: ToolRuntimeStream::Stdout,
        content: "ready".into(),
    }));
    sink.publish(runtime_event(ToolRuntimeEventKind::ProcessCompleted {
        exit_code: 0,
        duration_ms: 10,
    }));

    let first = live.recv().await.unwrap();
    let second = live.recv().await.unwrap();
    let third = live.recv().await.unwrap();

    assert!(first.event_id < second.event_id && second.event_id < third.event_id);
    assert!(matches!(
        first.event.kind,
        ToolRuntimeEventKind::ProcessStarted { .. }
    ));
    assert!(matches!(
        second.event.kind,
        ToolRuntimeEventKind::OutputChunk { .. }
    ));
    assert!(matches!(
        third.event.kind,
        ToolRuntimeEventKind::ProcessCompleted { .. }
    ));
}
