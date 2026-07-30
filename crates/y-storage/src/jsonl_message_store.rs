//! Shared JSONL message persistence used by both transcript projections.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::sync::Mutex;

use y_core::session::SessionError;
use y_core::types::{Message, SessionId};

#[derive(Debug, Clone, Copy)]
pub(crate) enum TranscriptKind {
    Context,
    Display,
}

impl TranscriptKind {
    const fn file_suffix(self) -> &'static str {
        match self {
            Self::Context => ".jsonl",
            Self::Display => ".display.jsonl",
        }
    }

    const fn temp_extension(self) -> &'static str {
        match self {
            Self::Context => "jsonl.tmp",
            Self::Display => "display.jsonl.tmp",
        }
    }

    const fn description(self) -> &'static str {
        match self {
            Self::Context => "transcript",
            Self::Display => "display transcript",
        }
    }
}

/// File mechanics shared by the context and display transcript adapters.
#[derive(Debug, Clone)]
pub(crate) struct JsonlMessageStore {
    base_dir: PathBuf,
    kind: TranscriptKind,
    mutation_lock: Arc<Mutex<()>>,
}

impl JsonlMessageStore {
    pub(crate) fn new(base_dir: impl Into<PathBuf>, kind: TranscriptKind) -> Self {
        Self {
            base_dir: base_dir.into(),
            kind,
            mutation_lock: Arc::new(Mutex::new(())),
        }
    }

    pub(crate) fn path(&self, session_id: &SessionId) -> PathBuf {
        self.base_dir.join(format!(
            "{}{}",
            session_id.as_str(),
            self.kind.file_suffix()
        ))
    }

    async fn ensure_dir(&self) -> Result<(), SessionError> {
        tokio::fs::create_dir_all(&self.base_dir)
            .await
            .map_err(|error| self.error("create", &error))
    }

    pub(crate) async fn append(
        &self,
        session_id: &SessionId,
        message: &Message,
    ) -> Result<(), SessionError> {
        self.ensure_dir().await?;
        let path = self.path(session_id);
        let mut line =
            serde_json::to_string(message).map_err(|error| SessionError::TranscriptError {
                message: format!("serialize message: {error}"),
            })?;
        line.push('\n');

        let _guard = self.mutation_lock.lock().await;
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await
            .map_err(|error| SessionError::TranscriptError {
                message: format!(
                    "open {} file {}: {error}",
                    self.kind.description(),
                    path.display()
                ),
            })?;
        file.write_all(line.as_bytes())
            .await
            .map_err(|error| self.error("write to", &error))?;
        file.flush()
            .await
            .map_err(|error| self.error("flush", &error))?;

        Ok(())
    }

    pub(crate) async fn read_all(
        &self,
        session_id: &SessionId,
    ) -> Result<Vec<Message>, SessionError> {
        let path = self.path(session_id);
        if !path.exists() {
            return Ok(Vec::new());
        }
        read_messages_from_file(&path).await
    }

    pub(crate) async fn read_last(
        &self,
        session_id: &SessionId,
        count: usize,
    ) -> Result<Vec<Message>, SessionError> {
        if count == 0 {
            return Ok(Vec::new());
        }
        let path = self.path(session_id);
        if !path.exists() {
            return Ok(Vec::new());
        }
        read_last_messages_from_file(&path, count).await
    }

    pub(crate) async fn message_count(
        &self,
        session_id: &SessionId,
    ) -> Result<usize, SessionError> {
        let path = self.path(session_id);
        if !path.exists() {
            return Ok(0);
        }
        fold_messages_from_file(&path, 0, |count, _message| *count += 1).await
    }

    pub(crate) async fn truncate(
        &self,
        session_id: &SessionId,
        keep_count: usize,
    ) -> Result<usize, SessionError> {
        let _guard = self.mutation_lock.lock().await;
        let mut messages = self.read_all(session_id).await?;
        if keep_count >= messages.len() {
            return Ok(0);
        }

        let removed = messages.len() - keep_count;
        messages.truncate(keep_count);
        self.rewrite(session_id, &messages).await?;
        Ok(removed)
    }

    pub(crate) async fn update_message(
        &self,
        session_id: &SessionId,
        message_id: &str,
        updated: &Message,
    ) -> Result<bool, SessionError> {
        let _guard = self.mutation_lock.lock().await;
        let mut messages = self.read_all(session_id).await?;
        let Some(message) = messages
            .iter_mut()
            .find(|message| message.message_id == message_id)
        else {
            return Ok(false);
        };

        message.clone_from(updated);
        self.rewrite(session_id, &messages).await?;
        Ok(true)
    }

    async fn rewrite(
        &self,
        session_id: &SessionId,
        messages: &[Message],
    ) -> Result<(), SessionError> {
        let path = self.path(session_id);
        let temp_path = path.with_extension(self.kind.temp_extension());
        let mut content = String::new();
        for message in messages {
            let line =
                serde_json::to_string(message).map_err(|error| SessionError::TranscriptError {
                    message: format!("serialize message: {error}"),
                })?;
            content.push_str(&line);
            content.push('\n');
        }

        tokio::fs::write(&temp_path, content.as_bytes())
            .await
            .map_err(|error| self.error("write temp", &error))?;
        tokio::fs::rename(&temp_path, &path)
            .await
            .map_err(|error| self.error("rename temp", &error))
    }

    fn error(&self, action: &str, error: &std::io::Error) -> SessionError {
        SessionError::TranscriptError {
            message: format!("{action} {}: {error}", self.kind.description()),
        }
    }
}

async fn read_messages_from_file(path: &Path) -> Result<Vec<Message>, SessionError> {
    fold_messages_from_file(path, Vec::new(), Vec::push).await
}

async fn fold_messages_from_file<T>(
    path: &Path,
    initial: T,
    mut fold: impl FnMut(&mut T, Message),
) -> Result<T, SessionError> {
    let file = open_transcript(path).await?;
    let reader = tokio::io::BufReader::new(file);
    let mut lines = reader.lines();
    let mut result = initial;
    let mut skipped = 0usize;

    while let Some(line) = next_line(&mut lines).await? {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match serde_json::from_str::<Message>(trimmed) {
            Ok(message) => fold(&mut result, message),
            Err(error) => {
                skipped += 1;
                tracing::warn!(
                    path = %path.display(),
                    error = %error,
                    "skipping unparseable transcript line",
                );
            }
        }
    }

    log_recovery(path, skipped, "full");
    Ok(result)
}

async fn read_last_messages_from_file(
    path: &Path,
    count: usize,
) -> Result<Vec<Message>, SessionError> {
    let file = open_transcript(path).await?;
    let reader = tokio::io::BufReader::new(file);
    let mut lines = reader.lines();
    let mut ring = VecDeque::with_capacity(count);

    while let Some(line) = next_line(&mut lines).await? {
        if line.trim().is_empty() {
            continue;
        }
        if ring.len() == count {
            ring.pop_front();
        }
        ring.push_back(line);
    }

    let mut messages = Vec::with_capacity(ring.len());
    let mut skipped = 0usize;
    for line in ring {
        match serde_json::from_str::<Message>(line.trim()) {
            Ok(message) => messages.push(message),
            Err(error) => {
                skipped += 1;
                tracing::warn!(
                    path = %path.display(),
                    error = %error,
                    "skipping unparseable transcript line (tail read)",
                );
            }
        }
    }

    log_recovery(path, skipped, "tail");
    Ok(messages)
}

async fn open_transcript(path: &Path) -> Result<tokio::fs::File, SessionError> {
    tokio::fs::File::open(path)
        .await
        .map_err(|error| SessionError::TranscriptError {
            message: format!("open transcript {}: {error}", path.display()),
        })
}

async fn next_line(
    lines: &mut tokio::io::Lines<tokio::io::BufReader<tokio::fs::File>>,
) -> Result<Option<String>, SessionError> {
    lines
        .next_line()
        .await
        .map_err(|error| SessionError::TranscriptError {
            message: format!("read line: {error}"),
        })
}

fn log_recovery(path: &Path, skipped: usize, read_kind: &'static str) {
    if skipped > 0 {
        tracing::warn!(
            path = %path.display(),
            skipped,
            read_kind,
            "recovered transcript with skipped malformed lines",
        );
    }
}
