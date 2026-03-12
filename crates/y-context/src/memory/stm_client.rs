//! STM client: session-scoped, in-memory experience store.
//!
//! Wraps an in-memory store implementing `ExperienceStore` for use
//! without a database dependency. Production code uses `SqliteExperienceStore`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};

use async_trait::async_trait;
use y_core::memory::{EvidenceType, ExperienceRecord, ExperienceStore, MemoryError};
use y_core::types::{now, SessionId, SkillId};

/// In-memory STM client for testing and development.
#[derive(Debug, Default)]
pub struct StmClient {
    /// Records keyed by (`session_id`, `slot_index`).
    records: HashMap<(String, u32), ExperienceRecord>,
    /// Next slot index per session.
    counters: HashMap<String, AtomicU32>,
}

impl StmClient {
    pub fn new() -> Self {
        Self::default()
    }

    fn next_slot(&mut self, session_id: &str) -> u32 {
        let counter = self
            .counters
            .entry(session_id.to_string())
            .or_insert_with(|| AtomicU32::new(0));
        counter.fetch_add(1, Ordering::Relaxed)
    }
}

#[async_trait]
impl ExperienceStore for StmClient {
    async fn compress(
        &self,
        _session_id: &SessionId,
        _summary: String,
        _evidence_type: EvidenceType,
        _skill_id: Option<SkillId>,
    ) -> Result<u32, MemoryError> {
        // Immutable async trait — defer to mutable helper
        Err(MemoryError::Other {
            message: "use compress_mut for mutable operations".to_string(),
        })
    }

    async fn read(
        &self,
        session_id: &SessionId,
        slot_index: u32,
    ) -> Result<ExperienceRecord, MemoryError> {
        let key = (session_id.to_string(), slot_index);
        self.records
            .get(&key)
            .cloned()
            .ok_or_else(|| MemoryError::NotFound {
                id: format!("{session_id}#{slot_index}"),
            })
    }

    async fn list(&self, session_id: &SessionId) -> Result<Vec<ExperienceRecord>, MemoryError> {
        let sid = session_id.to_string();
        let mut records: Vec<ExperienceRecord> = self
            .records
            .values()
            .filter(|r| r.session_id.to_string() == sid)
            .cloned()
            .collect();
        records.sort_by_key(|r| r.slot_index);
        Ok(records)
    }
}

impl StmClient {
    /// Mutable compress helper for testing.
    pub fn compress_mut(
        &mut self,
        session_id: &SessionId,
        summary: String,
        evidence_type: EvidenceType,
        skill_id: Option<SkillId>,
    ) -> u32 {
        let slot = self.next_slot(session_id.as_str());
        let token_estimate = u32::try_from(summary.len()).unwrap_or(u32::MAX).div_ceil(4);
        let record = ExperienceRecord {
            session_id: session_id.clone(),
            slot_index: slot,
            summary,
            evidence_type,
            skill_id,
            token_estimate,
            created_at: now(),
            metadata: serde_json::Value::Null,
        };
        self.records.insert((session_id.to_string(), slot), record);
        slot
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use y_core::types::SessionId;

    fn test_session() -> SessionId {
        SessionId::from_string("session-1")
    }

    /// T-MEM-002-01: `compress()` returns monotonic `slot_index`.
    #[test]
    fn test_experience_compress_assigns_slot() {
        let mut stm = StmClient::new();
        let s0 = stm.compress_mut(
            &test_session(),
            "step 1".into(),
            EvidenceType::TaskOutcome,
            None,
        );
        let s1 = stm.compress_mut(
            &test_session(),
            "step 2".into(),
            EvidenceType::TaskOutcome,
            None,
        );
        assert_eq!(s0, 0);
        assert_eq!(s1, 1);
    }

    /// T-MEM-002-02: `read(session, slot)` returns correct record.
    #[tokio::test]
    async fn test_experience_read_by_slot() {
        let mut stm = StmClient::new();
        let slot = stm.compress_mut(
            &test_session(),
            "found a bug".into(),
            EvidenceType::UserCorrection,
            None,
        );

        let record = stm.read(&test_session(), slot).await.unwrap();
        assert_eq!(record.summary, "found a bug");
        assert_eq!(record.evidence_type, EvidenceType::UserCorrection);
    }

    /// T-MEM-002-03: `list(session)` returns all session experiences.
    #[tokio::test]
    async fn test_experience_list_session() {
        let mut stm = StmClient::new();
        stm.compress_mut(
            &test_session(),
            "exp 1".into(),
            EvidenceType::TaskOutcome,
            None,
        );
        stm.compress_mut(
            &test_session(),
            "exp 2".into(),
            EvidenceType::TaskOutcome,
            None,
        );

        let records = stm.list(&test_session()).await.unwrap();
        assert_eq!(records.len(), 2);
    }

    /// T-MEM-002-04: Evidence type is preserved.
    #[test]
    fn test_experience_evidence_type_stored() {
        let mut stm = StmClient::new();
        stm.compress_mut(
            &test_session(),
            "obs".into(),
            EvidenceType::AgentObservation,
            None,
        );

        let key = (test_session().to_string(), 0);
        let record = stm.records.get(&key).unwrap();
        assert_eq!(record.evidence_type, EvidenceType::AgentObservation);
    }

    /// T-MEM-002-05: Token estimate is stored.
    #[test]
    fn test_experience_token_estimate_stored() {
        let mut stm = StmClient::new();
        stm.compress_mut(
            &test_session(),
            "a summary with some words".into(),
            EvidenceType::UserStated,
            None,
        );

        let key = (test_session().to_string(), 0);
        let record = stm.records.get(&key).unwrap();
        assert!(record.token_estimate > 0);
    }

    /// T-MEM-002-06: Cross-session isolation.
    #[tokio::test]
    async fn test_experience_cross_session_isolation() {
        let mut stm = StmClient::new();
        let s1 = SessionId::from_string("session-A");
        let s2 = SessionId::from_string("session-B");

        stm.compress_mut(&s1, "exp A".into(), EvidenceType::TaskOutcome, None);
        stm.compress_mut(&s2, "exp B".into(), EvidenceType::TaskOutcome, None);

        let list_a = stm.list(&s1).await.unwrap();
        let list_b = stm.list(&s2).await.unwrap();

        assert_eq!(list_a.len(), 1);
        assert_eq!(list_b.len(), 1);
        assert_eq!(list_a[0].summary, "exp A");
        assert_eq!(list_b[0].summary, "exp B");
    }
}
