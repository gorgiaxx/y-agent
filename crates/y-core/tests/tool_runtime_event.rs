use y_core::runtime::{RuntimeBackend, ToolRuntimeEvent, ToolRuntimeEventKind, ToolRuntimeStream};
use y_core::types::SessionId;

#[test]
fn output_chunk_serializes_as_transport_neutral_runtime_data() {
    let event = ToolRuntimeEvent::new(
        SessionId("session-1".into()),
        "process-1",
        "ShellExec",
        Some(RuntimeBackend::Native),
        ToolRuntimeEventKind::OutputChunk {
            stream: ToolRuntimeStream::Stdout,
            content: "ready\n".into(),
        },
    );

    let value = serde_json::to_value(&event).unwrap();

    assert_eq!(value["session_id"], "session-1");
    assert_eq!(value["task_id"], "process-1");
    assert_eq!(value["type"], "output_chunk");
    assert_eq!(value["stream"], "stdout");
    assert_eq!(value["content"], "ready\n");
    assert!(!event.is_terminal());
}

#[test]
fn completed_and_killed_events_are_terminal() {
    let completed = ToolRuntimeEventKind::ProcessCompleted {
        exit_code: 0,
        duration_ms: 125,
    };
    let killed = ToolRuntimeEventKind::ProcessKilled { duration_ms: 250 };

    assert!(completed.is_terminal());
    assert!(killed.is_terminal());
}
