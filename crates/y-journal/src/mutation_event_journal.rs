//! Append-only persistence for unified file mutation events.

use std::path::{Path, PathBuf};

use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;
use y_core::file_mutation::FileMutationEvent;

use crate::JournalError;

/// Durable JSONL journal for capability-declared file mutations.
pub struct MutationEventJournal {
    path: PathBuf,
    append_lock: Mutex<()>,
}

impl MutationEventJournal {
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, JournalError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(storage_error)?;
        }
        tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await
            .map_err(storage_error)?;
        Ok(Self {
            path,
            append_lock: Mutex::new(()),
        })
    }

    pub async fn append(&self, event: &FileMutationEvent) -> Result<(), JournalError> {
        let _guard = self.append_lock.lock().await;
        let mut line = serde_json::to_vec(event).map_err(storage_error)?;
        line.push(b'\n');
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .await
            .map_err(storage_error)?;
        file.write_all(&line).await.map_err(storage_error)?;
        file.flush().await.map_err(storage_error)?;
        file.sync_data().await.map_err(storage_error)?;
        Ok(())
    }
}

fn storage_error(error: impl std::fmt::Display) -> JournalError {
    JournalError::StorageError {
        message: error.to_string(),
    }
}
