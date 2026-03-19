//! Two-phase deduplication: content hash fast path + LLM 4-action model.
//!
//! Phase 1: SHA-256 content hash — identical content deduped without LLM.
//! Phase 2: Similar but not identical → delegate to LLM 4-action model
//!          (merge, `keep_both`, replace, skip).
//!
//! In development, Phase 2 uses a simple similarity heuristic instead of LLM.

use std::collections::HashSet;

use sha2::{Digest, Sha256};

use y_core::memory::Memory;

/// Result of deduplication check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DedupAction {
    /// Store as new — no duplicate found.
    StoreNew,
    /// Exact duplicate found — skip storage.
    SkipDuplicate,
    /// Similar content found — merge into existing.
    Merge { existing_id: String },
    /// Similar but distinct — keep both.
    KeepBoth,
}

/// Two-phase deduplicator.
#[derive(Debug, Default)]
pub struct Deduplicator {
    /// Set of content hashes for fast Phase 1 dedup.
    content_hashes: HashSet<String>,
    /// Content summaries for Phase 2 similarity check (simplified).
    content_snippets: Vec<(String, String)>, // (memory_id, first_100_chars)
}

impl Deduplicator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if a memory is a duplicate and determine the action.
    pub fn check(&self, memory: &Memory) -> DedupAction {
        let hash = content_hash(&memory.content);

        // Phase 1: Exact content hash match
        if self.content_hashes.contains(&hash) {
            return DedupAction::SkipDuplicate;
        }

        // Phase 2: Similarity check (simplified — no LLM in dev mode)
        let snippet = memory
            .content
            .chars()
            .take(100)
            .collect::<String>()
            .to_lowercase();
        for (existing_id, existing_snippet) in &self.content_snippets {
            let similarity = jaccard_similarity(&snippet, existing_snippet);
            if similarity > 0.6 {
                return DedupAction::Merge {
                    existing_id: existing_id.clone(),
                };
            }
        }

        DedupAction::StoreNew
    }

    /// Register a memory's content hash and snippet (call after storing).
    pub fn register(&mut self, memory: &Memory) {
        let hash = content_hash(&memory.content);
        self.content_hashes.insert(hash);

        let snippet = memory
            .content
            .chars()
            .take(100)
            .collect::<String>()
            .to_lowercase();
        self.content_snippets.push((memory.id.to_string(), snippet));
    }
}

/// SHA-256 content hash.
fn content_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Simple Jaccard similarity on word sets.
fn jaccard_similarity(a: &str, b: &str) -> f32 {
    let set_a: HashSet<&str> = a.split_whitespace().collect();
    let set_b: HashSet<&str> = b.split_whitespace().collect();

    let intersection = set_a.intersection(&set_b).count();
    let union = set_a.union(&set_b).count();

    if union == 0 {
        return 0.0;
    }

    let sim = intersection as f32 / union as f32;
    sim
}

#[cfg(test)]
mod tests {
    use super::*;
    use y_core::memory::MemoryType;
    use y_core::types::{now, MemoryId};

    fn make_memory(content: &str) -> Memory {
        let ts = now();
        Memory {
            id: MemoryId::new(),
            memory_type: MemoryType::Task,
            scopes: vec![],
            when_to_use: String::new(),
            content: content.to_string(),
            importance: 0.5,
            access_count: 0,
            created_at: ts,
            updated_at: ts,
            metadata: serde_json::Value::Null,
        }
    }

    /// T-MEM-004-01: Identical content deduped without LLM call.
    #[test]
    fn test_dedup_content_hash_fast_path() {
        let mut dedup = Deduplicator::new();
        let mem1 = make_memory("exact same content here");
        dedup.register(&mem1);

        let mem2 = make_memory("exact same content here");
        assert_eq!(dedup.check(&mem2), DedupAction::SkipDuplicate);
    }

    /// T-MEM-004-02: Similar but not identical triggers Phase 2.
    #[test]
    fn test_dedup_similar_content_llm_check() {
        let mut dedup = Deduplicator::new();
        let mem1 = make_memory("the quick brown fox jumps over the lazy dog");
        dedup.register(&mem1);

        let mem2 = make_memory("the quick brown fox jumps over the lazy cat");
        let action = dedup.check(&mem2);
        assert!(matches!(action, DedupAction::Merge { .. }));
    }

    /// T-MEM-004-03: Merge action returns existing ID.
    #[test]
    fn test_dedup_llm_merge_action() {
        let mut dedup = Deduplicator::new();
        let mem1 = make_memory("rust error handling with thiserror and anyhow");
        let id1 = mem1.id.to_string();
        dedup.register(&mem1);

        let mem2 = make_memory("rust error handling using thiserror and anyhow libraries");
        let action = dedup.check(&mem2);
        assert_eq!(action, DedupAction::Merge { existing_id: id1 });
    }

    /// T-MEM-004-04: Dissimilar content → keep both (`StoreNew`).
    #[test]
    fn test_dedup_dissimilar_content_no_check() {
        let mut dedup = Deduplicator::new();
        let mem1 = make_memory("rust error handling patterns");
        dedup.register(&mem1);

        let mem2 = make_memory("python web framework comparison Django vs Flask");
        assert_eq!(dedup.check(&mem2), DedupAction::StoreNew);
    }

    /// T-MEM-004-05: Empty deduplicator always returns `StoreNew`.
    #[test]
    fn test_dedup_empty_always_store_new() {
        let dedup = Deduplicator::new();
        let mem = make_memory("brand new content");
        assert_eq!(dedup.check(&mem), DedupAction::StoreNew);
    }
}
