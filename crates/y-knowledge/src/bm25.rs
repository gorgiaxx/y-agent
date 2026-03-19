//! BM25 keyword search engine.
//!
//! Implements an in-memory inverted index with Okapi BM25 scoring
//! for keyword-based retrieval.
//!
//! Parameters: `k1=1.2`, `b=0.75` (standard BM25 defaults).

use crate::tokenizer::Tokenizer;
use std::collections::HashMap;

/// BM25 scoring parameters.
const K1: f64 = 1.2;
const B: f64 = 0.75;

/// A posting in the inverted index.
#[derive(Debug, Clone)]
struct Posting {
    /// Chunk ID.
    chunk_id: String,
    /// Term frequency in this chunk.
    tf: u32,
}

/// In-memory inverted index with BM25 scoring.
///
/// Supports language-aware tokenization via the [`Tokenizer`] trait,
/// enabling both English and Chinese keyword search.
#[derive(Debug)]
pub struct Bm25Index<T: Tokenizer> {
    tokenizer: T,
    /// term → list of postings.
    index: HashMap<String, Vec<Posting>>,
    /// `chunk_id` → total token count in the chunk.
    doc_lengths: HashMap<String, u32>,
    /// Total number of indexed chunks.
    doc_count: u32,
    /// Sum of all document lengths (for avg calculation).
    total_length: u64,
}

/// A BM25 search result.
#[derive(Debug, Clone)]
pub struct Bm25Result {
    /// Chunk ID.
    pub chunk_id: String,
    /// BM25 score.
    pub score: f64,
}

impl<T: Tokenizer> Bm25Index<T> {
    /// Create a new BM25 index with the given tokenizer.
    pub fn new(tokenizer: T) -> Self {
        Self {
            tokenizer,
            index: HashMap::new(),
            doc_lengths: HashMap::new(),
            doc_count: 0,
            total_length: 0,
        }
    }

    /// Index a chunk of text.
    pub fn add(&mut self, chunk_id: &str, content: &str) {
        let tokens = self.tokenizer.tokenize(content);
        let doc_len = u32::try_from(tokens.len()).unwrap_or(u32::MAX);

        self.doc_lengths.insert(chunk_id.to_string(), doc_len);
        self.doc_count += 1;
        self.total_length += u64::from(doc_len);

        // Count term frequencies.
        let mut term_freqs: HashMap<String, u32> = HashMap::new();
        for token in tokens {
            *term_freqs.entry(token).or_insert(0) += 1;
        }

        // Update inverted index.
        for (term, tf) in term_freqs {
            self.index.entry(term).or_default().push(Posting {
                chunk_id: chunk_id.to_string(),
                tf,
            });
        }
    }

    /// Index multiple chunks in one call.
    ///
    /// Functionally equivalent to calling [`add`] for each item, but
    /// pre-reserves capacity in the document-lengths map to reduce
    /// allocator pressure when indexing thousands of chunks at once.
    pub fn add_bulk(&mut self, documents: &[(&str, &str)]) {
        self.doc_lengths.reserve(documents.len());

        for &(chunk_id, content) in documents {
            let tokens = self.tokenizer.tokenize(content);
            let doc_len = u32::try_from(tokens.len()).unwrap_or(u32::MAX);

            self.doc_lengths.insert(chunk_id.to_string(), doc_len);
            self.doc_count += 1;
            self.total_length += u64::from(doc_len);

            let mut term_freqs: HashMap<String, u32> = HashMap::new();
            for token in tokens {
                *term_freqs.entry(token).or_insert(0) += 1;
            }

            for (term, tf) in term_freqs {
                self.index.entry(term).or_default().push(Posting {
                    chunk_id: chunk_id.to_string(),
                    tf,
                });
            }
        }
    }

    /// Remove a chunk from the index.
    pub fn remove(&mut self, chunk_id: &str) {
        if let Some(doc_len) = self.doc_lengths.remove(chunk_id) {
            self.doc_count = self.doc_count.saturating_sub(1);
            self.total_length = self.total_length.saturating_sub(u64::from(doc_len));
        }

        // Remove postings for this chunk.
        for postings in self.index.values_mut() {
            postings.retain(|p| p.chunk_id != chunk_id);
        }

        // Clean up empty terms.
        self.index.retain(|_, postings| !postings.is_empty());
    }

    /// Remove all chunks matching the given set of IDs in a single pass.
    ///
    /// Much faster than calling `remove()` in a loop for large batches,
    /// since the inverted index is scanned only once (O(terms × postings))
    /// instead of once per chunk (O(chunks × terms × postings)).
    pub fn remove_bulk(&mut self, chunk_ids: &std::collections::HashSet<String>) {
        if chunk_ids.is_empty() {
            return;
        }

        // Update doc metadata.
        for id in chunk_ids {
            if let Some(doc_len) = self.doc_lengths.remove(id) {
                self.doc_count = self.doc_count.saturating_sub(1);
                self.total_length = self.total_length.saturating_sub(u64::from(doc_len));
            }
        }

        // Single-pass removal from inverted index.
        for postings in self.index.values_mut() {
            postings.retain(|p| !chunk_ids.contains(&p.chunk_id));
        }

        // Clean up empty terms.
        self.index.retain(|_, postings| !postings.is_empty());
    }

    /// Search the index and return scored results.
    pub fn search(&self, query: &str, top_k: usize) -> Vec<Bm25Result> {
        let query_tokens = self.tokenizer.tokenize(query);
        if query_tokens.is_empty() || self.doc_count == 0 {
            return vec![];
        }

        let avgdl = self.total_length as f64 / f64::from(self.doc_count);

        // Accumulate BM25 scores per chunk.
        let mut scores: HashMap<&str, f64> = HashMap::new();

        for term in &query_tokens {
            if let Some(postings) = self.index.get(term) {
                // IDF: log((N - n + 0.5) / (n + 0.5) + 1)
                let n = postings.len() as f64;
                let idf = ((f64::from(self.doc_count) - n + 0.5) / (n + 0.5) + 1.0).ln();

                for posting in postings {
                    let dl = f64::from(*self.doc_lengths.get(&posting.chunk_id).unwrap_or(&1));
                    let tf = f64::from(posting.tf);

                    // BM25 TF component.
                    let tf_norm = (tf * (K1 + 1.0)) / (tf + K1 * (1.0 - B + B * dl / avgdl));

                    *scores.entry(&posting.chunk_id).or_insert(0.0) += idf * tf_norm;
                }
            }
        }

        // Sort by score descending.
        let mut results: Vec<Bm25Result> = scores
            .into_iter()
            .map(|(chunk_id, score)| Bm25Result {
                chunk_id: chunk_id.to_string(),
                score,
            })
            .collect();

        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(top_k);
        results
    }

    /// Get the number of indexed chunks.
    pub fn len(&self) -> usize {
        self.doc_count as usize
    }

    /// Check if the index is empty.
    pub fn is_empty(&self) -> bool {
        self.doc_count == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokenizer::SimpleTokenizer;

    fn make_index() -> Bm25Index<SimpleTokenizer> {
        let mut index = Bm25Index::new(SimpleTokenizer::new());
        index.add("c1", "Rust error handling patterns and best practices");
        index.add("c2", "Python web frameworks Django and Flask");
        index.add("c3", "Rust async runtime with tokio and error recovery");
        index.add("c4", "JavaScript React state management patterns");
        index
    }

    #[test]
    fn test_bm25_basic_search() {
        let index = make_index();
        let results = index.search("Rust error", 10);
        assert!(!results.is_empty());
        // c1 and c3 both contain "rust" and "error".
        assert!(results.len() >= 2);
        // First result should be c1 or c3 (both have relevant terms).
        assert!(results[0].chunk_id == "c1" || results[0].chunk_id == "c3");
    }

    #[test]
    fn test_bm25_top_k_limit() {
        let index = make_index();
        let results = index.search("patterns", 2);
        assert!(results.len() <= 2);
    }

    #[test]
    fn test_bm25_no_match() {
        let index = make_index();
        let results = index.search("quantum physics", 10);
        assert!(results.is_empty());
    }

    #[test]
    fn test_bm25_empty_query() {
        let index = make_index();
        let results = index.search("", 10);
        assert!(results.is_empty());
    }

    #[test]
    fn test_bm25_scores_positive() {
        let index = make_index();
        let results = index.search("Rust", 10);
        for r in &results {
            assert!(r.score > 0.0, "BM25 scores should be positive");
        }
    }

    #[test]
    fn test_bm25_remove_document() {
        let mut index = make_index();
        assert_eq!(index.len(), 4);

        index.remove("c1");
        assert_eq!(index.len(), 3);

        let results = index.search("error handling patterns", 10);
        assert!(
            results.iter().all(|r| r.chunk_id != "c1"),
            "removed chunk should not appear in results"
        );
    }

    #[test]
    fn test_bm25_term_frequency_matters() {
        let mut index = Bm25Index::new(SimpleTokenizer::new());
        index.add("frequent", "rust rust rust programming");
        index.add("rare", "rust programming basics");

        let results = index.search("rust", 10);
        assert_eq!(results.len(), 2);
        // Higher TF should rank higher.
        assert_eq!(results[0].chunk_id, "frequent");
    }

    #[test]
    fn test_bm25_idf_matters() {
        let mut index = Bm25Index::new(SimpleTokenizer::new());
        // "common" appears in all docs.
        index.add("d1", "common word programming");
        index.add("d2", "common word design");
        index.add("d3", "rare unique specific programming");

        // Searching for "rare" should rank d3 higher due to IDF.
        let results = index.search("rare programming", 10);
        assert!(!results.is_empty());
        assert_eq!(results[0].chunk_id, "d3");
    }
}
