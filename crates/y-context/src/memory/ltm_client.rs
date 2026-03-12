//! LTM client: in-memory `MemoryClient` for development/testing.
//!
//! Production use would delegate to Qdrant (feature: `memory_ltm`),
//! but this implementation uses an in-memory `HashMap` for testing.

use std::collections::HashMap;

use async_trait::async_trait;
use y_core::memory::{Memory, MemoryClient, MemoryError, MemoryQuery, MemoryResult};
use y_core::types::MemoryId;

/// In-memory LTM client for development and testing.
#[derive(Debug, Default)]
pub struct LtmClient {
    memories: HashMap<String, Memory>,
}

impl LtmClient {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl MemoryClient for LtmClient {
    async fn remember(&self, _memory: Memory) -> Result<MemoryId, MemoryError> {
        // Immutable async trait — real impl needs interior mutability
        Err(MemoryError::Other {
            message: "use remember_mut for mutable operations".to_string(),
        })
    }

    async fn recall(&self, query: MemoryQuery) -> Result<Vec<MemoryResult>, MemoryError> {
        let query_lower = query.query.to_lowercase();

        let mut results: Vec<MemoryResult> = self
            .memories
            .values()
            .filter(|m| {
                // Type filter
                if let Some(ref mt) = query.memory_type {
                    if m.memory_type != *mt {
                        return false;
                    }
                }
                // Importance filter
                if let Some(min) = query.min_importance {
                    if m.importance < min {
                        return false;
                    }
                }
                true
            })
            .filter_map(|m| {
                // Simple substring relevance
                let content_lower = m.content.to_lowercase();
                let when_lower = m.when_to_use.to_lowercase();

                let relevance = if content_lower.contains(&query_lower) {
                    0.9
                } else if when_lower.contains(&query_lower) {
                    0.8
                } else {
                    // Word overlap fallback
                    let words: Vec<&str> = query_lower.split_whitespace().collect();
                    let matches = words
                        .iter()
                        .filter(|w| content_lower.contains(*w) || when_lower.contains(*w))
                        .count();
                    if matches > 0 {
                        #[allow(clippy::cast_precision_loss)]
                        let rel = matches as f32 / words.len().max(1) as f32 * 0.6;
                        rel
                    } else {
                        return None;
                    }
                };

                Some(MemoryResult {
                    memory: m.clone(),
                    relevance,
                })
            })
            .collect();

        results.sort_by(|a, b| {
            b.relevance
                .partial_cmp(&a.relevance)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let limit = if query.limit == 0 { 10 } else { query.limit };
        results.truncate(limit);

        Ok(results)
    }

    async fn forget(&self, _id: &MemoryId) -> Result<(), MemoryError> {
        Err(MemoryError::Other {
            message: "use forget_mut for mutable operations".to_string(),
        })
    }

    async fn get(&self, id: &MemoryId) -> Result<Memory, MemoryError> {
        self.memories
            .get(id.as_str())
            .cloned()
            .ok_or_else(|| MemoryError::NotFound { id: id.to_string() })
    }
}

impl LtmClient {
    /// Store a memory (mutable helper for testing).
    pub fn remember_mut(&mut self, memory: Memory) -> MemoryId {
        let id = memory.id.clone();
        self.memories.insert(id.to_string(), memory);
        id
    }

    /// Delete a memory (mutable helper for testing).
    pub fn forget_mut(&mut self, id: &MemoryId) -> Result<(), MemoryError> {
        self.memories
            .remove(id.as_str())
            .map(|_| ())
            .ok_or_else(|| MemoryError::NotFound { id: id.to_string() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use y_core::memory::MemoryType;
    use y_core::types::{now, MemoryId};

    fn test_memory(content: &str, mem_type: MemoryType, importance: f32) -> Memory {
        let ts = now();
        Memory {
            id: MemoryId::new(),
            memory_type: mem_type,
            scopes: vec!["workspace".to_string()],
            when_to_use: format!("When relevant to: {content}"),
            content: content.to_string(),
            importance,
            access_count: 0,
            created_at: ts,
            updated_at: ts,
            metadata: serde_json::Value::Null,
        }
    }

    /// T-MEM-003-01: Store and recall by query.
    #[tokio::test]
    async fn test_ltm_remember_and_recall() {
        let mut ltm = LtmClient::new();
        ltm.remember_mut(test_memory(
            "Rust error handling patterns",
            MemoryType::Tool,
            0.8,
        ));

        let results = ltm
            .recall(MemoryQuery {
                query: "error handling".to_string(),
                memory_type: None,
                scope: None,
                limit: 10,
                min_importance: None,
            })
            .await
            .unwrap();

        assert_eq!(results.len(), 1);
        assert!(results[0].relevance > 0.5);
    }

    /// T-MEM-003-02: Limit is respected.
    #[tokio::test]
    async fn test_ltm_recall_respects_limit() {
        let mut ltm = LtmClient::new();
        for i in 0..10 {
            ltm.remember_mut(test_memory(
                &format!("error pattern {i}"),
                MemoryType::Tool,
                0.5,
            ));
        }

        let results = ltm
            .recall(MemoryQuery {
                query: "error".to_string(),
                memory_type: None,
                scope: None,
                limit: 3,
                min_importance: None,
            })
            .await
            .unwrap();

        assert!(results.len() <= 3);
    }

    /// T-MEM-003-03: Filter by memory type.
    #[tokio::test]
    async fn test_ltm_recall_filter_by_type() {
        let mut ltm = LtmClient::new();
        ltm.remember_mut(test_memory("tool tip", MemoryType::Tool, 0.8));
        ltm.remember_mut(test_memory("personal pref", MemoryType::Personal, 0.8));

        let results = ltm
            .recall(MemoryQuery {
                query: "tip".to_string(),
                memory_type: Some(MemoryType::Tool),
                scope: None,
                limit: 10,
                min_importance: None,
            })
            .await
            .unwrap();

        assert!(results
            .iter()
            .all(|r| r.memory.memory_type == MemoryType::Tool));
    }

    /// T-MEM-003-04: Min importance filter.
    #[tokio::test]
    async fn test_ltm_recall_min_importance() {
        let mut ltm = LtmClient::new();
        ltm.remember_mut(test_memory("high importance", MemoryType::Task, 0.9));
        ltm.remember_mut(test_memory("low importance", MemoryType::Task, 0.2));

        let results = ltm
            .recall(MemoryQuery {
                query: "importance".to_string(),
                memory_type: None,
                scope: None,
                limit: 10,
                min_importance: Some(0.5),
            })
            .await
            .unwrap();

        assert!(results.iter().all(|r| r.memory.importance >= 0.5));
    }

    /// T-MEM-003-05: Forget removes memory.
    #[tokio::test]
    async fn test_ltm_forget() {
        let mut ltm = LtmClient::new();
        let mem = test_memory("to forget", MemoryType::Task, 0.5);
        let id = mem.id.clone();
        ltm.remember_mut(mem);

        ltm.forget_mut(&id).unwrap();
        assert!(ltm.get(&id).await.is_err());
    }

    /// T-MEM-003-06: Get by ID returns exact memory.
    #[tokio::test]
    async fn test_ltm_get_by_id() {
        let mut ltm = LtmClient::new();
        let mem = test_memory("specific memory", MemoryType::Personal, 0.7);
        let id = mem.id.clone();
        ltm.remember_mut(mem);

        let retrieved = ltm.get(&id).await.unwrap();
        assert_eq!(retrieved.content, "specific memory");
    }
}
