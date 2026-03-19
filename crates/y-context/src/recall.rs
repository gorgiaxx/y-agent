//! Memory recall: retrieves relevant memories for context injection.
//!
//! Design reference: memory-architecture-design.md §Recall
//!
//! Supports multiple recall methods:
//! - **Text**: Full-text substring/word-overlap search
//! - **Vector**: Cosine similarity via embeddings (requires `EmbeddingProvider`)
//! - **Hybrid**: Weighted combination of text and vector scores
//! - **MMR**: Maximal Marginal Relevance for diversity in results

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
    /// MMR diversity lambda (0.0 = pure diversity, 1.0 = pure relevance).
    #[serde(default = "default_mmr_lambda")]
    pub mmr_lambda: f64,
    /// Whether to apply MMR diversity filtering.
    #[serde(default)]
    pub use_mmr: bool,
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
fn default_mmr_lambda() -> f64 {
    0.7
}

impl Default for RecallConfig {
    fn default() -> Self {
        Self {
            method: RecallMethod::default(),
            relevance_threshold: default_threshold(),
            max_results: default_max_results(),
            text_weight: default_text_weight(),
            vector_weight: default_vector_weight(),
            mmr_lambda: default_mmr_lambda(),
            use_mmr: false,
        }
    }
}

/// In-memory recall store with text, vector, and hybrid search support.
pub struct RecallStore {
    memories: Vec<StoredMemory>,
    config: RecallConfig,
}

/// A stored memory entry with optional embedding.
#[derive(Debug, Clone)]
pub struct StoredMemory {
    pub content: String,
    pub source_session: Option<String>,
    pub importance: f64,
    /// Pre-computed embedding vector (if available).
    pub embedding: Option<Vec<f32>>,
}

impl RecallStore {
    /// Create a new empty store.
    pub fn new(config: RecallConfig) -> Self {
        Self {
            memories: Vec::new(),
            config,
        }
    }

    /// Add a memory entry (without embedding).
    pub fn add(&mut self, content: &str, source_session: Option<String>, importance: f64) {
        self.memories.push(StoredMemory {
            content: content.to_string(),
            source_session,
            importance,
            embedding: None,
        });
    }

    /// Add a memory entry with a pre-computed embedding.
    pub fn add_with_embedding(
        &mut self,
        content: &str,
        source_session: Option<String>,
        importance: f64,
        embedding: Vec<f32>,
    ) {
        self.memories.push(StoredMemory {
            content: content.to_string(),
            source_session,
            importance,
            embedding: Some(embedding),
        });
    }

    /// Recall memories relevant to a query.
    ///
    /// Uses the configured recall method (text, vector, or hybrid).
    /// If `use_mmr` is enabled, applies Maximal Marginal Relevance filtering.
    pub fn recall(&self, query: &str) -> Vec<RecalledMemory> {
        self.recall_with_embedding(query, None)
    }

    /// Recall with an optional query embedding for vector search.
    pub fn recall_with_embedding(
        &self,
        query: &str,
        query_embedding: Option<&[f32]>,
    ) -> Vec<RecalledMemory> {
        let mut scored: Vec<(usize, f64)> = self
            .memories
            .iter()
            .enumerate()
            .filter_map(|(i, m)| {
                let score = match self.config.method {
                    RecallMethod::Text
                    | RecallMethod::TimeWeighted
                    | RecallMethod::ImportanceBased => self.text_score(query, m),
                    RecallMethod::Vector => {
                        if let (Some(qe), Some(me)) = (query_embedding, m.embedding.as_ref()) {
                            cosine_similarity(qe, me) * m.importance
                        } else {
                            self.text_score(query, m) // Fallback to text.
                        }
                    }
                    RecallMethod::Hybrid => {
                        let text = self.text_score(query, m);
                        let vector =
                            if let (Some(qe), Some(me)) = (query_embedding, m.embedding.as_ref()) {
                                cosine_similarity(qe, me) * m.importance
                            } else {
                                text // Fallback: use text score for vector component too.
                            };
                        self.config.text_weight * text + self.config.vector_weight * vector
                    }
                };

                if score >= self.config.relevance_threshold {
                    Some((i, score))
                } else {
                    None
                }
            })
            .collect();

        // Sort by score descending.
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        // Apply MMR if enabled and there are embeddings.
        let selected_indices = if self.config.use_mmr && query_embedding.is_some() {
            // SAFETY: we just checked is_some() above, but clippy prefers
            // avoiding .unwrap() after .is_some(). We keep it for readability
            // since `if let` would force restructuring the else branch.
            #[allow(clippy::unnecessary_unwrap)]
            self.mmr_select(&scored, query_embedding.unwrap(), self.config.max_results)
        } else {
            scored
                .iter()
                .take(self.config.max_results)
                .map(|(i, _)| *i)
                .collect()
        };

        selected_indices
            .into_iter()
            .filter_map(|i| {
                let m = &self.memories[i];
                let score = scored.iter().find(|(idx, _)| *idx == i)?.1;
                Some(RecalledMemory {
                    content: m.content.clone(),
                    relevance: score,
                    source_session: m.source_session.clone(),
                    method: self.config.method.clone(),
                })
            })
            .collect()
    }

    /// Compute text-based relevance score.
    fn text_score(&self, query: &str, memory: &StoredMemory) -> f64 {
        let query_lower = query.to_lowercase();
        let content_lower = memory.content.to_lowercase();

        // Exact substring match.
        if content_lower.contains(&query_lower) || query_lower.contains(&content_lower) {
            return 0.8 * memory.importance;
        }

        // Word overlap.
        let query_words: std::collections::HashSet<&str> = query_lower.split_whitespace().collect();
        let content_words: std::collections::HashSet<&str> =
            content_lower.split_whitespace().collect();
        let overlap = query_words.intersection(&content_words).count();

        if overlap > 0 {
            #[allow(clippy::cast_precision_loss)]
            let score = (overlap as f64 / query_words.len().max(1) as f64) * memory.importance;
            return score;
        }

        0.0
    }

    /// Maximal Marginal Relevance selection.
    ///
    /// Balances relevance with diversity by penalizing candidates that are
    /// too similar to already-selected results.
    fn mmr_select(
        &self,
        candidates: &[(usize, f64)],
        _query_embedding: &[f32],
        k: usize,
    ) -> Vec<usize> {
        if candidates.is_empty() {
            return Vec::new();
        }

        let mut selected: Vec<usize> = Vec::new();
        let mut remaining: Vec<(usize, f64)> = candidates.to_vec();

        // Greedily select k items.
        for _ in 0..k {
            if remaining.is_empty() {
                break;
            }

            let best_idx = remaining
                .iter()
                .enumerate()
                .map(|(ri, (mi, score))| {
                    // Compute max similarity to already-selected items.
                    let max_sim = selected
                        .iter()
                        .map(|si| {
                            let s_emb = self.memories[*si].embedding.as_deref();
                            let c_emb = self.memories[*mi].embedding.as_deref();
                            match (s_emb, c_emb) {
                                (Some(a), Some(b)) => cosine_similarity(a, b),
                                _ => 0.0,
                            }
                        })
                        .fold(0.0_f64, f64::max);

                    let mmr_score =
                        self.config.mmr_lambda * score - (1.0 - self.config.mmr_lambda) * max_sim;
                    (ri, mmr_score)
                })
                .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
                .map_or(0, |(ri, _)| ri);

            let (mem_idx, _) = remaining.remove(best_idx);
            selected.push(mem_idx);
        }

        selected
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

/// Cosine similarity between two vectors.
fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }

    let dot: f64 = a
        .iter()
        .zip(b.iter())
        .map(|(x, y)| f64::from(*x) * f64::from(*y))
        .sum();
    let norm_a: f64 = a
        .iter()
        .map(|x| f64::from(*x) * f64::from(*x))
        .sum::<f64>()
        .sqrt();
    let norm_b: f64 = b
        .iter()
        .map(|x| f64::from(*x) * f64::from(*x))
        .sum::<f64>()
        .sqrt();

    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }

    dot / (norm_a * norm_b)
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

    /// T-P5-01: Hybrid search combines text and vector scores.
    #[test]
    fn test_hybrid_search_with_embeddings() {
        let config = RecallConfig {
            method: RecallMethod::Hybrid,
            relevance_threshold: 0.0,
            text_weight: 0.4,
            vector_weight: 0.6,
            ..Default::default()
        };
        let mut store = RecallStore::new(config);

        // Memory with embedding similar to query.
        store.add_with_embedding(
            "fixing the authentication bug",
            None,
            1.0,
            vec![0.9, 0.1, 0.0],
        );
        // Memory with embedding dissimilar to query.
        store.add_with_embedding("unrelated shopping list", None, 1.0, vec![0.0, 0.0, 1.0]);

        let query_embedding = vec![1.0, 0.0, 0.0];
        let results = store.recall_with_embedding("fixing bug", Some(&query_embedding));

        // The first result should be the authentication bug memory
        // (higher text AND vector score).
        assert!(!results.is_empty());
        assert!(results[0].content.contains("authentication"));
    }

    /// T-P5-02: MMR reduces duplicate/similar results.
    #[test]
    fn test_mmr_diversity() {
        let config = RecallConfig {
            method: RecallMethod::Vector,
            relevance_threshold: 0.0,
            max_results: 2,
            mmr_lambda: 0.5,
            use_mmr: true,
            ..Default::default()
        };
        let mut store = RecallStore::new(config);

        // Two very similar memories.
        store.add_with_embedding("fix bug in auth", None, 1.0, vec![1.0, 0.0, 0.0]);
        store.add_with_embedding("fix bug in auth module", None, 1.0, vec![0.99, 0.01, 0.0]);
        // One diverse memory.
        store.add_with_embedding("deploy to production", None, 1.0, vec![0.0, 1.0, 0.0]);

        let query_embedding = vec![1.0, 0.0, 0.0];
        let results = store.recall_with_embedding("fix auth bug", Some(&query_embedding));

        assert_eq!(results.len(), 2);
        // MMR should prefer one auth + one diverse, not two near-identical auth memories.
        let has_auth = results.iter().any(|r| r.content.contains("auth"));
        let has_deploy = results.iter().any(|r| r.content.contains("deploy"));
        assert!(has_auth);
        assert!(has_deploy, "MMR should select diverse results");
    }

    /// T-P5-03: Threshold filtering works with real scores.
    #[test]
    fn test_threshold_filtering() {
        let config = RecallConfig {
            method: RecallMethod::Vector,
            relevance_threshold: 0.5,
            ..Default::default()
        };
        let mut store = RecallStore::new(config);

        store.add_with_embedding("relevant memory", None, 1.0, vec![1.0, 0.0]);
        store.add_with_embedding("irrelevant memory", None, 1.0, vec![0.0, 1.0]);

        let query_embedding = vec![1.0, 0.0];
        let results = store.recall_with_embedding("relevant", Some(&query_embedding));

        // Only the relevant memory should pass threshold.
        assert_eq!(results.len(), 1);
        assert!(results[0].content.contains("relevant memory"));
    }

    /// Cosine similarity correctness.
    #[test]
    fn test_cosine_similarity() {
        assert!((cosine_similarity(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 0.001);
        assert!((cosine_similarity(&[1.0, 0.0], &[0.0, 1.0])).abs() < 0.001);
        assert!(cosine_similarity(&[], &[]) == 0.0);
    }
}
