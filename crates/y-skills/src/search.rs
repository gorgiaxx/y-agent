//! Skill search: tag matching and trigger pattern matching.

use y_core::skill::{SkillManifest, SkillSummary};

/// Searches through skills by tag and trigger pattern matching.
#[derive(Debug, Default)]
pub struct SkillSearch {
    /// All registered skills (kept in memory for fast search).
    manifests: Vec<SkillManifest>,
}

/// Skill search result with its deterministic relevance score.
#[derive(Debug, Clone)]
pub struct ScoredSkillSummary {
    pub summary: SkillSummary,
    pub score: usize,
}

impl SkillSearch {
    /// Create a new empty skill search index.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a manifest to the search index.
    pub fn index(&mut self, manifest: SkillManifest) {
        // Remove any existing manifest with the same ID
        self.manifests.retain(|m| m.id != manifest.id);
        self.manifests.push(manifest);
    }

    /// Remove a manifest from the search index.
    pub fn remove(&mut self, skill_id: &y_core::types::SkillId) {
        self.manifests.retain(|m| m.id != *skill_id);
    }

    /// Search skills by query string.
    ///
    /// Matches against tags and trigger patterns (case-insensitive).
    /// Returns compact `SkillSummary` entries, not full manifests.
    pub fn search(&self, query: &str, limit: usize) -> Vec<SkillSummary> {
        self.search_scored(query, limit, 1)
            .into_iter()
            .map(|result| result.summary)
            .collect()
    }

    /// Search with deterministic scores and a minimum relevance threshold.
    pub fn search_scored(
        &self,
        query: &str,
        limit: usize,
        min_score: usize,
    ) -> Vec<ScoredSkillSummary> {
        let query_tokens = query_tokens(query);
        let mut results: Vec<(usize, &SkillManifest)> = self
            .manifests
            .iter()
            .filter_map(|m| {
                let score = Self::score_match(m, &query_tokens);
                if score >= min_score {
                    Some((score, m))
                } else {
                    None
                }
            })
            .collect();

        // Sort by relevance score (descending)
        results.sort_by(|left, right| {
            right
                .0
                .cmp(&left.0)
                .then_with(|| left.1.name.cmp(&right.1.name))
        });

        results
            .into_iter()
            .take(limit)
            .map(|(score, m)| ScoredSkillSummary {
                summary: SkillSummary {
                    id: m.id.clone(),
                    name: m.name.clone(),
                    description: m.description.clone(),
                    tags: m.tags.clone(),
                    token_estimate: m.token_estimate,
                },
                score,
            })
            .collect()
    }

    /// Score how well a manifest matches a query.
    fn score_match(manifest: &SkillManifest, query_tokens: &[String]) -> usize {
        let mut score = 0;
        let name = manifest.name.to_lowercase();
        let description = manifest.description.to_lowercase();
        let tags: Vec<_> = manifest.tags.iter().map(|tag| tag.to_lowercase()).collect();
        let triggers: Vec<_> = manifest
            .trigger_patterns
            .iter()
            .map(|trigger| trigger.to_lowercase())
            .collect();

        for token in query_tokens {
            for tag in &tags {
                if tag == token {
                    score += 12;
                } else if tag.contains(token) {
                    score += 8;
                }
            }
            for trigger in &triggers {
                if trigger.contains(token) {
                    score += 5;
                }
            }
            if name.contains(token) {
                score += 4;
            }
            if description.contains(token) {
                score += 2;
            }
        }

        score
    }
}

fn query_tokens(query: &str) -> Vec<String> {
    const STOP_WORDS: &[&str] = &[
        "and", "are", "for", "from", "into", "please", "that", "the", "this", "with",
    ];

    let mut tokens: Vec<_> = query
        .split(|character: char| !character.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|token| token.len() >= 3 && !STOP_WORDS.contains(&token.as_str()))
        .collect();
    tokens.sort();
    tokens.dedup();
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;
    use y_core::skill::{SkillManifest, SkillVersion};
    use y_core::types::{now, SkillId};

    fn test_manifest(name: &str, tags: &[&str], triggers: &[&str]) -> SkillManifest {
        let now = now();
        SkillManifest {
            id: SkillId::new(),
            name: name.to_string(),
            description: format!("A skill about {name}"),
            version: SkillVersion("v1".to_string()),
            tags: tags.iter().map(|t| (*t).to_string()).collect(),
            trigger_patterns: triggers.iter().map(|t| (*t).to_string()).collect(),
            knowledge_bases: vec![],
            root_content: "root content".to_string(),
            sub_documents: vec![],
            token_estimate: 10,
            created_at: now,
            updated_at: now,
            classification: None,
            constraints: None,
            security: None,
            references: None,
            author: None,
            source_format: None,
            source_hash: None,
            state: None,
            root_path: None,
        }
    }

    /// T-SKILL-003-01: Search by tag returns matching skills.
    #[test]
    fn test_search_by_tag() {
        let mut search = SkillSearch::new();
        search.index(test_manifest(
            "rust-errors",
            &["rust", "error-handling"],
            &[],
        ));
        search.index(test_manifest("python-basics", &["python"], &[]));

        let results = search.search("rust", 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "rust-errors");
    }

    /// T-SKILL-003-02: Query matching trigger pattern returns skill.
    #[test]
    fn test_search_by_trigger_pattern() {
        let mut search = SkillSearch::new();
        search.index(test_manifest(
            "error-skill",
            &[],
            &["how to handle errors in rust"],
        ));

        let results = search.search("errors", 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "error-skill");
    }

    /// T-SKILL-003-03: Limit is respected.
    #[test]
    fn test_search_respects_limit() {
        let mut search = SkillSearch::new();
        for i in 0..10 {
            search.index(test_manifest(&format!("rust-skill-{i}"), &["rust"], &[]));
        }

        let results = search.search("rust", 3);
        assert_eq!(results.len(), 3);
    }

    /// T-SKILL-003-04: Unmatched query returns empty.
    #[test]
    fn test_search_no_match() {
        let mut search = SkillSearch::new();
        search.index(test_manifest("rust-errors", &["rust"], &[]));

        let results = search.search("javascript", 10);
        assert!(results.is_empty());
    }

    /// T-SKILL-003-05: Search returns `SkillSummary` without `root_content`.
    #[test]
    fn test_search_returns_summaries_not_full() {
        let mut search = SkillSearch::new();
        search.index(test_manifest("rust-skill", &["rust"], &[]));

        let results = search.search("rust", 10);
        assert_eq!(results.len(), 1);
        // SkillSummary has name, description, tags, token_estimate — no root_content
        assert_eq!(results[0].name, "rust-skill");
        assert!(!results[0].tags.is_empty());
    }

    #[test]
    fn test_search_matches_relevant_tokens_in_a_natural_language_request() {
        let mut search = SkillSearch::new();
        search.index(test_manifest(
            "rust-errors",
            &["rust", "error-handling"],
            &["diagnose rust compiler errors"],
        ));
        search.index(test_manifest("python-basics", &["python"], &[]));

        let results = search.search("Please review the Rust error handling in this change", 10);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "rust-errors");
    }

    #[test]
    fn test_scored_search_exposes_a_threshold_for_automatic_selection() {
        let mut search = SkillSearch::new();
        search.index(test_manifest("rust-errors", &["rust"], &[]));
        search.index(test_manifest("generic-review", &[], &["review"]));

        let results = search.search_scored("Review this Rust implementation", 10, 12);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].summary.name, "rust-errors");
        assert!(results[0].score >= 12);
    }
}
