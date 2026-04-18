//! Trace storage trait and in-memory implementation.

use std::collections::HashMap;
use std::sync::RwLock;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::types::{Observation, Score, Trace, TraceStatus};

/// Errors from trace storage operations.
#[derive(Debug, thiserror::Error)]
pub enum TraceStoreError {
    #[error("trace not found: {id}")]
    TraceNotFound { id: Uuid },

    #[error("observation not found: {id}")]
    ObservationNotFound { id: Uuid },

    #[error("storage error: {message}")]
    Storage { message: String },
}

/// Storage backend for diagnostic traces.
#[async_trait]
pub trait TraceStore: Send + Sync {
    /// Insert a new trace.
    async fn insert_trace(&self, trace: Trace) -> Result<(), TraceStoreError>;

    /// Get a trace by ID.
    async fn get_trace(&self, id: Uuid) -> Result<Trace, TraceStoreError>;

    /// Update an existing trace.
    async fn update_trace(&self, trace: Trace) -> Result<(), TraceStoreError>;

    /// List traces with optional filters.
    async fn list_traces(
        &self,
        status: Option<TraceStatus>,
        since: Option<DateTime<Utc>>,
        limit: usize,
    ) -> Result<Vec<Trace>, TraceStoreError>;

    /// Insert an observation for a trace.
    async fn insert_observation(&self, obs: Observation) -> Result<(), TraceStoreError>;

    /// Update an existing observation (e.g. Running -> Completed after
    /// streaming finishes).
    async fn update_observation(&self, obs: Observation) -> Result<(), TraceStoreError>;

    /// Get all observations for a trace (flat list).
    async fn get_observations(&self, trace_id: Uuid) -> Result<Vec<Observation>, TraceStoreError>;

    /// Insert a score.
    async fn insert_score(&self, score: Score) -> Result<(), TraceStoreError>;

    /// Get scores for a trace.
    async fn get_scores(&self, trace_id: Uuid) -> Result<Vec<Score>, TraceStoreError>;

    /// Delete traces older than a given date (retention cleanup).
    async fn delete_before(&self, before: DateTime<Utc>) -> Result<u64, TraceStoreError>;

    /// List traces belonging to a specific session.
    async fn list_traces_by_session(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<Trace>, TraceStoreError>;

    /// Get observations for multiple traces in a single batch.
    ///
    /// Returns all observations belonging to any of the supplied trace IDs.
    /// The default implementation falls back to per-id calls; backends can
    /// override with a single efficient query (e.g. `WHERE trace_id IN (...)`).
    async fn get_observations_by_trace_ids(
        &self,
        trace_ids: &[Uuid],
    ) -> Result<Vec<Observation>, TraceStoreError> {
        let mut all = Vec::new();
        for id in trace_ids {
            let obs = self.get_observations(*id).await?;
            all.extend(obs);
        }
        Ok(all)
    }
}

/// Blanket impl so `Arc<T>` also implements `TraceStore`.
#[async_trait]
impl<T: TraceStore + ?Sized> TraceStore for std::sync::Arc<T> {
    async fn insert_trace(&self, trace: Trace) -> Result<(), TraceStoreError> {
        (**self).insert_trace(trace).await
    }
    async fn get_trace(&self, id: Uuid) -> Result<Trace, TraceStoreError> {
        (**self).get_trace(id).await
    }
    async fn update_trace(&self, trace: Trace) -> Result<(), TraceStoreError> {
        (**self).update_trace(trace).await
    }
    async fn list_traces(
        &self,
        status: Option<TraceStatus>,
        since: Option<DateTime<Utc>>,
        limit: usize,
    ) -> Result<Vec<Trace>, TraceStoreError> {
        (**self).list_traces(status, since, limit).await
    }
    async fn insert_observation(&self, obs: Observation) -> Result<(), TraceStoreError> {
        (**self).insert_observation(obs).await
    }
    async fn update_observation(&self, obs: Observation) -> Result<(), TraceStoreError> {
        (**self).update_observation(obs).await
    }
    async fn get_observations(&self, trace_id: Uuid) -> Result<Vec<Observation>, TraceStoreError> {
        (**self).get_observations(trace_id).await
    }
    async fn insert_score(&self, score: Score) -> Result<(), TraceStoreError> {
        (**self).insert_score(score).await
    }
    async fn get_scores(&self, trace_id: Uuid) -> Result<Vec<Score>, TraceStoreError> {
        (**self).get_scores(trace_id).await
    }
    async fn delete_before(&self, before: DateTime<Utc>) -> Result<u64, TraceStoreError> {
        (**self).delete_before(before).await
    }
    async fn list_traces_by_session(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<Trace>, TraceStoreError> {
        (**self).list_traces_by_session(session_id, limit).await
    }
    async fn get_observations_by_trace_ids(
        &self,
        trace_ids: &[Uuid],
    ) -> Result<Vec<Observation>, TraceStoreError> {
        (**self).get_observations_by_trace_ids(trace_ids).await
    }
}

/// In-memory trace store for testing (no `PostgreSQL` required).
#[derive(Debug, Default)]
pub struct InMemoryTraceStore {
    traces: RwLock<HashMap<Uuid, Trace>>,
    observations: RwLock<HashMap<Uuid, Vec<Observation>>>,
    scores: RwLock<HashMap<Uuid, Vec<Score>>>,
}

impl InMemoryTraceStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl TraceStore for InMemoryTraceStore {
    async fn insert_trace(&self, trace: Trace) -> Result<(), TraceStoreError> {
        self.traces
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(trace.id, trace);
        Ok(())
    }

    async fn get_trace(&self, id: Uuid) -> Result<Trace, TraceStoreError> {
        self.traces
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&id)
            .cloned()
            .ok_or(TraceStoreError::TraceNotFound { id })
    }

    async fn update_trace(&self, trace: Trace) -> Result<(), TraceStoreError> {
        let mut map = self.traces.write().unwrap_or_else(std::sync::PoisonError::into_inner);
        if !map.contains_key(&trace.id) {
            return Err(TraceStoreError::TraceNotFound { id: trace.id });
        }
        map.insert(trace.id, trace);
        Ok(())
    }

    async fn list_traces(
        &self,
        status: Option<TraceStatus>,
        since: Option<DateTime<Utc>>,
        limit: usize,
    ) -> Result<Vec<Trace>, TraceStoreError> {
        let map = self.traces.read().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut results: Vec<Trace> = map
            .values()
            .filter(|t| status.is_none_or(|s| t.status == s))
            .filter(|t| since.is_none_or(|s| t.started_at >= s))
            .cloned()
            .collect();
        results.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        results.truncate(limit);
        Ok(results)
    }

    async fn insert_observation(&self, obs: Observation) -> Result<(), TraceStoreError> {
        self.observations
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entry(obs.trace_id)
            .or_default()
            .push(obs);
        Ok(())
    }

    async fn update_observation(&self, obs: Observation) -> Result<(), TraceStoreError> {
        let mut map = self.observations.write().unwrap_or_else(std::sync::PoisonError::into_inner);
        let entries = map
            .get_mut(&obs.trace_id)
            .ok_or(TraceStoreError::ObservationNotFound { id: obs.id })?;
        if let Some(existing) = entries.iter_mut().find(|o| o.id == obs.id) {
            *existing = obs;
            Ok(())
        } else {
            Err(TraceStoreError::ObservationNotFound { id: obs.id })
        }
    }

    async fn get_observations(&self, trace_id: Uuid) -> Result<Vec<Observation>, TraceStoreError> {
        Ok(self
            .observations
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&trace_id)
            .cloned()
            .unwrap_or_default())
    }

    async fn insert_score(&self, score: Score) -> Result<(), TraceStoreError> {
        self.scores
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entry(score.trace_id)
            .or_default()
            .push(score);
        Ok(())
    }

    async fn get_scores(&self, trace_id: Uuid) -> Result<Vec<Score>, TraceStoreError> {
        Ok(self
            .scores
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&trace_id)
            .cloned()
            .unwrap_or_default())
    }

    async fn delete_before(&self, before: DateTime<Utc>) -> Result<u64, TraceStoreError> {
        let mut traces = self.traces.write().unwrap_or_else(std::sync::PoisonError::into_inner);
        let ids_to_remove: Vec<Uuid> = traces
            .values()
            .filter(|t| t.started_at < before)
            .map(|t| t.id)
            .collect();
        let count = ids_to_remove.len() as u64;

        let mut obs_map = self.observations.write().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut score_map = self.scores.write().unwrap_or_else(std::sync::PoisonError::into_inner);
        for id in &ids_to_remove {
            traces.remove(id);
            obs_map.remove(id);
            score_map.remove(id);
        }

        Ok(count)
    }

    async fn list_traces_by_session(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<Trace>, TraceStoreError> {
        let map = self.traces.read().unwrap_or_else(std::sync::PoisonError::into_inner);
        let target = Uuid::parse_str(session_id).unwrap_or_default();
        let mut results: Vec<Trace> = map
            .values()
            .filter(|t| t.session_id == target)
            .cloned()
            .collect();
        results.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        results.truncate(limit);
        Ok(results)
    }

    async fn get_observations_by_trace_ids(
        &self,
        trace_ids: &[Uuid],
    ) -> Result<Vec<Observation>, TraceStoreError> {
        let map = self.observations.read().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut all = Vec::new();
        for id in trace_ids {
            if let Some(obs) = map.get(id) {
                all.extend(obs.iter().cloned());
            }
        }
        Ok(all)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;

    #[tokio::test]
    async fn test_trace_crud() {
        let store = InMemoryTraceStore::new();
        let session = Uuid::new_v4();

        let mut trace = Trace::new(session, "test-trace");
        let trace_id = trace.id;

        // Insert.
        store.insert_trace(trace.clone()).await.unwrap();

        // Get.
        let loaded = store.get_trace(trace_id).await.unwrap();
        assert_eq!(loaded.name, "test-trace");
        assert_eq!(loaded.status, TraceStatus::Active);

        // Update.
        trace.complete();
        store.update_trace(trace).await.unwrap();
        let loaded = store.get_trace(trace_id).await.unwrap();
        assert_eq!(loaded.status, TraceStatus::Completed);

        // List filtered.
        let active = store
            .list_traces(Some(TraceStatus::Active), None, 10)
            .await
            .unwrap();
        assert!(active.is_empty());

        let completed = store
            .list_traces(Some(TraceStatus::Completed), None, 10)
            .await
            .unwrap();
        assert_eq!(completed.len(), 1);
    }

    #[tokio::test]
    async fn test_observation_hierarchy() {
        let store = InMemoryTraceStore::new();
        let session = Uuid::new_v4();
        let trace = Trace::new(session, "parent-trace");
        let trace_id = trace.id;
        store.insert_trace(trace).await.unwrap();

        // Root observation.
        let mut root = Observation::new(trace_id, ObservationType::Span, "root-span");
        root.sequence = 0;
        let root_id = root.id;

        // Child observation.
        let mut child = Observation::new(trace_id, ObservationType::Generation, "llm-call");
        child.parent_id = Some(root_id);
        child.sequence = 1;
        child.model = Some("gpt-4".into());
        child.input_tokens = 100;
        child.output_tokens = 50;
        child.cost_usd = 0.003;

        store.insert_observation(root).await.unwrap();
        store.insert_observation(child).await.unwrap();

        let obs = store.get_observations(trace_id).await.unwrap();
        assert_eq!(obs.len(), 2);

        let child_obs = obs.iter().find(|o| o.parent_id.is_some()).unwrap();
        assert_eq!(child_obs.parent_id, Some(root_id));
        assert_eq!(child_obs.model.as_deref(), Some("gpt-4"));
    }

    #[tokio::test]
    async fn test_retention_delete_before() {
        let store = InMemoryTraceStore::new();
        let session = Uuid::new_v4();

        // Insert two traces.
        let trace1 = Trace::new(session, "old-trace");
        let trace2 = Trace::new(session, "new-trace");
        store.insert_trace(trace1).await.unwrap();
        store.insert_trace(trace2).await.unwrap();

        // Delete everything before now + 1 second (should delete all).
        let cutoff = Utc::now() + chrono::Duration::seconds(1);
        let deleted = store.delete_before(cutoff).await.unwrap();
        assert_eq!(deleted, 2);

        let remaining = store.list_traces(None, None, 100).await.unwrap();
        assert!(remaining.is_empty());
    }
}
