//! JSONL-based `DisplayTranscriptStore` implementation.
//!
//! Append-only transcript for GUI display. Uses `{session_id}.display.jsonl`
//! file naming to coexist alongside the context transcript.
//!
//! Design reference: `GUI_SESSION_SEPARATION_PLAN.md` §3.1, Step 1.2.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::sync::Mutex;
use tracing::instrument;

use y_core::session::{DisplayTranscriptStore, SessionError};
use y_core::types::{Message, SessionId};

use crate::transcript::read_messages_from_file;

/// JSONL file-based display transcript store.
///
/// Each session's display messages are stored in a separate `.display.jsonl`
/// file where each line is a JSON-serialized `Message`.
///
/// This store is append-only (never compacted). Only truncated during
/// undo/rollback operations.
#[derive(Debug, Clone)]
pub struct JsonlDisplayTranscriptStore {
    /// Base directory for transcript files.
    base_dir: PathBuf,
    /// Write lock to serialise concurrent appends (same rationale as
    /// `JsonlTranscriptStore::write_lock`).
    write_lock: Arc<Mutex<()>>,
}

impl JsonlDisplayTranscriptStore {
    /// Create a new display transcript store with the given base directory.
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
            write_lock: Arc::new(Mutex::new(())),
        }
    }

    /// Get the file path for a session's display transcript.
    fn transcript_path(&self, session_id: &SessionId) -> PathBuf {
        self.base_dir
            .join(format!("{}.display.jsonl", session_id.as_str()))
    }

    /// Ensure the base directory exists.
    async fn ensure_dir(&self) -> Result<(), SessionError> {
        tokio::fs::create_dir_all(&self.base_dir)
            .await
            .map_err(|e| SessionError::TranscriptError {
                message: format!("create display transcript dir: {e}"),
            })
    }
}

#[async_trait]
impl DisplayTranscriptStore for JsonlDisplayTranscriptStore {
    #[instrument(skip(self, message), fields(session_id = %session_id))]
    async fn append(&self, session_id: &SessionId, message: &Message) -> Result<(), SessionError> {
        self.ensure_dir().await?;

        let path = self.transcript_path(session_id);
        let mut line =
            serde_json::to_string(message).map_err(|e| SessionError::TranscriptError {
                message: format!("serialize message: {e}"),
            })?;
        line.push('\n');

        // Serialise writes so concurrent appends cannot interleave bytes.
        let _guard = self.write_lock.lock().await;

        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await
            .map_err(|e| SessionError::TranscriptError {
                message: format!("open display transcript file {}: {e}", path.display()),
            })?;

        file.write_all(line.as_bytes())
            .await
            .map_err(|e| SessionError::TranscriptError {
                message: format!("write to display transcript: {e}"),
            })?;

        file.flush()
            .await
            .map_err(|e| SessionError::TranscriptError {
                message: format!("flush display transcript: {e}"),
            })?;

        Ok(())
    }

    #[instrument(skip(self), fields(session_id = %session_id))]
    async fn read_all(&self, session_id: &SessionId) -> Result<Vec<Message>, SessionError> {
        let path = self.transcript_path(session_id);

        if !path.exists() {
            return Ok(Vec::new());
        }

        read_messages_from_file(&path).await
    }

    #[instrument(skip(self), fields(session_id = %session_id))]
    async fn message_count(&self, session_id: &SessionId) -> Result<usize, SessionError> {
        let path = self.transcript_path(session_id);

        if !path.exists() {
            return Ok(0);
        }

        let file =
            tokio::fs::File::open(&path)
                .await
                .map_err(|e| SessionError::TranscriptError {
                    message: format!("open display transcript: {e}"),
                })?;

        let reader = tokio::io::BufReader::new(file);
        let mut lines = reader.lines();
        let mut count = 0;

        while let Some(line) =
            lines
                .next_line()
                .await
                .map_err(|e| SessionError::TranscriptError {
                    message: format!("read line: {e}"),
                })?
        {
            if !line.trim().is_empty() {
                count += 1;
            }
        }

        Ok(count)
    }

    #[instrument(skip(self), fields(session_id = %session_id, keep_count = keep_count))]
    async fn truncate(
        &self,
        session_id: &SessionId,
        keep_count: usize,
    ) -> Result<usize, SessionError> {
        let all = self.read_all(session_id).await?;
        if keep_count >= all.len() {
            return Ok(0);
        }

        let removed = all.len() - keep_count;
        let kept = &all[..keep_count];

        // Atomic rewrite: write to temp file, then rename.
        let path = self.transcript_path(session_id);
        let tmp_path = path.with_extension("display.jsonl.tmp");

        let mut content = String::new();
        for msg in kept {
            let line = serde_json::to_string(msg).map_err(|e| SessionError::TranscriptError {
                message: format!("serialize message: {e}"),
            })?;
            content.push_str(&line);
            content.push('\n');
        }

        tokio::fs::write(&tmp_path, content.as_bytes())
            .await
            .map_err(|e| SessionError::TranscriptError {
                message: format!("write temp display transcript: {e}"),
            })?;

        tokio::fs::rename(&tmp_path, &path)
            .await
            .map_err(|e| SessionError::TranscriptError {
                message: format!("rename temp display transcript: {e}"),
            })?;

        Ok(removed)
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
