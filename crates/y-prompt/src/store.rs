//! `SectionStore`: TOML-based section persistence and retrieval.

use std::collections::HashMap;

use crate::section::{ContentSource, PromptSection, SectionId};

/// Errors from the section store.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("section not found: {id}")]
    NotFound { id: String },

    #[error("content load error for {id}: {message}")]
    ContentLoadError { id: String, message: String },

    #[error("TOML parse error: {message}")]
    ParseError { message: String },
}

/// In-memory section store with optional file-based content loading.
///
/// Sections can be registered inline or loaded from TOML files.
pub struct SectionStore {
    sections: HashMap<SectionId, PromptSection>,
}

impl SectionStore {
    /// Create a new empty section store.
    pub fn new() -> Self {
        Self {
            sections: HashMap::new(),
        }
    }

    /// Register a section.
    pub fn register(&mut self, section: PromptSection) {
        self.sections.insert(section.id.clone(), section);
    }

    /// Get a section by ID.
    pub fn get(&self, id: &str) -> Option<&PromptSection> {
        self.sections.get(id)
    }

    /// Load the text content of a section.
    ///
    /// For inline sources, returns the content directly.
    /// For file sources, reads from disk.
    /// For store sources, looks up the referenced key.
    pub fn load_content(&self, id: &str) -> Result<String, StoreError> {
        let section = self
            .get(id)
            .ok_or_else(|| StoreError::NotFound { id: id.to_string() })?;

        match &section.content_source {
            ContentSource::Inline(text) => Ok(text.clone()),
            ContentSource::File(path) => {
                std::fs::read_to_string(path).map_err(|e| StoreError::ContentLoadError {
                    id: id.to_string(),
                    message: e.to_string(),
                })
            }
            ContentSource::Store(key) => {
                // Recursively look up the referenced section.
                if key == id {
                    return Err(StoreError::ContentLoadError {
                        id: id.to_string(),
                        message: "self-referencing store key".into(),
                    });
                }
                self.load_content(key)
            }
        }
    }

    /// List all registered section IDs.
    pub fn section_ids(&self) -> Vec<&str> {
        self.sections.keys().map(String::as_str).collect()
    }

    /// Number of registered sections.
    pub fn len(&self) -> usize {
        self.sections.len()
    }

    /// Whether the store is empty.
    pub fn is_empty(&self) -> bool {
        self.sections.is_empty()
    }
}

impl Default for SectionStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use crate::section::SectionCategory;

    use super::*;

    fn inline_section(id: &str, content: &str, priority: i32) -> PromptSection {
        PromptSection {
            id: id.into(),
            content_source: ContentSource::Inline(content.into()),
            token_budget: 200,
            priority,
            condition: None,
            category: SectionCategory::Identity,
        }
    }

    #[test]
    fn test_store_register_and_get() {
        let mut store = SectionStore::new();
        store.register(inline_section("core.identity", "You are an agent.", 100));
        assert!(store.get("core.identity").is_some());
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn test_store_load_inline_content() {
        let mut store = SectionStore::new();
        store.register(inline_section("core.identity", "You are an agent.", 100));
        let content = store.load_content("core.identity").unwrap();
        assert_eq!(content, "You are an agent.");
    }

    #[test]
    fn test_store_load_not_found() {
        let store = SectionStore::new();
        let err = store.load_content("nonexistent").unwrap_err();
        assert!(matches!(err, StoreError::NotFound { .. }));
    }

    #[test]
    fn test_store_self_referencing_key() {
        let mut store = SectionStore::new();
        store.register(PromptSection {
            id: "self_ref".into(),
            content_source: ContentSource::Store("self_ref".into()),
            token_budget: 100,
            priority: 100,
            condition: None,
            category: SectionCategory::Identity,
        });
        let err = store.load_content("self_ref").unwrap_err();
        assert!(matches!(err, StoreError::ContentLoadError { .. }));
    }

    #[test]
    fn test_store_section_ids() {
        let mut store = SectionStore::new();
        store.register(inline_section("a", "A", 100));
        store.register(inline_section("b", "B", 200));
        let mut ids = store.section_ids();
        ids.sort_unstable();
        assert_eq!(ids, vec!["a", "b"]);
    }
}
