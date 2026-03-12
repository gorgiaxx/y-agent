//! Trace search: filter and sort traces.

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::trace_store::{TraceStore, TraceStoreError};
use crate::types::{Trace, TraceStatus};

/// Search query parameters.
#[derive(Debug, Default)]
pub struct TraceSearchQuery {
    /// Filter by status.
    pub status: Option<TraceStatus>,
    /// Filter by session.
    pub session_id: Option<Uuid>,
    /// Only traces started after this time.
    pub since: Option<DateTime<Utc>>,
    /// Only traces started before this time.
    pub before: Option<DateTime<Utc>>,
    /// Filter by tag (trace must contain all listed tags).
    pub tags: Vec<String>,
    /// Maximum results.
    pub limit: usize,
}

impl TraceSearchQuery {
    pub fn new() -> Self {
        Self {
            limit: 50,
            ..Default::default()
        }
    }

    #[must_use]
    pub fn with_status(mut self, status: TraceStatus) -> Self {
        self.status = Some(status);
        self
    }

    #[must_use]
    pub fn with_session(mut self, session_id: Uuid) -> Self {
        self.session_id = Some(session_id);
        self
    }

    #[must_use]
    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }

    #[must_use]
    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }
}

/// Search traces in the store.
pub struct TraceSearch<S> {
    store: S,
}

impl<S: TraceStore> TraceSearch<S> {
    pub fn new(store: S) -> Self {
        Self { store }
    }

    /// Execute a search query.
    pub async fn search(&self, query: &TraceSearchQuery) -> Result<Vec<Trace>, TraceStoreError> {
        let all = self
            .store
            .list_traces(query.status, query.since, query.limit * 2)
            .await?;

        let results: Vec<Trace> = all
            .into_iter()
            .filter(|t| query.session_id.is_none_or(|sid| t.session_id == sid))
            .filter(|t| query.before.is_none_or(|b| t.started_at < b))
            .filter(|t| {
                query
                    .tags
                    .iter()
                    .all(|tag| t.tags.iter().any(|tt| tt == tag))
            })
            .take(query.limit)
            .collect();

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace_store::InMemoryTraceStore;
    use crate::types::*;

    use std::sync::Arc;

    #[tokio::test]
    async fn test_search_by_tags_and_status() {
        let store = Arc::new(InMemoryTraceStore::new());
        let session = Uuid::new_v4();

        let mut t1 = Trace::new(session, "tagged-trace");
        t1.tags = vec!["important".into(), "production".into()];
        store.insert_trace(t1).await.unwrap();

        let mut t2 = Trace::new(session, "untagged-trace");
        t2.complete();
        store.insert_trace(t2).await.unwrap();

        let search = TraceSearch::new(store);

        // Search by tag.
        let query = TraceSearchQuery::new().with_tags(vec!["important".into()]);
        let results = search.search(&query).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "tagged-trace");

        // Search by status.
        let query = TraceSearchQuery::new().with_status(TraceStatus::Completed);
        let results = search.search(&query).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "untagged-trace");
    }
}
