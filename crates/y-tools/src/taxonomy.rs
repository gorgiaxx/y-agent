//! Hierarchical tool taxonomy for prompt-based tool discovery.
//!
//! Provides a tree-structured taxonomy of tool categories loaded from TOML.
//! The LLM sees only the compact root summary (~100 tokens) and uses
//! `ToolSearch` to drill into categories or find specific tools.
//!
//! Design reference: `docs/design/tool-search-design.md`,
//!                    `docs/standards/TOOL_CALL_PROTOCOL.md`

use std::collections::HashMap;
use std::fmt;

use serde::Deserialize;

/// A tool name reference within the taxonomy.
pub type ToolName = String;

/// Error type for taxonomy operations.
#[derive(Debug)]
pub enum TaxonomyError {
    /// TOML parsing failed.
    ParseError(String),
    /// No categories defined.
    EmptyTaxonomy,
}

impl fmt::Display for TaxonomyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TaxonomyError::ParseError(msg) => write!(f, "taxonomy parse error: {msg}"),
            TaxonomyError::EmptyTaxonomy => write!(f, "taxonomy has no categories"),
        }
    }
}

impl std::error::Error for TaxonomyError {}

/// Hierarchical tool taxonomy.
///
/// Loaded from a TOML configuration. Provides compact summaries for prompt
/// injection and drill-down navigation for the `ToolSearch` meta-tool.
#[derive(Debug, Clone)]
pub struct ToolTaxonomy {
    categories: HashMap<String, TaxonomyCategory>,
}

/// A top-level taxonomy category.
#[derive(Debug, Clone)]
pub struct TaxonomyCategory {
    /// Human-readable label.
    pub label: String,
    /// Description of what tools in this category do.
    pub description: String,
    /// Sub-categories within this category.
    pub subcategories: HashMap<String, TaxonomySubcategory>,
    /// Tools directly in this category (not in a subcategory).
    pub tools: Vec<ToolName>,
}

/// A sub-category within a taxonomy category.
#[derive(Debug, Clone)]
pub struct TaxonomySubcategory {
    /// Human-readable label.
    pub label: String,
    /// Description.
    pub description: String,
    /// Tools in this sub-category.
    pub tools: Vec<ToolName>,
}

// ---------------------------------------------------------------------------
// TOML deserialization types (internal)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct TomlTaxonomy {
    categories: HashMap<String, TomlCategory>,
}

#[derive(Deserialize)]
struct TomlCategory {
    label: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    subcategories: HashMap<String, TomlSubcategory>,
    #[serde(default)]
    tools: Vec<String>,
}

#[derive(Deserialize)]
struct TomlSubcategory {
    label: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    tools: Vec<String>,
}

// ---------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------

impl ToolTaxonomy {
    /// Parse a taxonomy from a TOML string.
    pub fn from_toml(config: &str) -> Result<Self, TaxonomyError> {
        let parsed: TomlTaxonomy =
            toml::from_str(config).map_err(|e| TaxonomyError::ParseError(e.to_string()))?;

        if parsed.categories.is_empty() {
            return Err(TaxonomyError::EmptyTaxonomy);
        }

        let categories = parsed
            .categories
            .into_iter()
            .map(|(key, cat)| {
                let subcategories = cat
                    .subcategories
                    .into_iter()
                    .map(|(sk, sc)| {
                        (
                            sk,
                            TaxonomySubcategory {
                                label: sc.label,
                                description: sc.description,
                                tools: sc.tools,
                            },
                        )
                    })
                    .collect();

                (
                    key,
                    TaxonomyCategory {
                        label: cat.label,
                        description: cat.description,
                        subcategories,
                        tools: cat.tools,
                    },
                )
            })
            .collect();

        Ok(Self { categories })
    }

    /// Generate a compact root summary for prompt injection.
    ///
    /// This should be ~100 tokens — category names and one-line descriptions.
    pub fn root_summary(&self) -> String {
        let mut lines = vec![
            "## Tool Categories\n".to_string(),
            "Use `ToolSearch` with a category key or keywords to discover tools.\n".to_string(),
            "| Category | Description |".to_string(),
            "|----------|-------------|".to_string(),
        ];

        let mut keys: Vec<&String> = self.categories.keys().collect();
        keys.sort();

        for key in keys {
            let cat = &self.categories[key];
            lines.push(format!("| {} | {} |", key, cat.description));
        }

        lines.join("\n")
    }

    /// Get detailed info for a specific category (subcategories + tools).
    pub fn category_detail(&self, category: &str) -> Option<String> {
        let cat = self.categories.get(category)?;
        let mut lines = Vec::new();
        lines.push(format!("## {} ({})\n", cat.label, category));
        lines.push(cat.description.clone());

        if !cat.tools.is_empty() {
            lines.push(format!("\nTools: {}", cat.tools.join(", ")));
        }

        if !cat.subcategories.is_empty() {
            lines.push("\nSubcategories:".to_string());
            let mut sub_keys: Vec<&String> = cat.subcategories.keys().collect();
            sub_keys.sort();
            for sk in sub_keys {
                let sub = &cat.subcategories[sk];
                let tools_str = if sub.tools.is_empty() {
                    String::new()
                } else {
                    format!(" — tools: {}", sub.tools.join(", "))
                };
                lines.push(format!(
                    "- **{}** ({}): {}{}",
                    sub.label, sk, sub.description, tools_str
                ));
            }
        }

        Some(lines.join("\n"))
    }

    /// Search for tools matching a keyword query.
    ///
    /// Splits the query on whitespace, commas, or semicolons into individual
    /// keywords and matches any keyword (OR semantics). A text field matches
    /// if it contains at least one keyword.
    ///
    /// Searches category labels, descriptions, subcategory labels/descriptions,
    /// and tool names. Returns matching tool names.
    pub fn search(&self, query: &str) -> Vec<ToolName> {
        let keywords: Vec<String> = query
            .split(|c: char| c.is_whitespace() || c == ',' || c == ';')
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .collect();

        if keywords.is_empty() {
            return Vec::new();
        }

        let text_matches = |text: &str| -> bool {
            let lower = text.to_lowercase();
            keywords.iter().any(|kw| lower.contains(kw.as_str()))
        };

        let mut seen = std::collections::HashSet::new();
        let mut results = Vec::new();

        let mut push_unique = |tool: &ToolName| {
            if seen.insert(tool.clone()) {
                results.push(tool.clone());
            }
        };

        for cat in self.categories.values() {
            for tool in &cat.tools {
                if text_matches(tool) {
                    push_unique(tool);
                }
            }

            let cat_match = text_matches(&cat.label) || text_matches(&cat.description);

            if cat_match {
                for tool in &cat.tools {
                    push_unique(tool);
                }
                for sub in cat.subcategories.values() {
                    for tool in &sub.tools {
                        push_unique(tool);
                    }
                }
            }

            for sub in cat.subcategories.values() {
                let sub_match = text_matches(&sub.label) || text_matches(&sub.description);

                for tool in &sub.tools {
                    if sub_match || text_matches(tool) {
                        push_unique(tool);
                    }
                }
            }
        }

        results.sort();
        results
    }

    /// Get all tool names in a category (including subcategories).
    pub fn tools_in_category(&self, category: &str) -> Vec<ToolName> {
        let Some(cat) = self.categories.get(category) else {
            return Vec::new();
        };

        let mut tools = cat.tools.clone();
        for sub in cat.subcategories.values() {
            tools.extend(sub.tools.iter().cloned());
        }
        tools.sort();
        tools.dedup();
        tools
    }

    /// Add a dynamic category at runtime (e.g. for MCP tools).
    ///
    /// If the category already exists, the tools are merged into it.
    pub fn add_dynamic_category(&mut self, key: &str, description: &str, tools: Vec<ToolName>) {
        if let Some(cat) = self.categories.get_mut(key) {
            for t in tools {
                if !cat.tools.contains(&t) {
                    cat.tools.push(t);
                }
            }
        } else {
            self.categories.insert(
                key.to_string(),
                TaxonomyCategory {
                    label: description.to_string(),
                    description: description.to_string(),
                    subcategories: HashMap::new(),
                    tools,
                },
            );
        }
    }

    /// Get the number of top-level categories.
    pub fn category_count(&self) -> usize {
        self.categories.len()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_TOML: &str = r#"
[categories.file]
label = "File Management"
description = "Read, write, and manage files"

[categories.file.subcategories.read]
label = "File Reading"
description = "Read file contents"
tools = ["FileRead"]

[categories.file.subcategories.write]
label = "File Writing"
description = "Create or modify files"
tools = ["FileWrite"]

[categories.shell]
label = "Shell"
description = "Execute shell commands"
tools = ["ShellExec"]

[categories.meta]
label = "Meta Tools"
description = "Tool management tools"
tools = ["ToolSearch"]
"#;

    #[test]
    fn test_from_toml_parses_categories() {
        let taxonomy = ToolTaxonomy::from_toml(TEST_TOML).unwrap();
        assert_eq!(taxonomy.category_count(), 3);
    }

    #[test]
    fn test_from_toml_empty_categories_is_error() {
        let toml = "[categories]\n";
        let result = ToolTaxonomy::from_toml(toml);
        assert!(result.is_err());
    }

    #[test]
    fn test_from_toml_invalid_toml_is_error() {
        let result = ToolTaxonomy::from_toml("not valid toml {{{");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("parse error"));
    }

    #[test]
    fn test_root_summary_contains_all_categories() {
        let taxonomy = ToolTaxonomy::from_toml(TEST_TOML).unwrap();
        let summary = taxonomy.root_summary();
        assert!(summary.contains("file"));
        assert!(summary.contains("shell"));
        assert!(summary.contains("meta"));
        assert!(summary.contains("Tool Categories"));
        assert!(summary.contains("ToolSearch"));
    }

    #[test]
    fn test_root_summary_is_compact() {
        let taxonomy = ToolTaxonomy::from_toml(TEST_TOML).unwrap();
        let summary = taxonomy.root_summary();
        // Rough token estimate: ~4 chars per token, should be well under 200 tokens.
        let estimated_tokens = summary.len() / 4;
        assert!(
            estimated_tokens < 200,
            "root summary too large: ~{estimated_tokens} tokens"
        );
    }

    #[test]
    fn test_category_detail_existing() {
        let taxonomy = ToolTaxonomy::from_toml(TEST_TOML).unwrap();
        let detail = taxonomy.category_detail("file").unwrap();
        assert!(detail.contains("File Management"));
        assert!(detail.contains("File Reading"));
        assert!(detail.contains("FileRead"));
        assert!(detail.contains("FileWrite"));
    }

    #[test]
    fn test_category_detail_with_direct_tools() {
        let taxonomy = ToolTaxonomy::from_toml(TEST_TOML).unwrap();
        let detail = taxonomy.category_detail("shell").unwrap();
        assert!(detail.contains("ShellExec"));
    }

    #[test]
    fn test_category_detail_nonexistent() {
        let taxonomy = ToolTaxonomy::from_toml(TEST_TOML).unwrap();
        assert!(taxonomy.category_detail("nonexistent").is_none());
    }

    #[test]
    fn test_search_by_tool_name() {
        let taxonomy = ToolTaxonomy::from_toml(TEST_TOML).unwrap();
        let results = taxonomy.search("FileRead");
        assert!(results.contains(&"FileRead".to_string()));
    }

    #[test]
    fn test_search_by_category_description() {
        let taxonomy = ToolTaxonomy::from_toml(TEST_TOML).unwrap();
        let results = taxonomy.search("shell commands");
        assert!(results.contains(&"ShellExec".to_string()));
    }

    #[test]
    fn test_search_multi_keyword_space_separated() {
        let taxonomy = ToolTaxonomy::from_toml(TEST_TOML).unwrap();
        // "tool search" should match "ToolSearch" via the "search" keyword
        let results = taxonomy.search("tool search");
        assert!(results.contains(&"ToolSearch".to_string()));
    }

    #[test]
    fn test_search_multi_keyword_comma_separated() {
        let taxonomy = ToolTaxonomy::from_toml(TEST_TOML).unwrap();
        let results = taxonomy.search("shell,file");
        assert!(results.contains(&"ShellExec".to_string()));
        assert!(results.contains(&"FileRead".to_string()));
    }

    #[test]
    fn test_search_multi_keyword_semicolon_separated() {
        let taxonomy = ToolTaxonomy::from_toml(TEST_TOML).unwrap();
        let results = taxonomy.search("shell; file");
        assert!(results.contains(&"ShellExec".to_string()));
        assert!(results.contains(&"FileRead".to_string()));
    }

    #[test]
    fn test_search_empty_query_returns_nothing() {
        let taxonomy = ToolTaxonomy::from_toml(TEST_TOML).unwrap();
        let results = taxonomy.search("");
        assert!(results.is_empty());
        // Also test whitespace-only.
        let results2 = taxonomy.search("   ");
        assert!(results2.is_empty());
    }

    #[test]
    fn test_search_by_subcategory_description() {
        let taxonomy = ToolTaxonomy::from_toml(TEST_TOML).unwrap();
        let results = taxonomy.search("Read file contents");
        assert!(results.contains(&"FileRead".to_string()));
    }

    #[test]
    fn test_search_case_insensitive() {
        let taxonomy = ToolTaxonomy::from_toml(TEST_TOML).unwrap();
        let results = taxonomy.search("FILE");
        assert!(!results.is_empty());
    }

    #[test]
    fn test_search_no_duplicates() {
        let taxonomy = ToolTaxonomy::from_toml(TEST_TOML).unwrap();
        let results = taxonomy.search("file");
        let unique: std::collections::HashSet<_> = results.iter().collect();
        assert_eq!(results.len(), unique.len());
    }

    #[test]
    fn test_search_no_results() {
        let taxonomy = ToolTaxonomy::from_toml(TEST_TOML).unwrap();
        let results = taxonomy.search("quantum_entanglement");
        assert!(results.is_empty());
    }

    #[test]
    fn test_tools_in_category() {
        let taxonomy = ToolTaxonomy::from_toml(TEST_TOML).unwrap();
        let tools = taxonomy.tools_in_category("file");
        assert!(tools.contains(&"FileRead".to_string()));
        assert!(tools.contains(&"FileWrite".to_string()));
    }

    #[test]
    fn test_tools_in_category_nonexistent() {
        let taxonomy = ToolTaxonomy::from_toml(TEST_TOML).unwrap();
        let tools = taxonomy.tools_in_category("nonexistent");
        assert!(tools.is_empty());
    }

    #[test]
    fn test_taxonomy_error_display() {
        let e = TaxonomyError::ParseError("bad toml".into());
        assert!(e.to_string().contains("bad toml"));

        let e = TaxonomyError::EmptyTaxonomy;
        assert!(e.to_string().contains("no categories"));
    }
}
