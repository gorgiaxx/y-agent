//! Quality filter for knowledge entries.
//!
//! Evaluates entries on minimum length, content deduplication, and
//! quality scoring before they are accepted into the knowledge base.

use crate::models::KnowledgeEntry;
use std::collections::HashSet;

/// Quality filter that evaluates knowledge entries for acceptance.
///
/// Checks:
/// - Minimum content length (< 50 tokens → reject)
/// - `content_hash` deduplication (exact match)
/// - Quality score computation (length + structure + domain match)
/// - `is_active` flag
pub struct QualityFilter {
    /// Minimum token count to accept an entry.
    min_tokens: u32,
    /// Set of already-seen content hashes for deduplication.
    seen_hashes: HashSet<String>,
}

impl QualityFilter {
    /// Create a new quality filter with default settings.
    pub fn new() -> Self {
        Self {
            min_tokens: 50,
            seen_hashes: HashSet::new(),
        }
    }

    /// Create a filter with a custom minimum token threshold.
    pub fn with_min_tokens(min_tokens: u32) -> Self {
        Self {
            min_tokens,
            seen_hashes: HashSet::new(),
        }
    }

    /// Evaluate an entry and return (accepted, `quality_score`).
    ///
    /// Quality score breakdown:
    /// - Length component (0.0–0.3): based on content length
    /// - Structure component (0.0–0.3): based on headings, paragraphs
    /// - Domain component (0.0–0.4): based on domain classification matches
    pub fn evaluate(&self, entry: &KnowledgeEntry) -> (bool, f32) {
        // Check is_active.
        if !entry.is_active {
            return (false, 0.0);
        }

        // Minimum length check.
        let token_count = estimate_tokens(&entry.content);
        if token_count < self.min_tokens {
            return (false, 0.0);
        }

        // Compute quality score.
        let length_score = compute_length_score(&entry.content);
        let structure_score = compute_structure_score(&entry.content);
        let domain_score = compute_domain_score(entry);

        let quality_score = length_score + structure_score + domain_score;
        (true, quality_score)
    }

    /// Check for duplicate content and register the hash.
    ///
    /// Returns `true` if the content hash is new (not a duplicate).
    pub fn check_duplicate(&mut self, content_hash: &str) -> bool {
        self.seen_hashes.insert(content_hash.to_string())
    }

    /// Reset the deduplication state.
    pub fn reset_dedup(&mut self) {
        self.seen_hashes.clear();
    }
}

impl Default for QualityFilter {
    fn default() -> Self {
        Self::new()
    }
}

/// Estimate token count (rough: chars / 4).
fn estimate_tokens(text: &str) -> u32 {
    let chars = u32::try_from(text.len()).unwrap_or(u32::MAX);
    chars.div_ceil(4)
}

/// Length score: 0.0–0.3.
///
/// - < 200 chars: 0.1
/// - 200–1000 chars: 0.2
/// - > 1000 chars: 0.3
fn compute_length_score(content: &str) -> f32 {
    match content.len() {
        0..200 => 0.1,
        200..1000 => 0.2,
        _ => 0.3,
    }
}

/// Structure score: 0.0–0.3.
///
/// Awards points for:
/// - Paragraphs (double newlines): +0.1 if >= 2
/// - Headings (lines starting with #): +0.1 if >= 1
/// - Sentences (sentence-ending punctuation): +0.1 if >= 3
fn compute_structure_score(content: &str) -> f32 {
    let mut score = 0.0_f32;

    // Paragraph count.
    let paragraphs = content.split("\n\n").filter(|s| !s.trim().is_empty()).count();
    if paragraphs >= 2 {
        score += 0.1;
    }

    // Heading count.
    let headings = content.lines().filter(|l| l.trim_start().starts_with('#')).count();
    if headings >= 1 {
        score += 0.1;
    }

    // Sentence count.
    let sentences = content
        .chars()
        .filter(|c| matches!(c, '.' | '!' | '?' | '。' | '！' | '？'))
        .count();
    if sentences >= 3 {
        score += 0.1;
    }

    score
}

/// Domain score: 0.0–0.4.
///
/// - No domains: 0.0
/// - 1 domain: 0.2
/// - 2+ domains: 0.4
fn compute_domain_score(entry: &KnowledgeEntry) -> f32 {
    match entry.domains.len() {
        0 => 0.0,
        1 => 0.2,
        _ => 0.4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{SourceRef, SourceType};
    use chrono::Utc;

    fn make_entry(content: &str) -> KnowledgeEntry {
        let source = SourceRef {
            source_type: SourceType::File,
            uri: "/test".to_string(),
            content_hash: "hash".to_string(),
            title: "Test".to_string(),
            author: None,
            fetched_at: Utc::now(),
            connector_id: None,
        };
        KnowledgeEntry::new("ws", "default", content, source)
    }

    #[test]
    fn test_quality_filter_accepts_good_content() {
        let filter = QualityFilter::new();
        let entry = make_entry(
            "This is a substantial document with enough content to pass quality checks. \
             It has multiple sentences and provides meaningful information about software development. \
             The quality score should be reasonable. Additional text to ensure we exceed the minimum token threshold for acceptance.",
        );
        let (accepted, score) = filter.evaluate(&entry);
        assert!(accepted, "good content should be accepted");
        assert!(score > 0.0, "score should be positive");
    }

    #[test]
    fn test_quality_filter_rejects_short_content() {
        let filter = QualityFilter::new();
        let entry = make_entry("Too short.");
        let (accepted, _) = filter.evaluate(&entry);
        assert!(!accepted, "short content should be rejected");
    }

    #[test]
    fn test_quality_filter_rejects_inactive() {
        let filter = QualityFilter::new();
        let mut entry = make_entry("Enough content to pass length check, this is a test document with many words.");
        entry.is_active = false;
        let (accepted, _) = filter.evaluate(&entry);
        assert!(!accepted, "inactive entry should be rejected");
    }

    #[test]
    fn test_quality_score_structure() {
        let filter = QualityFilter::new();

        // Content with structure — enough length to pass min tokens.
        let entry = make_entry(
            "# Title\n\nFirst paragraph with content about software engineering and best practices. \
             Second sentence explaining important details. Third one too for completeness.\n\n\
             Second paragraph with more detail about the actual implementation and design patterns.",
        );
        let (accepted, score) = filter.evaluate(&entry);
        assert!(accepted, "structured content should be accepted");
        assert!(
            score > 0.3,
            "structured content should score higher, got {score}"
        );
    }

    #[test]
    fn test_quality_score_with_domains() {
        let filter = QualityFilter::new();
        let mut entry = make_entry(
            "Long content about Rust programming that should pass all checks with flying colors and be accepted. \
             This document covers multiple topics including error handling, async operations, and testing strategies.",
        );
        entry.domains = vec!["rust".to_string(), "rust/async".to_string()];
        let (accepted, score) = filter.evaluate(&entry);
        assert!(accepted, "entry with domains should be accepted");
        assert!(
            score >= 0.5,
            "entry with domains should score higher, got {score}"
        );
    }

    #[test]
    fn test_deduplication() {
        let mut filter = QualityFilter::new();
        assert!(filter.check_duplicate("hash1"), "first occurrence should be new");
        assert!(!filter.check_duplicate("hash1"), "second occurrence should be duplicate");
        assert!(filter.check_duplicate("hash2"), "different hash should be new");
    }

    #[test]
    fn test_deduplication_reset() {
        let mut filter = QualityFilter::new();
        filter.check_duplicate("hash1");
        filter.reset_dedup();
        assert!(filter.check_duplicate("hash1"), "after reset should be new");
    }

    #[test]
    fn test_custom_min_tokens() {
        let filter = QualityFilter::with_min_tokens(10);
        let entry = make_entry("This is enough text to pass a low threshold easily.");
        let (accepted, _) = filter.evaluate(&entry);
        assert!(accepted);
    }
}
