//! Durable journal for skill execution experience.

use std::path::Path;

use crate::error::SkillModuleError;
use crate::experience::ExperienceRecord;
use crate::jsonl_journal::JsonlJournal;

/// Append-only JSONL journal for recoverable skill-evolution evidence.
#[derive(Debug, Clone)]
pub struct ExperienceJournal {
    inner: JsonlJournal,
}

impl ExperienceJournal {
    /// Open or create a journal at `path`.
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, SkillModuleError> {
        Ok(Self {
            inner: JsonlJournal::open(path, "experience").await?,
        })
    }

    /// Append one experience and sync it to durable storage.
    pub async fn append(&self, record: &ExperienceRecord) -> Result<(), SkillModuleError> {
        self.inner.append(record).await
    }

    /// Load all records in append order.
    pub async fn load_all(&self) -> Result<Vec<ExperienceRecord>, SkillModuleError> {
        self.inner.load_all().await
    }

    /// Filesystem path backing this journal.
    pub fn path(&self) -> &Path {
        self.inner.path()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::experience::{ExperienceOutcome, ExperienceRecord, TokenUsage};

    #[tokio::test]
    async fn test_experience_journal_persists_records_across_reopen() {
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join("evolution/experiences.jsonl");
        let journal = ExperienceJournal::open(&path).await.unwrap();
        journal.append(&record("experience-1")).await.unwrap();
        drop(journal);

        let reopened = ExperienceJournal::open(&path).await.unwrap();
        let records = reopened.load_all().await.unwrap();

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].id, "experience-1");
        assert_eq!(records[0].skill_id.as_deref(), Some("review-rust"));
    }

    fn record(id: &str) -> ExperienceRecord {
        ExperienceRecord {
            id: id.to_string(),
            timestamp: "2026-07-15T00:00:00Z".to_string(),
            skill_id: Some("review-rust".to_string()),
            skill_version: Some("v1".to_string()),
            task_description: "Review a Rust module".to_string(),
            outcome: ExperienceOutcome::Success,
            trajectory_summary: "Reviewed the module and reported findings".to_string(),
            key_decisions: vec![],
            evidence: vec![],
            tool_calls: vec![],
            error_messages: vec![],
            duration_ms: 42,
            token_usage: TokenUsage::new(10, 5),
        }
    }
}
