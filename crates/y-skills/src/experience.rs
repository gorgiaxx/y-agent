//! Experience capture: structured records of skill execution outcomes.
//!
//! Design reference: skill-versioning-evolution-design.md §Experience Capture
//!
//! Every skill execution can be recorded as an `ExperienceRecord`,
//! capturing the outcome, key decisions, evidence, and resource usage.
//! These records feed the evolution pipeline's pattern extraction.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Outcome of a skill execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExperienceOutcome {
    /// Task completed fully and correctly.
    Success,
    /// Task partially completed (some goals met).
    Partial,
    /// Task failed.
    Failure,
}

/// How a piece of evidence was obtained.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceProvenance {
    /// Explicitly stated by the user.
    UserStated,
    /// User corrected agent behavior.
    UserCorrection,
    /// Derived from task outcome.
    TaskOutcome,
    /// Observed by the agent during execution.
    AgentObservation,
}

/// A piece of evidence linked to an experience.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceEntry {
    /// What was observed or stated.
    pub content: String,
    /// How this evidence was obtained.
    pub provenance: EvidenceProvenance,
}

/// A tool call made during skill execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRecord {
    /// Tool name.
    pub name: String,
    /// Whether the tool call succeeded.
    pub success: bool,
    /// Duration of the tool call in milliseconds.
    pub duration_ms: u64,
}

/// Structured token usage for an experience.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    /// Prompt tokens consumed.
    pub prompt: u32,
    /// Completion tokens generated.
    pub completion: u32,
    /// Total tokens.
    pub total: u32,
}

impl TokenUsage {
    /// Create a new token usage record.
    pub fn new(prompt: u32, completion: u32) -> Self {
        Self {
            prompt,
            completion,
            total: prompt + completion,
        }
    }
}

/// A complete record of one skill execution experience.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperienceRecord {
    /// Unique record ID.
    pub id: String,
    /// When the execution occurred.
    pub timestamp: String,
    /// Optional skill ID (None if no specific skill was used).
    pub skill_id: Option<String>,
    /// Optional skill version at the time.
    pub skill_version: Option<String>,
    /// Description of the task that was attempted.
    pub task_description: String,
    /// Overall outcome.
    pub outcome: ExperienceOutcome,
    /// Summary of the execution trajectory.
    pub trajectory_summary: String,
    /// Key decisions made during execution.
    pub key_decisions: Vec<String>,
    /// Evidence entries.
    pub evidence: Vec<EvidenceEntry>,
    /// Tools invoked during execution.
    #[serde(default)]
    pub tool_calls: Vec<ToolCallRecord>,
    /// Errors encountered during execution.
    #[serde(default)]
    pub error_messages: Vec<String>,
    /// Execution duration in milliseconds.
    pub duration_ms: u64,
    /// Structured token usage.
    pub token_usage: TokenUsage,
}

// ---------------------------------------------------------------------------
// Experience store
// ---------------------------------------------------------------------------

/// In-memory store for experience records.
#[derive(Debug, Default)]
pub struct ExperienceStore {
    records: Vec<ExperienceRecord>,
}

impl ExperienceStore {
    /// Create a new empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an experience record.
    pub fn add(&mut self, record: ExperienceRecord) {
        self.records.push(record);
    }

    /// Query records by skill ID.
    pub fn by_skill(&self, skill_id: &str) -> Vec<&ExperienceRecord> {
        self.records
            .iter()
            .filter(|r| r.skill_id.as_deref() == Some(skill_id))
            .collect()
    }

    /// Query records by outcome.
    pub fn by_outcome(&self, outcome: &ExperienceOutcome) -> Vec<&ExperienceRecord> {
        self.records
            .iter()
            .filter(|r| &r.outcome == outcome)
            .collect()
    }

    /// Query records by evidence provenance.
    pub fn with_provenance(&self, provenance: &EvidenceProvenance) -> Vec<&ExperienceRecord> {
        self.records
            .iter()
            .filter(|r| r.evidence.iter().any(|e| &e.provenance == provenance))
            .collect()
    }

    /// Total number of records.
    pub fn count(&self) -> usize {
        self.records.len()
    }

    /// Get all records (for batch processing).
    pub fn all(&self) -> &[ExperienceRecord] {
        &self.records
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_record(skill_id: Option<&str>, outcome: ExperienceOutcome) -> ExperienceRecord {
        ExperienceRecord {
            id: uuid::Uuid::new_v4().to_string(),
            timestamp: "2026-03-10T00:00:00Z".to_string(),
            skill_id: skill_id.map(String::from),
            skill_version: None,
            task_description: "test task".to_string(),
            outcome,
            trajectory_summary: "did things".to_string(),
            key_decisions: vec!["chose approach A".to_string()],
            evidence: vec![EvidenceEntry {
                content: "user said it was correct".to_string(),
                provenance: EvidenceProvenance::UserStated,
            }],
            tool_calls: vec![ToolCallRecord {
                name: "file_read".to_string(),
                success: true,
                duration_ms: 50,
            }],
            error_messages: vec![],
            duration_ms: 5000,
            token_usage: TokenUsage::new(1000, 500),
        }
    }

    /// T-SK-S6-02: ExperienceRecord with tool_calls and TokenUsage.
    #[test]
    fn test_experience_with_tool_calls() {
        let record = sample_record(Some("skill-1"), ExperienceOutcome::Success);
        assert_eq!(record.tool_calls.len(), 1);
        assert_eq!(record.tool_calls[0].name, "file_read");
        assert!(record.tool_calls[0].success);
        assert_eq!(record.token_usage.prompt, 1000);
        assert_eq!(record.token_usage.completion, 500);
        assert_eq!(record.token_usage.total, 1500);
    }

    /// T-P3-39-01: Add and retrieve experience records.
    #[test]
    fn test_experience_store_add_query() {
        let mut store = ExperienceStore::new();
        store.add(sample_record(Some("skill-1"), ExperienceOutcome::Success));
        store.add(sample_record(Some("skill-1"), ExperienceOutcome::Failure));
        store.add(sample_record(Some("skill-2"), ExperienceOutcome::Success));

        assert_eq!(store.count(), 3);
        assert_eq!(store.by_skill("skill-1").len(), 2);
        assert_eq!(store.by_skill("skill-2").len(), 1);
    }

    /// T-P3-39-02: Query by outcome filters correctly.
    #[test]
    fn test_experience_query_by_outcome() {
        let mut store = ExperienceStore::new();
        store.add(sample_record(None, ExperienceOutcome::Success));
        store.add(sample_record(None, ExperienceOutcome::Failure));
        store.add(sample_record(None, ExperienceOutcome::Partial));

        assert_eq!(store.by_outcome(&ExperienceOutcome::Success).len(), 1);
        assert_eq!(store.by_outcome(&ExperienceOutcome::Failure).len(), 1);
    }

    /// T-P3-39-03: Query by evidence provenance.
    #[test]
    fn test_experience_query_by_provenance() {
        let mut store = ExperienceStore::new();
        store.add(sample_record(None, ExperienceOutcome::Success));

        let user_stated = store.with_provenance(&EvidenceProvenance::UserStated);
        assert_eq!(user_stated.len(), 1);

        let corrections = store.with_provenance(&EvidenceProvenance::UserCorrection);
        assert!(corrections.is_empty());
    }
}
