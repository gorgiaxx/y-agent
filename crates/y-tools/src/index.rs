//! `ToolIndex`: compact tool entries for LLM context injection.
//!
//! The index contains only name, description, and category — enough for the
//! LLM to decide whether to call `ToolSearch` for the full definition.
//! This design reduces context window consumption by 60-90%.

use std::collections::HashMap;

use y_core::tool::{ToolDefinition, ToolIndexEntry};
use y_core::types::ToolName;

/// Generates and maintains a compact index of registered tools.
///
/// The index is rebuilt on registration/unregistration events and
/// cached for fast access during context assembly.
pub struct ToolIndex {
    entries: HashMap<ToolName, ToolIndexEntry>,
}

impl ToolIndex {
    /// Create a new empty tool index.
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// Build an index entry from a full tool definition.
    fn entry_from_definition(def: &ToolDefinition) -> ToolIndexEntry {
        ToolIndexEntry {
            name: def.name.clone(),
            description: def.description.clone(),
            category: def.category,
        }
    }

    /// Add or update a tool in the index.
    pub fn add(&mut self, definition: &ToolDefinition) {
        let entry = Self::entry_from_definition(definition);
        self.entries.insert(definition.name.clone(), entry);
    }

    /// Remove a tool from the index.
    pub fn remove(&mut self, name: &ToolName) -> bool {
        self.entries.remove(name).is_some()
    }

    /// Get all index entries (for context injection).
    pub fn entries(&self) -> Vec<ToolIndexEntry> {
        self.entries.values().cloned().collect()
    }

    /// Number of tools in the index.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the index is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for ToolIndex {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use y_core::runtime::RuntimeCapability;
    use y_core::tool::{ToolCategory, ToolType};

    use super::*;

    fn sample_definition(name: &str) -> ToolDefinition {
        ToolDefinition {
            name: ToolName::from_string(name),
            description: format!("{name} description"),
            help: None,
            parameters: serde_json::json!({}),
            result_schema: None,
            category: ToolCategory::FileSystem,
            tool_type: ToolType::BuiltIn,
            capabilities: RuntimeCapability::default(),
            is_dangerous: false,
        }
    }

    #[test]
    fn test_index_returns_compact_entries() {
        let mut index = ToolIndex::new();
        index.add(&sample_definition("FileRead"));
        let entries = index.entries();
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(entry.name.as_str(), "FileRead");
        assert_eq!(entry.description, "FileRead description");
        assert_eq!(entry.category, ToolCategory::FileSystem);
    }

    #[test]
    fn test_index_excludes_full_schema() {
        let mut index = ToolIndex::new();
        let mut def = sample_definition("FileWrite");
        def.parameters = serde_json::json!({
            "type": "object",
            "properties": { "path": { "type": "string" } }
        });
        index.add(&def);
        let entries = index.entries();
        let serialized = serde_json::to_value(&entries[0]).unwrap();
        assert!(serialized.get("parameters").is_none());
    }

    #[test]
    fn test_index_includes_all_registered_tools() {
        let mut index = ToolIndex::new();
        for i in 0..5 {
            index.add(&sample_definition(&format!("tool_{i}")));
        }
        assert_eq!(index.len(), 5);
        assert_eq!(index.entries().len(), 5);
    }

    #[test]
    fn test_index_updates_on_register() {
        let mut index = ToolIndex::new();
        index.add(&sample_definition("FileRead"));
        assert_eq!(index.len(), 1);
        index.add(&sample_definition("FileWrite"));
        assert_eq!(index.len(), 2);
    }

    #[test]
    fn test_index_updates_on_unregister() {
        let mut index = ToolIndex::new();
        index.add(&sample_definition("FileRead"));
        index.add(&sample_definition("FileWrite"));
        assert_eq!(index.len(), 2);
        assert!(index.remove(&ToolName::from_string("FileRead")));
        assert_eq!(index.len(), 1);
        let names: Vec<_> = index
            .entries()
            .iter()
            .map(|e| e.name.as_str().to_string())
            .collect();
        assert!(!names.contains(&"FileRead".to_string()));
    }
}
