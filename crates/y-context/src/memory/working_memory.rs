//! Working Memory: pipeline-scoped in-memory blackboard with 4 cognitive categories.
//!
//! WM is created fresh per pipeline run and dropped after the pipeline completes.
//! It tracks token estimates for budget awareness.

use std::collections::HashMap;

/// Cognitive categories for working memory slots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CognitiveCategory {
    /// Current goals and plans.
    Goals,
    /// Observations from tools and environment.
    Observations,
    /// Intermediate reasoning steps.
    Reasoning,
    /// Actions taken and their results.
    Actions,
}

/// A single slot in working memory.
#[derive(Debug, Clone)]
pub struct WmSlot {
    pub key: String,
    pub value: String,
    pub category: CognitiveCategory,
    pub token_estimate: u32,
}

/// Pipeline-scoped in-memory blackboard.
///
/// Created fresh per pipeline execution. Put/get/clear slots organized
/// by cognitive category, with token budget tracking.
#[derive(Debug, Default)]
pub struct WorkingMemory {
    slots: HashMap<String, WmSlot>,
    total_tokens: u32,
}

impl WorkingMemory {
    pub fn new() -> Self {
        Self::default()
    }

    /// Put a value in a named slot under a cognitive category.
    pub fn put(&mut self, key: &str, value: &str, category: CognitiveCategory) {
        let token_estimate = estimate_tokens(value);

        // Remove old slot token count if replacing
        if let Some(old) = self.slots.get(key) {
            self.total_tokens = self.total_tokens.saturating_sub(old.token_estimate);
        }

        self.total_tokens += token_estimate;
        self.slots.insert(
            key.to_string(),
            WmSlot {
                key: key.to_string(),
                value: value.to_string(),
                category,
                token_estimate,
            },
        );
    }

    /// Get a slot value by key.
    pub fn get(&self, key: &str) -> Option<&WmSlot> {
        self.slots.get(key)
    }

    /// Get all slots in a given cognitive category.
    pub fn by_category(&self, category: CognitiveCategory) -> Vec<&WmSlot> {
        self.slots
            .values()
            .filter(|s| s.category == category)
            .collect()
    }

    /// Total estimated tokens in all slots.
    pub fn total_tokens(&self) -> u32 {
        self.total_tokens
    }

    /// Number of slots.
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    /// Whether the working memory is empty.
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// Clear all slots.
    pub fn clear(&mut self) {
        self.slots.clear();
        self.total_tokens = 0;
    }
}

fn estimate_tokens(text: &str) -> u32 {
    let chars = u32::try_from(text.len()).unwrap_or(u32::MAX);
    chars.div_ceil(4)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// T-MEM-001-01: Put value in slot, get returns same value.
    #[test]
    fn test_wm_put_get_slot() {
        let mut wm = WorkingMemory::new();
        wm.put("plan", "step 1: read file", CognitiveCategory::Goals);

        let slot = wm.get("plan").unwrap();
        assert_eq!(slot.value, "step 1: read file");
        assert_eq!(slot.category, CognitiveCategory::Goals);
    }

    /// T-MEM-001-02: Put in each of 4 categories, each accessible.
    #[test]
    fn test_wm_cognitive_categories() {
        let mut wm = WorkingMemory::new();
        wm.put("goal", "fix bug", CognitiveCategory::Goals);
        wm.put("obs", "error log", CognitiveCategory::Observations);
        wm.put(
            "reason",
            "likely null pointer",
            CognitiveCategory::Reasoning,
        );
        wm.put("act", "patched file", CognitiveCategory::Actions);

        assert_eq!(wm.by_category(CognitiveCategory::Goals).len(), 1);
        assert_eq!(wm.by_category(CognitiveCategory::Observations).len(), 1);
        assert_eq!(wm.by_category(CognitiveCategory::Reasoning).len(), 1);
        assert_eq!(wm.by_category(CognitiveCategory::Actions).len(), 1);
    }

    /// T-MEM-001-03: Token estimate is tracked.
    #[test]
    fn test_wm_token_estimate_tracked() {
        let mut wm = WorkingMemory::new();
        assert_eq!(wm.total_tokens(), 0);

        wm.put(
            "goal",
            "fix the bug in the codebase",
            CognitiveCategory::Goals,
        );
        assert!(wm.total_tokens() > 0);
    }

    /// T-MEM-001-04: Clear resets all slots.
    #[test]
    fn test_wm_clear_resets() {
        let mut wm = WorkingMemory::new();
        wm.put("a", "val", CognitiveCategory::Goals);
        wm.put("b", "val", CognitiveCategory::Actions);

        wm.clear();
        assert!(wm.is_empty());
        assert_eq!(wm.total_tokens(), 0);
    }

    /// T-MEM-001-05: Pipeline-scoped lifetime (created → used → drop).
    #[test]
    fn test_wm_pipeline_scoped_lifetime() {
        let result = {
            let mut wm = WorkingMemory::new();
            wm.put("task", "process data", CognitiveCategory::Goals);
            wm.get("task").map(|s| s.value.clone())
        };
        // WM is dropped here but the value was extracted
        assert_eq!(result, Some("process data".to_string()));
    }
}
