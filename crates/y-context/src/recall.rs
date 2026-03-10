//! Memory recall: retrieves relevant memories for context injection.

use serde::{Deserialize, Serialize};

/// Recall method for memory search.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RecallMethod {
    /// Full-text search.
    Text,
    /// Vector similarity search.
    Vector,
    /// Weighted combination of text and vector.
    #[default]
    Hybrid,
    /// Decay factor for older memories.
    TimeWeighted,
    /// Score multiplied by stored importance.
    ImportanceBased,
}

/// A recalled memory item.
#[derive(Debug, Clone)]
pub struct RecalledMemory {
    /// Memory content.
    pub content: String,
    /// Relevance score (0.0 - 1.0).
    pub relevance: f64,
    /// Source session ID.
    pub source_session: Option<String>,
    /// Recall method used.
    pub method: RecallMethod,
}

/// Configuration for memory recall.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecallConfig {
    /// Recall method.
    #[serde(default)]
    pub method: RecallMethod,
    /// Minimum relevance threshold (default 0.6).
    #[serde(default = "default_threshold")]
    pub relevance_threshold: f64,
    /// Maximum results to return.
    #[serde(default = "default_max_results")]
    pub max_results: usize,
    /// Weights for hybrid search (`text_weight` + `vector_weight` = 1.0).
    #[serde(default = "default_text_weight")]
    pub text_weight: f64,
    #[serde(default = "default_vector_weight")]
    pub vector_weight: f64,
}

fn default_threshold() -> f64 {
    0.6
}
fn default_max_results() -> usize {
    5
}
fn default_text_weight() -> f64 {
    0.4
}
fn default_vector_weight() -> f64 {
    0.6
}

impl Default for RecallConfig {
    fn default() -> Self {
        Self {
            method: RecallMethod::default(),
            relevance_threshold: default_threshold(),
            max_results: default_max_results(),
            text_weight: default_text_weight(),
            vector_weight: default_vector_weight(),
        }
    }
}

/// In-memory recall store (placeholder — real implementation uses vector DB).
pub struct RecallStore {
    memories: Vec<StoredMemory>,
    config: RecallConfig,
}

/// A stored memory entry.
#[derive(Debug, Clone)]
pub struct StoredMemory {
    pub content: String,
    pub source_session: Option<String>,
    pub importance: f64,
}

impl RecallStore {
    /// Create a new empty store.
    pub fn new(config: RecallConfig) -> Self {
        Self {
            memories: Vec::new(),
            config,
        }
    }

    /// Add a memory entry.
    pub fn add(&mut self, content: &str, source_session: Option<String>, importance: f64) {
        self.memories.push(StoredMemory {
            content: content.to_string(),
            source_session,
            importance,
        });
    }

    /// Recall memories relevant to a query.
    ///
    /// Placeholder: uses simple substring matching instead of vector search.
    pub fn recall(&self, query: &str) -> Vec<RecalledMemory> {
        let query_lower = query.to_lowercase();
        let mut results: Vec<RecalledMemory> = self
            .memories
            .iter()
            .filter_map(|m| {
                let content_lower = m.content.to_lowercase();
                // Simple relevance: substring match gives score based on overlap.
                if content_lower.contains(&query_lower) || query_lower.contains(&content_lower) {
                    Some(RecalledMemory {
                        content: m.content.clone(),
                        relevance: 0.8 * m.importance,
                        source_session: m.source_session.clone(),
                        method: self.config.method.clone(),
                    })
                } else {
                    // Check word overlap.
                    let query_words: std::collections::HashSet<&str> =
                        query_lower.split_whitespace().collect();
                    let content_words: std::collections::HashSet<&str> =
                        content_lower.split_whitespace().collect();
                    let overlap = query_words.intersection(&content_words).count();
                    if overlap > 0 {
                        #[allow(clippy::cast_precision_loss)]
                        let relevance =
                            (overlap as f64 / query_words.len().max(1) as f64) * m.importance;
                        if relevance >= self.config.relevance_threshold {
                            return Some(RecalledMemory {
                                content: m.content.clone(),
                                relevance,
                                source_session: m.source_session.clone(),
                                method: self.config.method.clone(),
                            });
                        }
                    }
                    None
                }
            })
            .collect();

        // Sort by relevance descending, take max_results.
        results.sort_by(|a, b| {
            b.relevance
                .partial_cmp(&a.relevance)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(self.config.max_results);
        results
    }

    /// Number of stored memories.
    pub fn len(&self) -> usize {
        self.memories.len()
    }

    /// Whether the store is empty.
    pub fn is_empty(&self) -> bool {
        self.memories.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_recall_substring_match() {
        let mut store = RecallStore::new(RecallConfig::default());
        store.add("The API key is xyz123", None, 1.0);
        store.add("Meeting notes from Monday", None, 0.8);

        let results = store.recall("API key");
        assert_eq!(results.len(), 1);
        assert!(results[0].content.contains("xyz123"));
    }

    #[test]
    fn test_recall_word_overlap() {
        let mut store = RecallStore::new(RecallConfig {
            relevance_threshold: 0.3,
            ..Default::default()
        });
        store.add("important meeting about deployment", None, 1.0);
        store.add("cat pictures collection", None, 1.0);

        let results = store.recall("deployment plan");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_recall_empty() {
        let store = RecallStore::new(RecallConfig::default());
        let results = store.recall("anything");
        assert!(results.is_empty());
    }

    #[test]
    fn test_recall_max_results() {
        let mut store = RecallStore::new(RecallConfig {
            max_results: 2,
            relevance_threshold: 0.0,
            ..Default::default()
        });
        for i in 0..10 {
            store.add(&format!("memory about topic {i}"), None, 1.0);
        }
        let results = store.recall("topic");
        assert!(results.len() <= 2);
    }
}
