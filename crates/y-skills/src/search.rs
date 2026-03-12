//! Skill search: tag matching and trigger pattern matching.

use y_core::skill::{SkillManifest, SkillSummary};

/// Searches through skills by tag and trigger pattern matching.
#[derive(Debug, Default)]
pub struct SkillSearch {
    /// All registered skills (kept in memory for fast search).
    manifests: Vec<SkillManifest>,
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
        let query_lower = query.to_lowercase();

        let mut results: Vec<(usize, &SkillManifest)> = self
            .manifests
            .iter()
            .filter_map(|m| {
                let score = Self::score_match(m, &query_lower);
                if score > 0 {
                    Some((score, m))
                } else {
                    None
                }
            })
            .collect();

        // Sort by relevance score (descending)
        results.sort_by(|a, b| b.0.cmp(&a.0));

        results
            .into_iter()
            .take(limit)
            .map(|(_, m)| SkillSummary {
                id: m.id.clone(),
                name: m.name.clone(),
                description: m.description.clone(),
                tags: m.tags.clone(),
                token_estimate: m.token_estimate,
            })
            .collect()
    }

    /// Score how well a manifest matches a query.
    fn score_match(manifest: &SkillManifest, query_lower: &str) -> usize {
        let mut score = 0;

        // Tag match (highest weight)
        for tag in &manifest.tags {
            if tag.to_lowercase().contains(query_lower) {
                score += 10;
            }
        }

        // Trigger pattern match
        for pattern in &manifest.trigger_patterns {
            if pattern.to_lowercase().contains(query_lower) {
                score += 5;
            }
        }

        // Name/description match (lower weight)
        if manifest.name.to_lowercase().contains(query_lower) {
            score += 3;
        }
        if manifest.description.to_lowercase().contains(query_lower) {
            score += 1;
        }

        score
    }
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
            safety: None,
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
}
