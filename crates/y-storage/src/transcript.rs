//! JSONL-based `TranscriptStore` implementation.

use std::path::PathBuf;

use async_trait::async_trait;
use tracing::instrument;

use y_core::session::{SessionError, TranscriptStore};
use y_core::types::{Message, SessionId};

use crate::jsonl_message_store::{JsonlMessageStore, TranscriptKind};

/// JSONL file-based transcript store.
///
/// Each session's messages are stored in a separate `.jsonl` file
/// where each line is a JSON-serialized `Message`.
#[derive(Debug, Clone)]
pub struct JsonlTranscriptStore {
    inner: JsonlMessageStore,
}

impl JsonlTranscriptStore {
    /// Create a new transcript store with the given base directory.
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            inner: JsonlMessageStore::new(base_dir, TranscriptKind::Context),
        }
    }

    /// Get the file path for a session's transcript.
    #[cfg(test)]
    fn transcript_path(&self, session_id: &SessionId) -> PathBuf {
        self.inner.path(session_id)
    }
}

#[async_trait]
impl TranscriptStore for JsonlTranscriptStore {
    #[instrument(skip(self, message), fields(session_id = %session_id))]
    async fn append(&self, session_id: &SessionId, message: &Message) -> Result<(), SessionError> {
        self.inner.append(session_id, message).await
    }

    #[instrument(skip(self), fields(session_id = %session_id))]
    async fn read_all(&self, session_id: &SessionId) -> Result<Vec<Message>, SessionError> {
        self.inner.read_all(session_id).await
    }

    #[instrument(skip(self), fields(session_id = %session_id, count = count))]
    async fn read_last(
        &self,
        session_id: &SessionId,
        count: usize,
    ) -> Result<Vec<Message>, SessionError> {
        self.inner.read_last(session_id, count).await
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

    #[instrument(skip(self, updated), fields(session_id = %session_id, message_id = %message_id))]
    async fn update_message(
        &self,
        session_id: &SessionId,
        message_id: &str,
        updated: &Message,
    ) -> Result<bool, SessionError> {
        self.inner
            .update_message(session_id, message_id, updated)
            .await
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

    fn temp_store() -> (tempfile::TempDir, JsonlTranscriptStore) {
        let dir = tempfile::tempdir().expect("create temp dir");
        let store = JsonlTranscriptStore::new(dir.path());
        (dir, store)
    }

    #[tokio::test]
    async fn test_transcript_append_and_read_all() {
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
    async fn test_read_all_skips_corrupt_lines() {
        // A crash mid-append can leave a truncated/garbled JSONL line. A single
        // bad line must NOT abort the whole read (which the GUI would render as
        // an empty session) -- valid lines around it must still be recovered.
        let (_dir, store) = temp_store();
        let session_id = SessionId::new();

        let path = store.transcript_path(&session_id);
        let first = serde_json::to_string(&test_message("first")).unwrap();
        tokio::fs::write(
            &path,
            format!("{first}\n{{\"role\":\"assistant\",\"content\": <-- truncated\n"),
        )
        .await
        .unwrap();

        // A valid trailing message appended after the corrupt line.
        store
            .append(&session_id, &test_message("third"))
            .await
            .unwrap();

        let messages = store.read_all(&session_id).await.unwrap();
        assert_eq!(messages.len(), 2, "corrupt middle line should be skipped");
        assert_eq!(messages[0].content, "first");
        assert_eq!(messages[1].content, "third");
    }

    #[tokio::test]
    async fn test_transcript_read_last_n() {
        let (_dir, store) = temp_store();
        let session_id = SessionId::new();

        for i in 0..10 {
            store
                .append(&session_id, &test_message(&format!("msg-{i}")))
                .await
                .unwrap();
        }

        let last_3 = store.read_last(&session_id, 3).await.unwrap();
        assert_eq!(last_3.len(), 3);
        assert_eq!(last_3[0].content, "msg-7");
        assert_eq!(last_3[1].content, "msg-8");
        assert_eq!(last_3[2].content, "msg-9");
    }

    #[tokio::test]
    async fn test_transcript_read_last_zero_returns_empty() {
        let (_dir, store) = temp_store();
        let session_id = SessionId::new();
        store
            .append(&session_id, &test_message("present"))
            .await
            .unwrap();

        let messages = store.read_last(&session_id, 0).await.unwrap();
        assert!(messages.is_empty());
    }

    #[tokio::test]
    async fn test_transcript_message_count() {
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
    async fn test_transcript_message_count_skips_corrupt_lines() {
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
    async fn test_transcript_empty_session() {
        let (_dir, store) = temp_store();
        let session_id = SessionId::new();

        let messages = store.read_all(&session_id).await.unwrap();
        assert!(messages.is_empty());
    }

    #[tokio::test]
    async fn test_transcript_jsonl_format() {
        let (dir, store) = temp_store();
        let session_id = SessionId::new();

        store
            .append(&session_id, &test_message("test"))
            .await
            .unwrap();

        // Read the raw file and verify each line is valid JSON.
        let path = dir.path().join(format!("{}.jsonl", session_id.as_str()));
        let content = tokio::fs::read_to_string(&path).await.unwrap();
        for line in content.lines() {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                assert!(
                    serde_json::from_str::<serde_json::Value>(trimmed).is_ok(),
                    "line should be valid JSON: {trimmed}"
                );
            }
        }
    }

    #[tokio::test]
    async fn test_transcript_concurrent_append() {
        let (_dir, store) = temp_store();
        let session_id = SessionId::new();

        // Write from multiple tasks concurrently.
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
