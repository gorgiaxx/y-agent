//! Cross-resource linker: replaces redundant content with resource references.
//!
//! Uses keyword overlap for similarity detection (deterministic).
//! Designed for future embedding-based similarity via `y-provider`.

use std::collections::HashSet;

/// A replacement made by the linker.
#[derive(Debug, Clone)]
pub struct LinkReplacement {
    /// The reference inserted (e.g., `[skill:essay-writer]`).
    pub reference: String,
    /// The original text that was replaced.
    pub original_text: String,
    /// Similarity score (0.0–1.0).
    pub similarity: f64,
}

/// Report of all linkages applied.
#[derive(Debug, Clone)]
pub struct LinkageReport {
    /// Replacements made.
    pub replacements: Vec<LinkReplacement>,
    /// Total similarity matches found (including below threshold).
    pub total_matches: usize,
}

/// Links skill content to existing resources by detecting overlap.
#[derive(Debug)]
pub struct ResourceLinker {
    /// Similarity threshold for linking (0.0–1.0).
    similarity_threshold: f64,
}

impl ResourceLinker {
    /// Create a new linker with default threshold (0.5 keyword overlap).
    pub fn new() -> Self {
        Self {
            similarity_threshold: 0.5,
        }
    }

    /// Create a linker with a custom threshold.
    pub fn with_threshold(threshold: f64) -> Self {
        Self {
            similarity_threshold: threshold,
        }
    }

    /// Link content against known skills and tools.
    ///
    /// `known_skills` and `known_tools` are slices of `(name, description)` pairs.
    pub fn link(
        &self,
        content: &str,
        known_skills: &[(&str, &str)],
        known_tools: &[(&str, &str)],
    ) -> LinkageReport {
        let content_words = Self::extract_keywords(content);
        let mut replacements = Vec::new();
        let mut total_matches = 0usize;

        // Check against known skills
        for (name, description) in known_skills {
            let desc_words = Self::extract_keywords(description);
            let similarity = Self::keyword_similarity(&content_words, &desc_words);

            if similarity > 0.0 {
                total_matches += 1;
            }
            if similarity >= self.similarity_threshold {
                replacements.push(LinkReplacement {
                    reference: format!("[skill:{name}]"),
                    original_text: (*description).to_string(),
                    similarity,
                });
            }
        }

        // Check against known tools
        for (name, description) in known_tools {
            let desc_words = Self::extract_keywords(description);
            let similarity = Self::keyword_similarity(&content_words, &desc_words);

            if similarity > 0.0 {
                total_matches += 1;
            }
            if similarity >= self.similarity_threshold {
                replacements.push(LinkReplacement {
                    reference: format!("[tool:{name}]"),
                    original_text: (*description).to_string(),
                    similarity,
                });
            }
        }

        LinkageReport {
            replacements,
            total_matches,
        }
    }

    fn extract_keywords(text: &str) -> HashSet<String> {
        text.to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| w.len() > 3) // skip short words
            .map(String::from)
            .collect()
    }

    fn keyword_similarity(a: &HashSet<String>, b: &HashSet<String>) -> f64 {
        if a.is_empty() || b.is_empty() {
            return 0.0;
        }
        let intersection = a.intersection(b).count();
        let smaller = a.len().min(b.len());
        // Values are always small (word counts), no real precision loss.
        let result = intersection as f64 / smaller as f64;
        result
    }
}

impl Default for ResourceLinker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// T-SK-S5-05: Linker detects overlap with existing skills.
    #[test]
    fn test_linker_detects_skill_overlap() {
        let linker = ResourceLinker::with_threshold(0.3);

        let content = "Help the user write essays with clear structure, \
                        proper grammar, and strong arguments.";

        let known_skills = [
            (
                "essay-writer",
                "Write essays with clear structure and proper grammar",
            ),
            ("code-review", "Review code for bugs and style issues"),
        ];

        let report = linker.link(content, &known_skills, &[]);

        assert!(!report.replacements.is_empty());
        assert!(report.replacements[0].reference.contains("essay-writer"));
        assert!(report.replacements[0].similarity > 0.0);
    }

    /// No match for unrelated content.
    #[test]
    fn test_linker_no_match_unrelated() {
        let linker = ResourceLinker::with_threshold(0.5);
        let content = "Deploy kubernetes clusters on AWS infrastructure";

        let known_skills = [("essay-writer", "Write essays with proper grammar")];

        let report = linker.link(content, &known_skills, &[]);
        assert!(report.replacements.is_empty());
    }

    /// Tool matching works.
    #[test]
    fn test_linker_matches_tools() {
        let linker = ResourceLinker::with_threshold(0.3);
        let content = "Search the database for matching records and return results";

        let known_tools = [("db-search", "Search database for matching records")];

        let report = linker.link(content, &[], &known_tools);

        assert!(!report.replacements.is_empty());
        assert!(report.replacements[0].reference.contains("tool:db-search"));
    }
}
