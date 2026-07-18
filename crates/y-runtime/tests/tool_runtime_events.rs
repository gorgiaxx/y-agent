use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::mpsc;
use y_core::runtime::{
    ExecutionRequest, ProcessCapability, RuntimeAdapter, RuntimeBackend, RuntimeCapability,
    ToolRuntimeEvent, ToolRuntimeEventKind, ToolRuntimeEventSink,
};
use y_core::types::SessionId;
use y_runtime::{NativeRuntime, RuntimeConfig};

struct RecordingSink {
    tx: mpsc::UnboundedSender<ToolRuntimeEvent>,
}

impl ToolRuntimeEventSink for RecordingSink {
    fn publish(&self, event: ToolRuntimeEvent) {
        let _ = self.tx.send(event);
    }
}

#[tokio::test]
async fn native_background_process_pushes_started_output_and_completion() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let sink = Arc::new(RecordingSink { tx });
    let runtime = NativeRuntime::with_event_sink(RuntimeConfig::default(), None, sink);
    let (shell, shell_flag) = y_core::platform::shell_command();
    let request = ExecutionRequest {
        command: shell,
        args: vec![shell_flag, "printf runtime-output".into()],
        working_dir: None,
        env: HashMap::new(),
        stdin: None,
        owner_session_id: Some(SessionId("session-1".into())),
        event_tool_name: Some("LspServer".to_string()),
        capabilities: RuntimeCapability {
            process: ProcessCapability {
                shell: true,
                background: true,
                ..Default::default()
            },
            ..Default::default()
        },
        image: None,
    };

    let handle = runtime.spawn(request).await.unwrap();
    assert_eq!(handle.backend, RuntimeBackend::Native);

    let mut events = Vec::new();
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while let Some(event) = rx.recv().await {
            let terminal = event.is_terminal();
            events.push(event);
            if terminal {
                break;
            }
        }
    })
    .await
    .unwrap();

    assert!(matches!(
        events.first().map(|event| &event.kind),
        Some(ToolRuntimeEventKind::ProcessStarted { .. })
    ));
    assert!(events.iter().all(|event| event.tool_name == "LspServer"));
    assert!(events.iter().any(|event| matches!(
        &event.kind,
        ToolRuntimeEventKind::OutputChunk { content, .. } if content == "runtime-output"
    )));
    assert!(events.iter().any(|event| matches!(
        event.kind,
        ToolRuntimeEventKind::ProcessCompleted { exit_code: 0, .. }
    )));
    assert!(matches!(
        events.last().map(|event| &event.kind),
        Some(ToolRuntimeEventKind::ProcessCompleted { exit_code: 0, .. })
    ));
}
