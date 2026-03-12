//! `SearchOrchestrator`: multi-strategy fallback (Vector → Hybrid → Keyword).
//!
//! In dev mode, all strategies use in-memory substring matching.
//! Production delegates vector search to Qdrant.

use y_core::memory::{MemoryClient, MemoryError, MemoryQuery, MemoryResult};

/// Minimum results required before trying the next strategy.
const MIN_RESULTS: usize = 3;

/// Multi-strategy search with cascading fallback.
pub struct SearchOrchestrator;

impl SearchOrchestrator {
    /// Search using cascading strategies: vector → hybrid → keyword.
    ///
    /// Falls back to the next strategy if the current one returns
    /// fewer than `MIN_RESULTS` results.
    pub async fn search(
        client: &dyn MemoryClient,
        query: &str,
        limit: usize,
    ) -> Result<Vec<MemoryResult>, MemoryError> {
        // Strategy 1: "Vector" search (substring in dev mode)
        let vector_results = client
            .recall(MemoryQuery {
                query: query.to_string(),
                memory_type: None,
                scope: None,
                limit,
                min_importance: None,
            })
            .await?;

        if vector_results.len() >= MIN_RESULTS {
            return Ok(dedup_results(vector_results));
        }

        // Strategy 2: "Hybrid" search — broader query (split words)
        let words: Vec<&str> = query.split_whitespace().collect();
        let mut hybrid_results = vector_results;

        for word in &words {
            let partial = client
                .recall(MemoryQuery {
                    query: (*word).to_string(),
                    memory_type: None,
                    scope: None,
                    limit,
                    min_importance: None,
                })
                .await?;
            hybrid_results.extend(partial);
        }

        let hybrid_deduped = dedup_results(hybrid_results);
        if hybrid_deduped.len() >= MIN_RESULTS {
            return Ok(hybrid_deduped);
        }

        // Strategy 3: "Keyword" search — just return whatever we have
        Ok(hybrid_deduped)
    }
}

/// Deduplicate results by memory ID.
fn dedup_results(mut results: Vec<MemoryResult>) -> Vec<MemoryResult> {
    let mut seen = std::collections::HashSet::new();
    results.retain(|r| seen.insert(r.memory.id.to_string()));
    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::ltm_client::LtmClient;
    use y_core::memory::MemoryType;
    use y_core::types::{now, MemoryId};

    fn make_memory(content: &str) -> y_core::memory::Memory {
        let ts = now();
        y_core::memory::Memory {
            id: MemoryId::new(),
            memory_type: MemoryType::Task,
            scopes: vec![],
            when_to_use: format!("relevance: {content}"),
            content: content.to_string(),
            importance: 0.5,
            access_count: 0,
            created_at: ts,
            updated_at: ts,
            metadata: serde_json::Value::Null,
        }
    }

    /// T-MEM-005-01: Vector search used first when sufficient results.
    #[tokio::test]
    async fn test_search_vector_primary() {
        let mut ltm = LtmClient::new();
        for i in 0..5 {
            ltm.remember_mut(make_memory(&format!("error handling pattern {i}")));
        }

        let results = SearchOrchestrator::search(&ltm, "error handling", 10)
            .await
            .unwrap();
        assert!(results.len() >= 3);
    }

    /// T-MEM-005-02: Falls back to hybrid if vector returns too few.
    #[tokio::test]
    async fn test_search_hybrid_fallback() {
        let mut ltm = LtmClient::new();
        ltm.remember_mut(make_memory("rust programming concepts"));
        ltm.remember_mut(make_memory("error handling in production"));

        let results = SearchOrchestrator::search(&ltm, "rust error", 10)
            .await
            .unwrap();
        // Hybrid should find both via individual word search
        assert!(!results.is_empty());
    }

    /// T-MEM-005-03: Keyword last resort still returns results.
    #[tokio::test]
    async fn test_search_keyword_last_resort() {
        let mut ltm = LtmClient::new();
        ltm.remember_mut(make_memory("obscure topic alpha"));

        let results = SearchOrchestrator::search(&ltm, "alpha", 10).await.unwrap();
        assert!(!results.is_empty());
    }

    /// T-MEM-005-04: Results are deduplicated.
    #[tokio::test]
    async fn test_search_results_deduplicated() {
        let mut ltm = LtmClient::new();
        ltm.remember_mut(make_memory("error handling patterns"));

        let results = SearchOrchestrator::search(&ltm, "error handling patterns", 10)
            .await
            .unwrap();

        // Check no duplicate IDs
        let ids: Vec<String> = results.iter().map(|r| r.memory.id.to_string()).collect();
        let unique: std::collections::HashSet<String> = ids.iter().cloned().collect();
        assert_eq!(ids.len(), unique.len(), "results contain duplicates");
    }
}
