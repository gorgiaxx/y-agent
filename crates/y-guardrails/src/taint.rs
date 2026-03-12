//! Taint tracking: propagation of trust tags through data flow.
//!
//! User input and external data are tagged as tainted. Taint propagates
//! through tool execution. Tainted data is blocked from reaching
//! dangerous sinks (filesystem writes, shell execution) unless sanitized.

use std::collections::{HashMap, HashSet};

/// A taint tag identifying the source of untrusted data.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TaintTag {
    /// Source of the taint (e.g., "`user_input`", "`web_response`").
    pub source: String,
    /// When the taint was applied.
    pub applied_at: String,
}

/// Tracks taint tags on data flowing through the system.
#[derive(Debug, Default)]
pub struct TaintTracker {
    /// Map from data ID to its taint tags.
    tags: HashMap<String, HashSet<TaintTag>>,
    /// Sinks that block tainted data.
    dangerous_sinks: HashSet<String>,
    /// Sanitizers that remove taint.
    sanitizers: HashSet<String>,
}

/// Result of checking taint against a sink.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaintCheckResult {
    /// Data is clean — no taint tags.
    Clean,
    /// Data is tainted but the sink is safe.
    TaintedButSafe,
    /// Data is tainted and the sink is dangerous — blocked.
    Blocked { tag: TaintTag, sink: String },
}

impl TaintTracker {
    /// Create a new taint tracker.
    pub fn new() -> Self {
        let mut dangerous_sinks = HashSet::new();
        // Default dangerous sinks
        dangerous_sinks.insert("filesystem_write".to_string());
        dangerous_sinks.insert("shell_execute".to_string());
        dangerous_sinks.insert("network_send".to_string());

        Self {
            tags: HashMap::new(),
            dangerous_sinks,
            sanitizers: HashSet::new(),
        }
    }

    /// Register a dangerous sink.
    pub fn add_dangerous_sink(&mut self, sink: impl Into<String>) {
        self.dangerous_sinks.insert(sink.into());
    }

    /// Register a sanitizer.
    pub fn add_sanitizer(&mut self, sanitizer: impl Into<String>) {
        self.sanitizers.insert(sanitizer.into());
    }

    /// Apply a taint tag to a piece of data.
    pub fn taint(&mut self, data_id: impl Into<String>, tag: TaintTag) {
        self.tags.entry(data_id.into()).or_default().insert(tag);
    }

    /// Propagate taint from source data to derived data (e.g., tool output).
    pub fn propagate(&mut self, source_id: &str, derived_id: impl Into<String>) {
        if let Some(source_tags) = self.tags.get(source_id).cloned() {
            let entry = self.tags.entry(derived_id.into()).or_default();
            for tag in source_tags {
                entry.insert(tag);
            }
        }
    }

    /// Remove all taint from data (e.g., after passing through a sanitizer).
    pub fn sanitize(&mut self, data_id: &str) {
        self.tags.remove(data_id);
    }

    /// Check whether data is clean (no taint tags).
    pub fn is_clean(&self, data_id: &str) -> bool {
        self.tags.get(data_id).is_none_or(HashSet::is_empty)
    }

    /// Get taint tags for a piece of data.
    pub fn get_tags(&self, data_id: &str) -> Vec<&TaintTag> {
        self.tags
            .get(data_id)
            .map(|set| set.iter().collect())
            .unwrap_or_default()
    }

    /// Check whether tainted data can reach a specific sink.
    pub fn check_sink(&self, data_id: &str, sink: &str) -> TaintCheckResult {
        let tags = match self.tags.get(data_id) {
            Some(t) if !t.is_empty() => t,
            _ => return TaintCheckResult::Clean,
        };

        if self.dangerous_sinks.contains(sink) {
            // Return the first taint tag as the blocking reason.
            // Safety: `tags` is guaranteed non-empty by the match guard above.
            if let Some(tag) = tags.iter().next() {
                TaintCheckResult::Blocked {
                    tag: tag.clone(),
                    sink: sink.to_string(),
                }
            } else {
                TaintCheckResult::Clean
            }
        } else {
            TaintCheckResult::TaintedButSafe
        }
    }

    /// Process data through a sanitizer step.
    /// Returns `true` if the sanitizer was recognized and taint was removed.
    pub fn apply_sanitizer(&mut self, data_id: &str, sanitizer: &str) -> bool {
        if self.sanitizers.contains(sanitizer) {
            self.sanitize(data_id);
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user_taint() -> TaintTag {
        TaintTag {
            source: "user_input".to_string(),
            applied_at: "2026-03-10T00:00:00Z".to_string(),
        }
    }

    /// T-GUARD-003-01: User input marked as tainted propagates.
    #[test]
    fn test_taint_tag_user_input() {
        let mut tracker = TaintTracker::new();
        tracker.taint("input_1", user_taint());

        assert!(!tracker.is_clean("input_1"));
        assert_eq!(tracker.get_tags("input_1").len(), 1);
        assert_eq!(tracker.get_tags("input_1")[0].source, "user_input");
    }

    /// T-GUARD-003-02: Tainted input → tool → output inherits taint.
    #[test]
    fn test_taint_propagation_through_tool() {
        let mut tracker = TaintTracker::new();
        tracker.taint("input_1", user_taint());

        // Tool processes input_1, produces output_1
        tracker.propagate("input_1", "output_1");

        assert!(!tracker.is_clean("output_1"));
        assert_eq!(tracker.get_tags("output_1")[0].source, "user_input");
    }

    /// T-GUARD-003-03: Tainted data passes sanitizer → taint removed.
    #[test]
    fn test_taint_sanitization() {
        let mut tracker = TaintTracker::new();
        tracker.add_sanitizer("html_escape");
        tracker.taint("input_1", user_taint());

        assert!(!tracker.is_clean("input_1"));
        let applied = tracker.apply_sanitizer("input_1", "html_escape");
        assert!(applied);
        assert!(tracker.is_clean("input_1"));
    }

    /// T-GUARD-003-04: Tainted data → filesystem write → blocked.
    #[test]
    fn test_taint_blocks_dangerous_sink() {
        let mut tracker = TaintTracker::new();
        tracker.taint("input_1", user_taint());

        let result = tracker.check_sink("input_1", "filesystem_write");
        assert_eq!(
            result,
            TaintCheckResult::Blocked {
                tag: user_taint(),
                sink: "filesystem_write".to_string(),
            }
        );
    }

    /// T-GUARD-003-05: Clean input has no taint tags.
    #[test]
    fn test_taint_no_propagation_for_clean_data() {
        let tracker = TaintTracker::new();

        assert!(tracker.is_clean("clean_data"));
        assert!(tracker.get_tags("clean_data").is_empty());

        let result = tracker.check_sink("clean_data", "filesystem_write");
        assert_eq!(result, TaintCheckResult::Clean);
    }
}
