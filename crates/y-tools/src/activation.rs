//! `ToolActivationSet`: LRU cache of active (fully-loaded) tools.
//!
//! The activation set tracks which tools are currently loaded with full
//! definitions in the session. It enforces a ceiling (default 20) with
//! LRU eviction. Tools marked `always_active` are never evicted.

use std::collections::{HashMap, VecDeque};

use y_core::tool::ToolDefinition;
use y_core::types::ToolName;

/// Session-scoped set of active tools with LRU eviction.
pub struct ToolActivationSet {
    /// Maximum number of active tools.
    ceiling: usize,
    /// Active tool definitions, keyed by name.
    active: HashMap<ToolName, ToolDefinition>,
    /// LRU order: front = oldest (first to evict), back = most recently used.
    lru_order: VecDeque<ToolName>,
    /// Tools that should never be evicted.
    always_active: std::collections::HashSet<ToolName>,
}

impl ToolActivationSet {
    /// Create a new activation set with the given ceiling.
    pub fn new(ceiling: usize) -> Self {
        Self {
            ceiling,
            active: HashMap::new(),
            lru_order: VecDeque::new(),
            always_active: std::collections::HashSet::new(),
        }
    }

    /// Activate a tool (add to the active set).
    pub fn activate(&mut self, definition: ToolDefinition) {
        let name = definition.name.clone();

        let already_active = self.active.contains_key(&name);

        if already_active {
            // Refresh LRU position for existing tool.
            self.lru_order.retain(|n| n != &name);
            self.lru_order.push_back(name);
            // Update the definition in-place.
            if let Some(entry) = self.active.get_mut(&definition.name) {
                *entry = definition;
            }
        } else {
            // Evict if at ceiling.
            while self.active.len() >= self.ceiling && self.evict_oldest() {}
            self.active.insert(name.clone(), definition);
            self.lru_order.push_back(name);
        }
    }

    /// Deactivate a specific tool.
    pub fn deactivate(&mut self, name: &ToolName) -> bool {
        if self.active.remove(name).is_some() {
            self.lru_order.retain(|n| n != name);
            self.always_active.remove(name);
            true
        } else {
            false
        }
    }

    /// Mark a tool as always active (never evicted).
    pub fn set_always_active(&mut self, name: &ToolName) {
        if self.active.contains_key(name) {
            self.always_active.insert(name.clone());
        }
    }

    /// Get a reference to an active tool's definition and refresh its LRU position.
    pub fn get(&mut self, name: &ToolName) -> Option<&ToolDefinition> {
        if self.active.contains_key(name) {
            self.refresh_lru(name);
            self.active.get(name)
        } else {
            None
        }
    }

    /// Get a reference to an active tool's definition WITHOUT refreshing LRU.
    ///
    /// Use for read-only lookups where LRU order should not change
    /// (e.g., building `ChatRequest.tools` from the active set).
    pub fn peek(&self, name: &ToolName) -> Option<&ToolDefinition> {
        self.active.get(name)
    }

    /// Get all active tool definitions.
    pub fn active_definitions(&self) -> Vec<&ToolDefinition> {
        self.active.values().collect()
    }

    /// Get definitions of tools marked as always-active.
    pub fn always_active_definitions(&self) -> Vec<&ToolDefinition> {
        self.always_active
            .iter()
            .filter_map(|name| self.active.get(name))
            .collect()
    }

    /// Number of active tools.
    pub fn len(&self) -> usize {
        self.active.len()
    }

    /// Whether the set is empty.
    pub fn is_empty(&self) -> bool {
        self.active.is_empty()
    }

    /// Check if a tool is in the active set.
    pub fn contains(&self, name: &ToolName) -> bool {
        self.active.contains_key(name)
    }

    fn refresh_lru(&mut self, name: &ToolName) {
        self.lru_order.retain(|n| n != name);
        self.lru_order.push_back(name.clone());
    }

    fn evict_oldest(&mut self) -> bool {
        let evict_pos = self
            .lru_order
            .iter()
            .position(|n| !self.always_active.contains(n));

        if let Some(pos) = evict_pos {
            if let Some(name) = self.lru_order.remove(pos) {
                self.active.remove(&name);
                return true;
            }
        }
        false
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
            description: format!("{name} tool"),
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
    fn test_activation_add_tool() {
        let mut set = ToolActivationSet::new(20);
        set.activate(sample_definition("FileRead"));
        assert!(set.contains(&ToolName::from_string("FileRead")));
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn test_activation_lru_eviction_at_ceiling() {
        let mut set = ToolActivationSet::new(20);
        for i in 0..21 {
            set.activate(sample_definition(&format!("tool_{i}")));
        }
        assert_eq!(set.len(), 20);
        assert!(!set.contains(&ToolName::from_string("tool_0")));
        assert!(set.contains(&ToolName::from_string("tool_20")));
    }

    #[test]
    fn test_activation_access_refreshes_lru() {
        let mut set = ToolActivationSet::new(3);
        set.activate(sample_definition("a"));
        set.activate(sample_definition("b"));
        set.activate(sample_definition("c"));

        set.get(&ToolName::from_string("a"));

        set.activate(sample_definition("d"));
        assert!(set.contains(&ToolName::from_string("a")));
        assert!(!set.contains(&ToolName::from_string("b")));
        assert!(set.contains(&ToolName::from_string("c")));
        assert!(set.contains(&ToolName::from_string("d")));
    }

    #[test]
    fn test_activation_always_active_not_evicted() {
        let mut set = ToolActivationSet::new(3);
        set.activate(sample_definition("always"));
        set.set_always_active(&ToolName::from_string("always"));
        set.activate(sample_definition("b"));
        set.activate(sample_definition("c"));

        set.activate(sample_definition("d"));
        assert!(set.contains(&ToolName::from_string("always")));
        assert!(!set.contains(&ToolName::from_string("b")));
    }

    #[test]
    fn test_activation_get_active_definitions() {
        let mut set = ToolActivationSet::new(20);
        for i in 0..5 {
            set.activate(sample_definition(&format!("tool_{i}")));
        }
        let defs = set.active_definitions();
        assert_eq!(defs.len(), 5);
    }

    #[test]
    fn test_activation_deactivate() {
        let mut set = ToolActivationSet::new(20);
        set.activate(sample_definition("FileRead"));
        assert!(set.deactivate(&ToolName::from_string("FileRead")));
        assert!(!set.contains(&ToolName::from_string("FileRead")));
        assert_eq!(set.len(), 0);
    }
}
