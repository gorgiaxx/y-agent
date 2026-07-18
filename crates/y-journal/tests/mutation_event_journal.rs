use y_core::file_mutation::{FileMutationEvent, FileMutationOperation};
use y_core::types::SessionId;
use y_journal::MutationEventJournal;

#[tokio::test]
async fn appends_file_mutation_events_as_jsonl_without_inline_content() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("events.jsonl");
    let journal = MutationEventJournal::open(&path).await.unwrap();
    let event = FileMutationEvent {
        tool_call_id: "call-1".into(),
        session_id: SessionId("session-1".into()),
        agent_id: "root".into(),
        operation: FileMutationOperation::Modify,
        absolute_path: "/workspace/file.txt".into(),
        destination_path: None,
        before_hash: Some("sha256:before".into()),
        after_hash: Some("sha256:after".into()),
        previous_content_ref: Some("cas:sha256:before".into()),
        new_content_ref: Some("cas:sha256:after".into()),
        is_new_file: false,
    };

    journal.append(&event).await.unwrap();

    let persisted = tokio::fs::read_to_string(path).await.unwrap();
    assert!(persisted.ends_with('\n'));
    let value: serde_json::Value = serde_json::from_str(persisted.trim()).unwrap();
    assert_eq!(value["tool_call_id"], "call-1");
    assert!(value.get("content").is_none());
}
