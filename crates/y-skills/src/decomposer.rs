//! Document decomposer: splits oversized skill content into root + sub-documents.
//!
//! Uses heading-based splitting with token budget awareness. Currently
//! deterministic (pattern-based); designed for future LLM-assisted enhancement.

use std::fmt::Write;

use crate::manifest::estimate_tokens;

/// Default token threshold per root document.
const DEFAULT_TOKEN_THRESHOLD: u32 = 2000;

/// A sub-document produced by decomposition.
#[derive(Debug, Clone)]
pub struct SubDocEntry {
    /// Unique identifier for this sub-document.
    pub id: String,
    /// Title extracted from the heading.
    pub title: String,
    /// The sub-document content.
    pub content: String,
    /// Estimated token count.
    pub token_estimate: u32,
    /// Depth level in the tree (1-indexed).
    pub level: u32,
}

/// Result of document decomposition.
#[derive(Debug, Clone)]
pub struct DecomposedSkill {
    /// Root document content (condensed summary or intro).
    pub root_content: String,
    /// Sub-documents split from the original.
    pub sub_documents: Vec<SubDocEntry>,
    /// Tree index: maps sub-doc IDs to their titles for navigation.
    pub tree_index: Vec<(String, String)>,
}

/// Splits oversized skill content into a tree structure.
#[derive(Debug)]
pub struct DocumentDecomposer {
    /// Token threshold for the root document.
    token_threshold: u32,
}

#[allow(clippy::unused_self)]
impl DocumentDecomposer {
    /// Create a new decomposer with default threshold (2000 tokens).
    pub fn new() -> Self {
        Self {
            token_threshold: DEFAULT_TOKEN_THRESHOLD,
        }
    }

    /// Create a decomposer with a custom threshold.
    pub fn with_threshold(threshold: u32) -> Self {
        Self {
            token_threshold: threshold,
        }
    }

    /// Decompose content into root + sub-documents.
    pub fn decompose(&self, content: &str) -> DecomposedSkill {
        let total_tokens = estimate_tokens(content);

        // Under threshold: keep as single root
        if total_tokens <= self.token_threshold {
            return DecomposedSkill {
                root_content: content.to_string(),
                sub_documents: vec![],
                tree_index: vec![],
            };
        }

        // Split by level-2 headings (## )
        let sections = self.split_by_headings(content);

        if sections.len() <= 1 {
            // No headings to split on — keep as single root
            return DecomposedSkill {
                root_content: content.to_string(),
                sub_documents: vec![],
                tree_index: vec![],
            };
        }

        // First section becomes root (intro/summary), rest become sub-docs
        let root_content = self.build_root(&sections);
        let mut sub_documents = Vec::new();
        let mut tree_index = Vec::new();

        for (i, section) in sections.iter().enumerate().skip(1) {
            let id = format!("sub_{i:02}");
            let title = section.heading.clone();
            let token_estimate = estimate_tokens(&section.content);

            tree_index.push((id.clone(), title.clone()));
            sub_documents.push(SubDocEntry {
                id,
                title,
                content: section.content.clone(),
                token_estimate,
                level: 1,
            });
        }

        DecomposedSkill {
            root_content,
            sub_documents,
            tree_index,
        }
    }

    fn split_by_headings(&self, content: &str) -> Vec<Section> {
        let mut sections = Vec::new();
        let mut current_heading = String::new();
        let mut current_lines: Vec<&str> = Vec::new();

        for line in content.lines() {
            if line.starts_with("## ") {
                // Save previous section
                if !current_lines.is_empty() || !current_heading.is_empty() {
                    sections.push(Section {
                        heading: current_heading.clone(),
                        content: current_lines.join("\n"),
                    });
                }
                current_heading = line.trim_start_matches("## ").to_string();
                current_lines = vec![line];
            } else {
                current_lines.push(line);
            }
        }

        // Push last section
        if !current_lines.is_empty() || !current_heading.is_empty() {
            sections.push(Section {
                heading: current_heading,
                content: current_lines.join("\n"),
            });
        }

        sections
    }

    fn build_root(&self, sections: &[Section]) -> String {
        let mut root = String::new();

        // Keep the intro (first section) in full
        if let Some(intro) = sections.first() {
            root.push_str(&intro.content);
        }

        // Add a table of contents pointing to sub-docs
        root.push_str("\n\n## Contents\n\n");
        for (i, section) in sections.iter().enumerate().skip(1) {
            let id = format!("sub_{i:02}");
            let _ = writeln!(root, "- [{id}] {}", section.heading);
        }

        root
    }
}

impl Default for DocumentDecomposer {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
struct Section {
    heading: String,
    content: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// T-SK-S5-01: Decomposer splits 5000-token document into root + sub-docs.
    #[test]
    fn test_decomposer_splits_large_document() {
        let decomposer = DocumentDecomposer::with_threshold(100);

        // Build content with multiple sections that exceed threshold
        let mut content = String::from("# My Skill\n\nIntroduction paragraph.\n\n");
        for i in 1..=5 {
            content.push_str(&format!("## Section {i}\n\n"));
            // Add enough text to exceed threshold
            for j in 1..=20 {
                content.push_str(&format!(
                    "Line {j} of section {i} with some content to fill tokens.\n"
                ));
            }
            content.push('\n');
        }

        let result = decomposer.decompose(&content);

        assert!(
            !result.sub_documents.is_empty(),
            "should have sub-documents"
        );
        assert_eq!(result.sub_documents.len(), 5);
        assert_eq!(result.tree_index.len(), 5);
        assert!(result.root_content.contains("Contents"));
        assert_eq!(result.sub_documents[0].title, "Section 1");
        assert_eq!(result.sub_documents[0].level, 1);
    }

    /// T-SK-S5-02: Decomposer keeps sub-threshold content as single root.
    #[test]
    fn test_decomposer_keeps_small_content() {
        let decomposer = DocumentDecomposer::new(); // 2000 token threshold
        let content = "# Small Skill\n\nThis is a small skill that fits in one document.";

        let result = decomposer.decompose(content);

        assert!(result.sub_documents.is_empty());
        assert_eq!(result.root_content, content);
        assert!(result.tree_index.is_empty());
    }

    /// No headings available — stays as single root.
    #[test]
    fn test_decomposer_no_headings() {
        let decomposer = DocumentDecomposer::with_threshold(10);
        let content = "Just a long block of text without any headings at all.\n\
                        More text here to exceed the threshold for sure.\n\
                        And even more text to make sure we pass the limit.";

        let result = decomposer.decompose(content);

        // Without ## headings, can't split, so stays as root
        assert!(result.sub_documents.is_empty());
    }
}
