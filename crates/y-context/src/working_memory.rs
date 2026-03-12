//! Working Memory: cognitive-category-scoped slots for micro-agent pipelines.
//!
//! Design reference: micro-agent-pipeline-design.md §Working Memory
//!
//! Working Memory provides typed slots organized by cognitive categories.
//! Each slot carries a `token_estimate` for budget enforcement. The
//! micro-agent pipeline reads/writes slots to pass structured data
//! between stateless steps without accumulating full conversation context.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Cognitive categories
// ---------------------------------------------------------------------------

/// Cognitive categories that organize Working Memory slots.
///
/// Each micro-agent step declares which categories it reads (inputs)
/// and which category it writes (output). This scoping ensures steps
/// receive only the information they need, keeping token usage minimal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CognitiveCategory {
    /// Raw observations from the environment (files, tool output, user input).
    Perception,
    /// Structural understanding (code maps, dependency graphs, schemas).
    Structure,
    /// Analytical reasoning (root-cause conclusions, trade-off evaluations).
    Analysis,
    /// Executable plans and artifacts (diffs, commands, code changes).
    Execution,
    /// Verification results (test output, review checklist outcomes).
    Validation,
}

// ---------------------------------------------------------------------------
// Working Memory slots
// ---------------------------------------------------------------------------

/// A single Working Memory slot containing typed data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkingMemorySlot {
    /// The cognitive category this slot belongs to.
    pub category: CognitiveCategory,
    /// A human-readable label for this slot.
    pub label: String,
    /// The slot value (structured JSON).
    pub value: serde_json::Value,
    /// Estimated token count for this slot's content.
    pub token_estimate: u32,
    /// The pipeline step that produced this slot.
    pub producer_step: Option<String>,
}

// ---------------------------------------------------------------------------
// Working Memory
// ---------------------------------------------------------------------------

/// Token budget configuration for Working Memory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenBudget {
    /// Maximum total tokens across all slots.
    pub max_total_tokens: u32,
    /// Optional per-category limits.
    pub category_limits: HashMap<CognitiveCategory, u32>,
}

impl Default for TokenBudget {
    fn default() -> Self {
        Self {
            max_total_tokens: 4000,
            category_limits: HashMap::new(),
        }
    }
}

/// Error from Working Memory operations.
#[derive(Debug, thiserror::Error)]
pub enum WorkingMemoryError {
    #[error("token budget exceeded: used {used}, limit {limit}")]
    BudgetExceeded { used: u32, limit: u32 },
    #[error("category budget exceeded for {category:?}: used {used}, limit {limit}")]
    CategoryBudgetExceeded {
        category: CognitiveCategory,
        used: u32,
        limit: u32,
    },
    #[error("slot not found: {label}")]
    SlotNotFound { label: String },
}

/// Working Memory for micro-agent pipeline communication.
///
/// Slots are organized by cognitive category and carry token estimates
/// for budget enforcement. Steps read from input categories and write
/// to a single output category.
#[derive(Debug, Default)]
pub struct WorkingMemory {
    /// All slots indexed by label.
    slots: HashMap<String, WorkingMemorySlot>,
    /// Token budget configuration.
    budget: TokenBudget,
}

impl WorkingMemory {
    /// Create a new Working Memory with default budget.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new Working Memory with a specific budget.
    pub fn with_budget(budget: TokenBudget) -> Self {
        Self {
            slots: HashMap::new(),
            budget,
        }
    }

    /// Write a slot to Working Memory.
    ///
    /// Enforces token budget: rejects writes that would exceed the total
    /// or per-category limit.
    pub fn write_slot(&mut self, slot: WorkingMemorySlot) -> Result<(), WorkingMemoryError> {
        // Calculate what total would be after this write.
        let current_total = self.total_tokens();
        // If updating an existing slot, subtract its old tokens.
        let old_tokens = self.slots.get(&slot.label).map_or(0, |s| s.token_estimate);
        let new_total = current_total - old_tokens + slot.token_estimate;

        if new_total > self.budget.max_total_tokens {
            return Err(WorkingMemoryError::BudgetExceeded {
                used: new_total,
                limit: self.budget.max_total_tokens,
            });
        }

        // Check per-category limit.
        if let Some(&cat_limit) = self.budget.category_limits.get(&slot.category) {
            let current_cat = self.tokens_for_category(slot.category);
            let old_cat = self
                .slots
                .get(&slot.label)
                .filter(|s| s.category == slot.category)
                .map_or(0, |s| s.token_estimate);
            let new_cat = current_cat - old_cat + slot.token_estimate;

            if new_cat > cat_limit {
                return Err(WorkingMemoryError::CategoryBudgetExceeded {
                    category: slot.category,
                    used: new_cat,
                    limit: cat_limit,
                });
            }
        }

        self.slots.insert(slot.label.clone(), slot);
        Ok(())
    }

    /// Read a slot by label.
    pub fn read_slot(&self, label: &str) -> Option<&WorkingMemorySlot> {
        self.slots.get(label)
    }

    /// Get all slots for a given cognitive category.
    pub fn slots_for_category(&self, category: CognitiveCategory) -> Vec<&WorkingMemorySlot> {
        self.slots
            .values()
            .filter(|s| s.category == category)
            .collect()
    }

    /// Total estimated tokens across all slots.
    pub fn total_tokens(&self) -> u32 {
        self.slots.values().map(|s| s.token_estimate).sum()
    }

    /// Tokens used by a specific cognitive category.
    pub fn tokens_for_category(&self, category: CognitiveCategory) -> u32 {
        self.slots
            .values()
            .filter(|s| s.category == category)
            .map(|s| s.token_estimate)
            .sum()
    }

    /// Number of slots.
    pub fn slot_count(&self) -> usize {
        self.slots.len()
    }

    /// Check if empty.
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// Remove a slot by label.
    pub fn remove_slot(&mut self, label: &str) -> Option<WorkingMemorySlot> {
        self.slots.remove(label)
    }

    /// Get all slot labels.
    pub fn labels(&self) -> Vec<&str> {
        self.slots.keys().map(String::as_str).collect()
    }

    /// Extract slots for specified categories (used to build per-step context).
    ///
    /// Returns a new `WorkingMemory` containing only slots from the requested
    /// categories. The budget is inherited from the source.
    #[must_use]
    pub fn extract_for_categories(&self, categories: &[CognitiveCategory]) -> WorkingMemory {
        let filtered: HashMap<String, WorkingMemorySlot> = self
            .slots
            .iter()
            .filter(|(_, slot)| categories.contains(&slot.category))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        WorkingMemory {
            slots: filtered,
            budget: self.budget.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn perception_slot(label: &str, tokens: u32) -> WorkingMemorySlot {
        WorkingMemorySlot {
            category: CognitiveCategory::Perception,
            label: label.to_string(),
            value: serde_json::json!({"data": label}),
            token_estimate: tokens,
            producer_step: Some("step-1".to_string()),
        }
    }

    fn analysis_slot(label: &str, tokens: u32) -> WorkingMemorySlot {
        WorkingMemorySlot {
            category: CognitiveCategory::Analysis,
            label: label.to_string(),
            value: serde_json::json!({"result": label}),
            token_estimate: tokens,
            producer_step: Some("step-2".to_string()),
        }
    }

    /// T-P3-36-01: Write and read a Working Memory slot.
    #[test]
    fn test_wm_write_read_slot() {
        let mut wm = WorkingMemory::new();
        let slot = perception_slot("file_contents", 100);
        wm.write_slot(slot).unwrap();

        let read = wm.read_slot("file_contents").unwrap();
        assert_eq!(read.category, CognitiveCategory::Perception);
        assert_eq!(read.token_estimate, 100);
        assert_eq!(read.producer_step.as_deref(), Some("step-1"));
    }

    /// T-P3-36-02: Slots are scoped by cognitive category.
    #[test]
    fn test_wm_category_scoping() {
        let mut wm = WorkingMemory::new();
        wm.write_slot(perception_slot("obs-1", 50)).unwrap();
        wm.write_slot(perception_slot("obs-2", 75)).unwrap();
        wm.write_slot(analysis_slot("conclusion", 200)).unwrap();

        let perception = wm.slots_for_category(CognitiveCategory::Perception);
        assert_eq!(perception.len(), 2);

        let analysis = wm.slots_for_category(CognitiveCategory::Analysis);
        assert_eq!(analysis.len(), 1);

        let structure = wm.slots_for_category(CognitiveCategory::Structure);
        assert!(structure.is_empty());
    }

    /// T-P3-36-03: Token budget enforcement rejects over-budget writes.
    #[test]
    fn test_wm_token_budget_enforcement() {
        let budget = TokenBudget {
            max_total_tokens: 200,
            category_limits: HashMap::new(),
        };
        let mut wm = WorkingMemory::with_budget(budget);

        wm.write_slot(perception_slot("small", 100)).unwrap();
        // This should fail: 100 + 150 = 250 > 200
        let result = wm.write_slot(perception_slot("too-big", 150));
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            WorkingMemoryError::BudgetExceeded { .. }
        ));

        // Total should still be 100 (the failed write didn't modify state).
        assert_eq!(wm.total_tokens(), 100);
    }

    /// T-P3-36-04: Per-category budget enforcement.
    #[test]
    fn test_wm_category_budget_enforcement() {
        let mut category_limits = HashMap::new();
        category_limits.insert(CognitiveCategory::Perception, 100);

        let budget = TokenBudget {
            max_total_tokens: 4000,
            category_limits,
        };
        let mut wm = WorkingMemory::with_budget(budget);

        wm.write_slot(perception_slot("obs-1", 80)).unwrap();
        // Category limit 100, used 80, adding 50 = 130 > 100 → fail
        let result = wm.write_slot(perception_slot("obs-2", 50));
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            WorkingMemoryError::CategoryBudgetExceeded { .. }
        ));

        // Analysis category is not limited, should still work.
        wm.write_slot(analysis_slot("ok", 500)).unwrap();
        assert_eq!(wm.slot_count(), 2);
    }

    /// T-P3-36-05: Extract slots for specific categories builds a filtered view.
    #[test]
    fn test_wm_extract_for_categories() {
        let mut wm = WorkingMemory::new();
        wm.write_slot(perception_slot("obs-1", 50)).unwrap();
        wm.write_slot(analysis_slot("conclusion", 200)).unwrap();
        wm.write_slot(WorkingMemorySlot {
            category: CognitiveCategory::Structure,
            label: "code-map".to_string(),
            value: serde_json::json!({"map": "..."}),
            token_estimate: 300,
            producer_step: None,
        })
        .unwrap();

        let filtered = wm
            .extract_for_categories(&[CognitiveCategory::Perception, CognitiveCategory::Structure]);
        assert_eq!(filtered.slot_count(), 2);
        assert!(filtered.read_slot("obs-1").is_some());
        assert!(filtered.read_slot("code-map").is_some());
        assert!(filtered.read_slot("conclusion").is_none());
    }

    /// T-P3-36-06: Updating an existing slot reclaims token budget.
    #[test]
    fn test_wm_update_slot_reclaims_budget() {
        let budget = TokenBudget {
            max_total_tokens: 200,
            category_limits: HashMap::new(),
        };
        let mut wm = WorkingMemory::with_budget(budget);

        wm.write_slot(perception_slot("data", 150)).unwrap();
        assert_eq!(wm.total_tokens(), 150);

        // Update the same slot with smaller content — should reclaim budget.
        wm.write_slot(perception_slot("data", 50)).unwrap();
        assert_eq!(wm.total_tokens(), 50);

        // Now we have room for more.
        wm.write_slot(analysis_slot("extra", 100)).unwrap();
        assert_eq!(wm.total_tokens(), 150);
    }
}
