//! JSONL-based `DisplayTranscriptStore` implementation.
//!
//! Append-only transcript for GUI display. Uses `{session_id}.display.jsonl`
//! file naming to coexist alongside the context transcript.
//!
//! Frontend contract: `docs/standards/FRONTEND_REUSE_STANDARD.md`.

use std::path::PathBuf;

use async_trait::async_trait;
use tracing::instrument;

use y_core::session::{DisplayTranscriptStore, SessionError};
use y_core::types::{Message, SessionId};

use crate::jsonl_message_store::{JsonlMessageStore, TranscriptKind};

/// JSONL file-based display transcript store.
///
/// Each session's display messages are stored in a separate `.display.jsonl`
/// file where each line is a JSON-serialized `Message`.
///
/// This store is append-only (never compacted). Only truncated during
/// undo/rollback operations.
#[derive(Debug, Clone)]
pub struct JsonlDisplayTranscriptStore {
    inner: JsonlMessageStore,
}

impl JsonlDisplayTranscriptStore {
    /// Create a new display transcript store with the given base directory.
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            inner: JsonlMessageStore::new(base_dir, TranscriptKind::Display),
        }
    }

    /// Get the file path for a session's display transcript.
    #[cfg(test)]
    fn transcript_path(&self, session_id: &SessionId) -> PathBuf {
        self.inner.path(session_id)
    }
}

#[async_trait]
impl DisplayTranscriptStore for JsonlDisplayTranscriptStore {
    #[instrument(skip(self, message), fields(session_id = %session_id))]
    async fn append(&self, session_id: &SessionId, message: &Message) -> Result<(), SessionError> {
        self.inner.append(session_id, message).await
    }

    #[instrument(skip(self), fields(session_id = %session_id))]
    async fn read_all(&self, session_id: &SessionId) -> Result<Vec<Message>, SessionError> {
        self.inner.read_all(session_id).await
    }

    #[instrument(skip(self), fields(session_id = %session_id))]
    async fn message_count(&self, session_id: &SessionId) -> Result<usize, SessionError> {
        self.inner.message_count(session_id).await
    }

    #[instrument(skip(self), fields(session_id = %session_id, keep_count = keep_count))]
    async fn truncate(
        &self,
        session_id: &SessionId,
        keep_count: usize,
    ) -> Result<usize, SessionError> {
        self.inner.truncate(session_id, keep_count).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use y_core::types::{Message, Role};

    fn test_message(content: &str) -> Message {
        Message {
            message_id: y_core::types::generate_message_id(),
            role: Role::User,
            content: content.to_string(),
            tool_call_id: None,
            tool_calls: vec![],
            timestamp: chrono::Utc::now(),
            metadata: serde_json::Value::Null,
        }
    }

    fn temp_store() -> (tempfile::TempDir, JsonlDisplayTranscriptStore) {
        let dir = tempfile::tempdir().expect("create temp dir");
        let store = JsonlDisplayTranscriptStore::new(dir.path());
        (dir, store)
    }

    #[tokio::test]
    async fn test_display_append_and_read_all() {
        let (_dir, store) = temp_store();
        let session_id = SessionId::new();

        store
            .append(&session_id, &test_message("hello"))
            .await
            .unwrap();
        store
            .append(&session_id, &test_message("world"))
            .await
            .unwrap();
        store.append(&session_id, &test_message("!")).await.unwrap();

        let messages = store.read_all(&session_id).await.unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].content, "hello");
        assert_eq!(messages[1].content, "world");
        assert_eq!(messages[2].content, "!");
    }

    #[tokio::test]
    async fn test_display_message_count() {
        let (_dir, store) = temp_store();
        let session_id = SessionId::new();

        for i in 0..5 {
            store
                .append(&session_id, &test_message(&format!("msg-{i}")))
                .await
                .unwrap();
        }

        let count = store.message_count(&session_id).await.unwrap();
        assert_eq!(count, 5);
    }

    #[tokio::test]
    async fn test_display_message_count_skips_corrupt_lines() {
        let (_dir, store) = temp_store();
        let session_id = SessionId::new();
        let path = store.transcript_path(&session_id);
        let first = serde_json::to_string(&test_message("first")).unwrap();
        let third = serde_json::to_string(&test_message("third")).unwrap();

        tokio::fs::write(&path, format!("{first}\nnot-json\n{third}\n"))
            .await
            .unwrap();

        let count = store.message_count(&session_id).await.unwrap();
        assert_eq!(count, 2, "only readable messages should be counted");
    }

    #[tokio::test]
    async fn test_display_empty_session() {
        let (_dir, store) = temp_store();
        let session_id = SessionId::new();

        let messages = store.read_all(&session_id).await.unwrap();
        assert!(messages.is_empty());

        let count = store.message_count(&session_id).await.unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_display_truncate() {
        let (_dir, store) = temp_store();
        let session_id = SessionId::new();

        for i in 0..5 {
            store
                .append(&session_id, &test_message(&format!("msg-{i}")))
                .await
                .unwrap();
        }

        let removed = store.truncate(&session_id, 3).await.unwrap();
        assert_eq!(removed, 2);

        let messages = store.read_all(&session_id).await.unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].content, "msg-0");
        assert_eq!(messages[2].content, "msg-2");
    }

    #[tokio::test]
    async fn test_display_truncate_noop() {
        let (_dir, store) = temp_store();
        let session_id = SessionId::new();

        store
            .append(&session_id, &test_message("solo"))
            .await
            .unwrap();

        let removed = store.truncate(&session_id, 5).await.unwrap();
        assert_eq!(removed, 0);
    }

    #[tokio::test]
    async fn test_display_uses_display_jsonl_extension() {
        let (dir, store) = temp_store();
        let session_id = SessionId::new();

        store
            .append(&session_id, &test_message("test"))
            .await
            .unwrap();

        let path = dir
            .path()
            .join(format!("{}.display.jsonl", session_id.as_str()));
        assert!(
            path.exists(),
            "display transcript should use .display.jsonl"
        );
    }

    #[tokio::test]
    async fn test_display_concurrent_append() {
        let (_dir, store) = temp_store();
        let session_id = SessionId::new();

        let mut handles = Vec::new();
        for i in 0..10 {
            let store = store.clone();
            let sid = session_id.clone();
            handles.push(tokio::spawn(async move {
                store
                    .append(&sid, &test_message(&format!("concurrent-{i}")))
                    .await
                    .unwrap();
            }));
        }

        for h in handles {
            h.await.unwrap();
        }

        let messages = store.read_all(&session_id).await.unwrap();
        assert_eq!(
            messages.len(),
            10,
            "all concurrent messages should be present"
        );
    }
}
