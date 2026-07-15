//! Shared append-only JSONL persistence for skill lifecycle journals.

use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio::io::AsyncWriteExt;

use crate::error::SkillModuleError;

#[derive(Debug, Clone)]
pub(crate) struct JsonlJournal {
    path: PathBuf,
    record_name: &'static str,
}

impl JsonlJournal {
    pub(crate) async fn open(
        path: impl AsRef<Path>,
        record_name: &'static str,
    ) -> Result<Self, SkillModuleError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|error| SkillModuleError::Other {
                    message: format!(
                        "failed to create {record_name} journal directory {}: {error}",
                        parent.display()
                    ),
                })?;
        }
        tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await
            .map_err(|error| SkillModuleError::Other {
                message: format!(
                    "failed to open {record_name} journal {}: {error}",
                    path.display()
                ),
            })?;
        Ok(Self { path, record_name })
    }

    pub(crate) async fn append<T: Serialize>(&self, record: &T) -> Result<(), SkillModuleError> {
        let mut encoded = serde_json::to_vec(record)?;
        encoded.push(b'\n');

        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .await
            .map_err(|error| SkillModuleError::Other {
                message: format!(
                    "failed to open {} journal {} for append: {error}",
                    self.record_name,
                    self.path.display()
                ),
            })?;
        file.write_all(&encoded)
            .await
            .map_err(|error| SkillModuleError::Other {
                message: format!(
                    "failed to append {} journal {}: {error}",
                    self.record_name,
                    self.path.display()
                ),
            })?;
        file.sync_data()
            .await
            .map_err(|error| SkillModuleError::Other {
                message: format!(
                    "failed to sync {} journal {}: {error}",
                    self.record_name,
                    self.path.display()
                ),
            })
    }

    pub(crate) async fn load_all<T: DeserializeOwned>(&self) -> Result<Vec<T>, SkillModuleError> {
        let content = tokio::fs::read_to_string(&self.path)
            .await
            .map_err(|error| SkillModuleError::Other {
                message: format!(
                    "failed to read {} journal {}: {error}",
                    self.record_name,
                    self.path.display()
                ),
            })?;

        content
            .lines()
            .enumerate()
            .filter(|(_, line)| !line.trim().is_empty())
            .map(|(index, line)| {
                serde_json::from_str(line).map_err(|error| SkillModuleError::Other {
                    message: format!(
                        "invalid {} journal record at {}:{}: {error}",
                        self.record_name,
                        self.path.display(),
                        index + 1
                    ),
                })
            })
            .collect()
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}
