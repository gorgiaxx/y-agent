//! Garbage collection: removes unreferenced version snapshots.
//!
//! The GC identifies version objects that are not the current HEAD
//! and are older than the configured keep window. It can run
//! on-demand via CLI or as a scheduled background task.

use std::collections::HashSet;

use crate::version::VersionStore;

/// Garbage collector for skill version objects.
#[derive(Debug)]
pub struct SkillGarbageCollector {
    /// Number of recent versions to keep per skill (default: 10).
    keep_count: usize,
}

/// Result of a garbage collection run.
#[derive(Debug, Clone)]
pub struct GcResult {
    /// Skill name that was garbage-collected.
    pub skill_name: String,
    /// Number of versions removed.
    pub removed_count: usize,
    /// Number of versions retained.
    pub retained_count: usize,
    /// Hashes of removed versions.
    pub removed_hashes: Vec<String>,
}

impl SkillGarbageCollector {
    /// Create a garbage collector with the given keep count.
    pub fn new(keep_count: usize) -> Self {
        Self { keep_count }
    }

    /// Create a garbage collector with the default keep count (10).
    pub fn default_gc() -> Self {
        Self::new(10)
    }

    /// Identify which versions of a skill should be pruned.
    ///
    /// Returns hashes that can be safely removed (not HEAD, not in keep window).
    pub fn identify_prunable(&self, store: &VersionStore, skill_id: &str) -> Vec<String> {
        let history = store.history(skill_id);
        if history.len() <= self.keep_count {
            return vec![];
        }

        let active = store.active_version(skill_id);
        let active_hash = active.map(|v| v.0.as_str());

        // Keep the most recent `keep_count` versions and the active HEAD
        let mut keep: HashSet<String> = HashSet::new();
        if let Some(hash) = active_hash {
            keep.insert(hash.to_string());
        }

        // Keep the N most recent versions
        for version in history.iter().rev().take(self.keep_count) {
            keep.insert(version.0.clone());
        }

        // Everything else is prunable
        history
            .iter()
            .filter(|v| !keep.contains(&v.0))
            .map(|v| v.0.clone())
            .collect()
    }

    /// Prune unreferenced versions for a specific skill.
    ///
    /// Returns the GC result with details of what was removed.
    /// Note: this only identifies what *should* be pruned from the in-memory store.
    /// Actual filesystem cleanup would be handled by the persistent version store.
    pub fn prune(&self, store: &VersionStore, skill_id: &str) -> GcResult {
        let prunable = self.identify_prunable(store, skill_id);
        let total = store.history(skill_id).len();

        GcResult {
            skill_name: skill_id.to_string(),
            removed_count: prunable.len(),
            retained_count: total - prunable.len(),
            removed_hashes: prunable,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// T-SK-S3-07: GC removes unreferenced objects beyond `keep_count`.
    #[test]
    fn test_gc_removes_beyond_keep_count() {
        let mut store = VersionStore::new();
        let skill_id = "gc-test-skill";

        // Register 15 versions
        for i in 0..15 {
            store.register_version(skill_id, format!("content-v{i}").as_bytes());
        }

        assert_eq!(store.history(skill_id).len(), 15);

        let gc = SkillGarbageCollector::new(5);
        let result = gc.prune(&store, skill_id);

        assert_eq!(result.removed_count, 10);
        assert_eq!(result.retained_count, 5);
        assert_eq!(result.removed_hashes.len(), 10);
    }

    /// T-SK-S3-08: GC preserves HEAD and recent versions.
    #[test]
    fn test_gc_preserves_head_and_recent() {
        let mut store = VersionStore::new();
        let skill_id = "gc-preserve-test";

        let mut versions = vec![];
        for i in 0..8 {
            let v = store.register_version(skill_id, format!("content-v{i}").as_bytes());
            versions.push(v);
        }

        let gc = SkillGarbageCollector::new(3);
        let prunable = gc.identify_prunable(&store, skill_id);

        // HEAD (last version) should NOT be in prunable
        let head = store.active_version(skill_id).unwrap();
        assert!(!prunable.contains(&head.0), "HEAD should not be prunable");

        // Most recent 3 should NOT be prunable
        for v in versions.iter().rev().take(3) {
            assert!(
                !prunable.contains(&v.0),
                "recent version {} should not be prunable",
                v.0
            );
        }

        // Should prune 5 versions (8 - 3 kept)
        assert_eq!(prunable.len(), 5);
    }

    /// When history is shorter than `keep_count`, nothing is pruned.
    #[test]
    fn test_gc_nothing_to_prune() {
        let mut store = VersionStore::new();
        let skill_id = "small-skill";

        store.register_version(skill_id, b"v1");
        store.register_version(skill_id, b"v2");

        let gc = SkillGarbageCollector::new(5);
        let result = gc.prune(&store, skill_id);

        assert_eq!(result.removed_count, 0);
        assert_eq!(result.retained_count, 2);
    }

    /// GC default keeps 10 versions.
    #[test]
    fn test_gc_default_keep_count() {
        let gc = SkillGarbageCollector::default_gc();
        let mut store = VersionStore::new();
        let skill_id = "default-gc";

        for i in 0..12 {
            store.register_version(skill_id, format!("v{i}").as_bytes());
        }

        let result = gc.prune(&store, skill_id);
        assert_eq!(result.retained_count, 10);
        assert_eq!(result.removed_count, 2);
    }
}
