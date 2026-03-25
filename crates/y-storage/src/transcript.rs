//! JSONL-based `TranscriptStore` implementation.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::sync::Mutex;
use tracing::instrument;

use y_core::session::{SessionError, TranscriptStore};
use y_core::types::{Message, SessionId};

/// JSONL file-based transcript store.
///
/// Each session's messages are stored in a separate `.jsonl` file
/// where each line is a JSON-serialized `Message`.
#[derive(Debug, Clone)]
pub struct JsonlTranscriptStore {
    /// Base directory for transcript files.
    base_dir: PathBuf,
    /// Write lock to serialise concurrent appends.
    ///
    /// `O_APPEND` guarantees atomic positioning but NOT atomic writes for
    /// buffers exceeding `PIPE_BUF` (~4KB). Serialising through this mutex
    /// prevents interleaved bytes when long messages are written concurrently.
    write_lock: Arc<Mutex<()>>,
}

impl JsonlTranscriptStore {
    /// Create a new transcript store with the given base directory.
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
            write_lock: Arc::new(Mutex::new(())),
        }
    }

    /// Get the file path for a session's transcript.
    fn transcript_path(&self, session_id: &SessionId) -> PathBuf {
        self.base_dir.join(format!("{}.jsonl", session_id.as_str()))
    }

    /// Ensure the base directory exists.
    async fn ensure_dir(&self) -> Result<(), SessionError> {
        tokio::fs::create_dir_all(&self.base_dir)
            .await
            .map_err(|e| SessionError::TranscriptError {
                message: format!("create transcript dir: {e}"),
            })
    }
}

#[async_trait]
impl TranscriptStore for JsonlTranscriptStore {
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
                message: format!("open transcript file {}: {e}", path.display()),
            })?;

        file.write_all(line.as_bytes())
            .await
            .map_err(|e| SessionError::TranscriptError {
                message: format!("write to transcript: {e}"),
            })?;

        file.flush()
            .await
            .map_err(|e| SessionError::TranscriptError {
                message: format!("flush transcript: {e}"),
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

    #[instrument(skip(self), fields(session_id = %session_id, count = count))]
    async fn read_last(
        &self,
        session_id: &SessionId,
        count: usize,
    ) -> Result<Vec<Message>, SessionError> {
        let all = self.read_all(session_id).await?;
        let start = all.len().saturating_sub(count);
        Ok(all[start..].to_vec())
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
                    message: format!("open transcript: {e}"),
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
        let tmp_path = path.with_extension("jsonl.tmp");

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
                message: format!("write temp transcript: {e}"),
            })?;

        tokio::fs::rename(&tmp_path, &path)
            .await
            .map_err(|e| SessionError::TranscriptError {
                message: format!("rename temp transcript: {e}"),
            })?;

        Ok(removed)
    }
}

/// Read all messages from a JSONL file.
pub(crate) async fn read_messages_from_file(path: &Path) -> Result<Vec<Message>, SessionError> {
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|e| SessionError::TranscriptError {
            message: format!("open transcript {}: {e}", path.display()),
        })?;

    let reader = tokio::io::BufReader::new(file);
    let mut lines = reader.lines();
    let mut messages = Vec::new();

    while let Some(line) = lines
        .next_line()
        .await
        .map_err(|e| SessionError::TranscriptError {
            message: format!("read line: {e}"),
        })?
    {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let msg: Message =
            serde_json::from_str(trimmed).map_err(|e| SessionError::TranscriptError {
                message: format!("parse message: {e}"),
            })?;
        messages.push(msg);
    }

    Ok(messages)
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
