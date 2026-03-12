//! Trace replay: reconstruct execution steps chronologically.

use uuid::Uuid;

use crate::trace_store::{TraceStore, TraceStoreError};
use crate::types::Observation;

/// Result of a replay: an ordered sequence of observations.
#[derive(Debug)]
pub struct ReplayResult {
    /// Observations ordered chronologically.
    pub steps: Vec<Observation>,
    /// Total number of observations.
    pub total_observations: usize,
}

/// Replay engine that reconstructs a trace's execution timeline.
pub struct TraceReplay<S> {
    store: S,
}

impl<S: TraceStore> TraceReplay<S> {
    pub fn new(store: S) -> Self {
        Self { store }
    }

    /// Replay a trace: return all observations in chronological order.
    pub async fn replay(&self, trace_id: Uuid) -> Result<ReplayResult, TraceStoreError> {
        // Verify trace exists.
        let _trace = self.store.get_trace(trace_id).await?;

        let mut observations = self.store.get_observations(trace_id).await?;
        observations.sort_by(|a, b| {
            a.started_at
                .cmp(&b.started_at)
                .then(a.sequence.cmp(&b.sequence))
        });

        let total = observations.len();

        Ok(ReplayResult {
            steps: observations,
            total_observations: total,
        })
    }

    /// Replay a subtree rooted at a specific observation.
    pub async fn replay_subtree(
        &self,
        trace_id: Uuid,
        root_obs_id: Uuid,
    ) -> Result<ReplayResult, TraceStoreError> {
        let all_obs = self.store.get_observations(trace_id).await?;

        // Collect the subtree by walking parent links.
        let mut subtree_ids = std::collections::HashSet::new();
        subtree_ids.insert(root_obs_id);

        // Fixed-point: keep adding children until stable.
        loop {
            let before = subtree_ids.len();
            for obs in &all_obs {
                if let Some(pid) = obs.parent_id {
                    if subtree_ids.contains(&pid) {
                        subtree_ids.insert(obs.id);
                    }
                }
            }
            if subtree_ids.len() == before {
                break;
            }
        }

        let mut steps: Vec<Observation> = all_obs
            .into_iter()
            .filter(|o| subtree_ids.contains(&o.id))
            .collect();
        steps.sort_by(|a, b| {
            a.started_at
                .cmp(&b.started_at)
                .then(a.sequence.cmp(&b.sequence))
        });
        let total = steps.len();

        Ok(ReplayResult {
            steps,
            total_observations: total,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace_store::InMemoryTraceStore;
    use crate::types::*;

    use std::sync::Arc;

    #[tokio::test]
    async fn test_replay_chronological_order() {
        let store = Arc::new(InMemoryTraceStore::new());
        let session = Uuid::new_v4();
        let trace = Trace::new(session, "replay-test");
        let trace_id = trace.id;
        store.insert_trace(trace).await.unwrap();

        // Insert observations in reverse order.
        let mut obs2 = Observation::new(trace_id, ObservationType::ToolCall, "tool-call");
        obs2.sequence = 2;
        store.insert_observation(obs2).await.unwrap();

        let mut obs1 = Observation::new(trace_id, ObservationType::Generation, "llm");
        obs1.sequence = 1;
        store.insert_observation(obs1).await.unwrap();

        let replay = TraceReplay::new(store);
        let result = replay.replay(trace_id).await.unwrap();

        assert_eq!(result.total_observations, 2);
        // Should be sorted by started_at then sequence.
        assert!(result.steps[0].started_at <= result.steps[1].started_at);
    }
}
