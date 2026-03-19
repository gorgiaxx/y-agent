//! Domain classifier for knowledge entries.
//!
//! Classifies content into domain categories using keyword-based rules.
//! Supports hierarchical domain taxonomy (e.g., `rust/async`, `testing/automation`).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Classifier trait
// ---------------------------------------------------------------------------

/// Trait for domain classification of content.
pub trait Classifier: Send + Sync {
    /// Classify content and return matching domain paths.
    fn classify(&self, content: &str) -> Vec<String>;
}

// ---------------------------------------------------------------------------
// Domain Taxonomy
// ---------------------------------------------------------------------------

/// A node in the domain taxonomy tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainNode {
    /// Domain path (e.g., `"rust/async"`).
    pub path: String,
    /// Keywords that trigger this domain (case-insensitive).
    pub keywords: Vec<String>,
}

/// Hierarchical domain taxonomy for keyword-based classification.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DomainTaxonomy {
    /// All domain nodes in the taxonomy.
    pub domains: Vec<DomainNode>,
}

impl DomainTaxonomy {
    /// Create a new empty taxonomy.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a domain with its keywords.
    pub fn add_domain(&mut self, path: impl Into<String>, keywords: Vec<&str>) -> &mut Self {
        self.domains.push(DomainNode {
            path: path.into(),
            keywords: keywords.into_iter().map(str::to_lowercase).collect(),
        });
        self
    }

    /// Build a default taxonomy covering common development domains.
    ///
    /// Keywords are chosen to be **specific** enough that casual mentions of
    /// common English words (e.g. "result", "test", "query") do not trigger
    /// false-positive classifications.
    pub fn default_taxonomy() -> Self {
        let mut taxonomy = Self::new();

        taxonomy
            .add_domain(
                "rust",
                vec![
                    "rustc",
                    "cargo",
                    "crate",
                    "rust programming",
                    "rustfmt",
                    "clippy",
                    "rust-analyzer",
                    "cargo.toml",
                ],
            )
            .add_domain(
                "rust/async",
                vec![
                    "tokio",
                    "async-trait",
                    "futures-rs",
                    "async_std",
                    "tokio::spawn",
                ],
            )
            .add_domain(
                "rust/error",
                vec!["thiserror", "anyhow", "error chain", "impl error"],
            )
            .add_domain(
                "python",
                vec![
                    "python",
                    "pip install",
                    "django",
                    "flask",
                    "pytorch",
                    "numpy",
                    "pandas",
                    "pyproject",
                ],
            )
            .add_domain(
                "javascript",
                vec![
                    "javascript",
                    "typescript",
                    "react",
                    "vue.js",
                    "angular",
                    "npm install",
                    "package.json",
                    "webpack",
                    "vite",
                ],
            )
            .add_domain(
                "testing",
                vec![
                    "unittest",
                    "pytest",
                    "jest",
                    "test suite",
                    "#[test]",
                    "test coverage",
                    "integration test",
                ],
            )
            .add_domain(
                "testing/automation",
                vec![
                    "github actions",
                    "ci/cd",
                    "pipeline",
                    "gitlab ci",
                    "jenkins",
                ],
            )
            .add_domain(
                "database",
                vec![
                    "postgres",
                    "postgresql",
                    "sqlite",
                    "mysql",
                    "database schema",
                    "sql query",
                    "migration",
                ],
            )
            .add_domain(
                "database/vector",
                vec![
                    "qdrant",
                    "pinecone",
                    "weaviate",
                    "vector database",
                    "embedding model",
                    "cosine similarity",
                ],
            )
            .add_domain(
                "devops",
                vec![
                    "docker",
                    "kubernetes",
                    "dockerfile",
                    "helm",
                    "terraform",
                    "container image",
                ],
            )
            .add_domain(
                "ai/llm",
                vec![
                    "llm",
                    "openai",
                    "claude",
                    "language model",
                    "prompt engineering",
                    "fine-tuning",
                    "transformer",
                ],
            );

        taxonomy
    }
}

// ---------------------------------------------------------------------------
// Rule-Based Classifier
// ---------------------------------------------------------------------------

/// Keyword-based domain classifier.
///
/// Matches content against a `DomainTaxonomy` by checking for keyword
/// occurrences (case-insensitive). A domain must have at least
/// `MIN_KEYWORD_HITS` matching keywords to be included, which reduces
/// false positives from common English words.
#[derive(Debug, Clone)]
pub struct RuleBasedClassifier {
    taxonomy: DomainTaxonomy,
}

/// Minimum number of keyword hits required for a domain to be classified.
///
/// Setting this to 2 means a single accidental word match will not cause
/// a domain tag to appear — at least two keywords must be present.
const MIN_KEYWORD_HITS: usize = 2;

impl RuleBasedClassifier {
    /// Create a classifier with a custom taxonomy.
    pub fn new(taxonomy: DomainTaxonomy) -> Self {
        Self { taxonomy }
    }

    /// Create a classifier with the default development taxonomy.
    pub fn default_taxonomy() -> Self {
        Self::new(DomainTaxonomy::default_taxonomy())
    }
}

impl Classifier for RuleBasedClassifier {
    fn classify(&self, content: &str) -> Vec<String> {
        let content_lower = content.to_lowercase();
        let words: Vec<&str> = content_lower.split_whitespace().collect();

        // Count keyword hits per domain.
        let mut scores: HashMap<&str, usize> = HashMap::new();

        for domain in &self.taxonomy.domains {
            let hits: usize = domain
                .keywords
                .iter()
                .filter(|kw| {
                    if kw.contains(' ') {
                        // Multi-word keyword: substring match.
                        content_lower.contains(kw.as_str())
                    } else if kw.len() <= 3 {
                        // Short keyword: require exact word match to
                        // avoid false positives (e.g. "ci" in "recipes").
                        words.iter().any(|w| {
                            let stripped: String =
                                w.chars().filter(|c| c.is_alphanumeric()).collect();
                            stripped == kw.as_str()
                        })
                    } else {
                        // Longer keyword: substring match is fine.
                        content_lower.contains(kw.as_str())
                    }
                })
                .count();
            if hits >= MIN_KEYWORD_HITS {
                scores.insert(&domain.path, hits);
            }
        }

        // Sort by hit count descending, return domain paths.
        let mut sorted: Vec<(&str, usize)> = scores.into_iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(&a.1));

        sorted
            .into_iter()
            .map(|(path, _)| path.to_string())
            .collect()
    }
}

// ---------------------------------------------------------------------------
// LLM Classifier (placeholder)
// ---------------------------------------------------------------------------

/// Placeholder trait for future LLM-based classification.
///
/// Will use an LLM to classify content into domains with higher accuracy
/// than keyword matching. Implementation deferred to Sprint 3+.
#[async_trait::async_trait]
pub trait LlmClassifier: Send + Sync {
    /// Classify content using an LLM.
    async fn classify_with_llm(
        &self,
        content: &str,
    ) -> Result<Vec<String>, crate::error::KnowledgeError>;
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rule_based_classifier_rust() {
        let classifier = RuleBasedClassifier::default_taxonomy();
        // Need at least 2 keyword hits per domain
        let domains =
            classifier.classify("Rust programming with cargo build using tokio and async-trait");

        assert!(
            domains.iter().any(|d| d == "rust"),
            "expected 'rust' domain, got {domains:?}"
        );
        assert!(
            domains.iter().any(|d| d == "rust/async"),
            "expected 'rust/async' domain, got {domains:?}"
        );
    }

    #[test]
    fn test_rule_based_classifier_python() {
        let classifier = RuleBasedClassifier::default_taxonomy();
        let domains =
            classifier.classify("Python web framework with Django and numpy data processing");

        assert!(
            domains.iter().any(|d| d == "python"),
            "expected 'python' domain, got {domains:?}"
        );
    }

    #[test]
    fn test_rule_based_classifier_no_match() {
        let classifier = RuleBasedClassifier::default_taxonomy();
        let domains = classifier.classify("Cooking recipes and gardening tips");
        assert!(domains.is_empty(), "expected no domains, got {domains:?}");
    }

    #[test]
    fn test_rule_based_classifier_no_false_positive_common_words() {
        let classifier = RuleBasedClassifier::default_taxonomy();
        // Text with common English words that used to trigger false positives
        let domains = classifier.classify(
            "The result of the test showed that the query returned a vector of future values. \
             We want to deploy this node to handle async requests.",
        );
        assert!(
            domains.is_empty(),
            "expected no false-positive domains from common words, got {domains:?}"
        );
    }

    #[test]
    fn test_rule_based_classifier_multiple_domains() {
        let classifier = RuleBasedClassifier::default_taxonomy();
        let domains = classifier.classify(
            "Using pytest and jest for test coverage in a CI/CD pipeline with github actions",
        );

        assert!(
            domains.len() >= 2,
            "expected multiple domains, got {domains:?}"
        );
    }

    #[test]
    fn test_rule_based_classifier_case_insensitive() {
        let classifier = RuleBasedClassifier::default_taxonomy();
        let domains = classifier.classify("RUST PROGRAMMING with CARGO and CLIPPY");

        assert!(
            domains.iter().any(|d| d == "rust"),
            "case-insensitive match failed, got {domains:?}"
        );
    }

    #[test]
    fn test_custom_taxonomy() {
        let mut taxonomy = DomainTaxonomy::new();
        taxonomy.add_domain("cooking", vec!["recipe", "bake", "cook"]);

        let classifier = RuleBasedClassifier::new(taxonomy);
        let domains = classifier.classify("A baking recipe for cookies");

        assert!(
            domains.iter().any(|d| d == "cooking"),
            "custom taxonomy failed, got {domains:?}"
        );
    }

    #[test]
    fn test_taxonomy_serialization() {
        let taxonomy = DomainTaxonomy::default_taxonomy();
        let json = serde_json::to_string(&taxonomy).expect("serialize");
        let deserialized: DomainTaxonomy = serde_json::from_str(&json).expect("deserialize");
        assert!(!deserialized.domains.is_empty());
    }
}
