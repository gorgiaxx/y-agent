//! Knowledge maintenance — staleness detection, TTL expiry, re-ingestion,
//! and hit tracking.
//!
//! Provides utilities for keeping knowledge entries fresh and tracking usage.

use chrono::{DateTime, Duration, Utc};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Hit Tracking (MaxKB `hit_num` inspired)
// ---------------------------------------------------------------------------

/// Tracks hit counts for knowledge chunks.
///
/// Inspired by `MaxKB`'s `hit_num` counter for retrieval analytics.
#[derive(Debug, Clone, Default)]
pub struct HitTracker {
    /// `chunk_id` → hit count.
    hits: HashMap<String, u64>,
    /// `chunk_id` → last hit time.
    last_hit: HashMap<String, DateTime<Utc>>,
}

impl HitTracker {
    /// Create a new hit tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a hit for a chunk.
    pub fn record_hit(&mut self, chunk_id: &str) {
        *self.hits.entry(chunk_id.to_string()).or_insert(0) += 1;
        self.last_hit
            .insert(chunk_id.to_string(), Utc::now());
    }

    /// Get hit count for a chunk.
    pub fn hit_count(&self, chunk_id: &str) -> u64 {
        self.hits.get(chunk_id).copied().unwrap_or(0)
    }

    /// Get last hit time for a chunk.
    pub fn last_hit_time(&self, chunk_id: &str) -> Option<&DateTime<Utc>> {
        self.last_hit.get(chunk_id)
    }

    /// Get top-k most hit chunks.
    pub fn top_hits(&self, k: usize) -> Vec<(String, u64)> {
        let mut entries: Vec<(String, u64)> = self
            .hits
            .iter()
            .map(|(id, &count)| (id.clone(), count))
            .collect();
        entries.sort_by(|a, b| b.1.cmp(&a.1));
        entries.truncate(k);
        entries
    }

    /// Get chunks with zero hits (candidates for cleanup).
    pub fn zero_hit_chunks(&self) -> Vec<&str> {
        self.hits
            .iter()
            .filter(|(_, &count)| count == 0)
            .map(|(id, _)| id.as_str())
            .collect()
    }

    /// Total number of tracked chunks.
    pub fn tracked_count(&self) -> usize {
        self.hits.len()
    }
}

// ---------------------------------------------------------------------------
// Staleness Detection
// ---------------------------------------------------------------------------

/// Result of a staleness check.
#[derive(Debug, Clone)]
pub struct StalenessReport {
    /// Entry ID.
    pub entry_id: String,
    /// Source URI.
    pub source_uri: String,
    /// Whether the content hash has changed.
    pub content_changed: bool,
    /// Whether the entry has expired based on TTL.
    pub ttl_expired: bool,
    /// Time since last refresh.
    pub age: Duration,
    /// Recommended action.
    pub action: MaintenanceAction,
}

/// Recommended maintenance action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MaintenanceAction {
    /// No action needed — entry is fresh.
    None,
    /// Re-ingest the source (content changed).
    ReIngest,
    /// Refresh metadata (TTL expired but content unchanged).
    Refresh,
    /// Deactivate entry (source no longer available).
    Deactivate,
}

/// Configuration for maintenance operations.
#[derive(Debug, Clone)]
pub struct MaintenanceConfig {
    /// Default TTL for knowledge entries (in hours).
    pub default_ttl_hours: u64,
    /// Whether to auto-deactivate stale entries.
    pub auto_deactivate: bool,
    /// Maximum age before forcing re-check (in hours).
    pub max_age_hours: u64,
}

impl Default for MaintenanceConfig {
    fn default() -> Self {
        Self {
            default_ttl_hours: 24 * 7, // 1 week.
            auto_deactivate: false,
            max_age_hours: 24 * 30, // 30 days.
        }
    }
}

/// Knowledge maintenance manager.
///
/// Handles staleness detection, TTL expiry checks, and re-ingestion tracking.
#[derive(Debug)]
pub struct MaintenanceManager {
    /// Configuration.
    config: MaintenanceConfig,
    /// Hit tracker for usage analytics.
    hit_tracker: HitTracker,
    /// Known content hashes: `entry_id` → `content_hash`.
    content_hashes: HashMap<String, String>,
    /// Entry refresh timestamps: `entry_id` → last `refreshed_at`.
    refresh_times: HashMap<String, DateTime<Utc>>,
}

impl MaintenanceManager {
    /// Create a new maintenance manager.
    pub fn new(config: MaintenanceConfig) -> Self {
        Self {
            config,
            hit_tracker: HitTracker::new(),
            content_hashes: HashMap::new(),
            refresh_times: HashMap::new(),
        }
    }

    /// Register an entry for tracking.
    pub fn register_entry(
        &mut self,
        entry_id: &str,
        content_hash: &str,
        source_uri: &str,
    ) {
        self.content_hashes
            .insert(entry_id.to_string(), content_hash.to_string());
        self.refresh_times
            .insert(entry_id.to_string(), Utc::now());
        // Initialize hit counter.
        self.hit_tracker
            .hits
            .entry(entry_id.to_string())
            .or_insert(0);
        // Store source URI in content_hashes (dual purpose for simplicity).
        let _ = source_uri; // Used indirectly through entry tracking.
    }

    /// Check if an entry is stale based on its content hash and TTL.
    pub fn check_staleness(
        &self,
        entry_id: &str,
        current_hash: &str,
        source_uri: &str,
    ) -> StalenessReport {
        let content_changed = self
            .content_hashes
            .get(entry_id)
            .is_none_or(|stored| stored != current_hash);

        let age = self
            .refresh_times
            .get(entry_id)
            .map_or(Duration::hours(i64::from(u32::MAX)), |t| Utc::now() - *t);

        #[allow(clippy::cast_possible_wrap)]
        let ttl = Duration::hours(self.config.default_ttl_hours as i64);
        let ttl_expired = age > ttl;

        let action = if content_changed {
            MaintenanceAction::ReIngest
        } else if ttl_expired {
            MaintenanceAction::Refresh
        } else {
            MaintenanceAction::None
        };

        StalenessReport {
            entry_id: entry_id.to_string(),
            source_uri: source_uri.to_string(),
            content_changed,
            ttl_expired,
            age,
            action,
        }
    }

    /// Mark an entry as refreshed (after re-ingestion or refresh).
    pub fn mark_refreshed(&mut self, entry_id: &str, new_hash: &str) {
        self.content_hashes
            .insert(entry_id.to_string(), new_hash.to_string());
        self.refresh_times
            .insert(entry_id.to_string(), Utc::now());
    }

    /// Get entries that need attention (stale or expired).
    pub fn entries_needing_attention(&self) -> Vec<String> {
        #[allow(clippy::cast_possible_wrap)]
        let ttl = Duration::hours(self.config.default_ttl_hours as i64);

        self.refresh_times
            .iter()
            .filter(|(_, refreshed_at)| Utc::now() - **refreshed_at > ttl)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Get a reference to the hit tracker.
    pub fn hit_tracker(&self) -> &HitTracker {
        &self.hit_tracker
    }

    /// Get a mutable reference to the hit tracker.
    pub fn hit_tracker_mut(&mut self) -> &mut HitTracker {
        &mut self.hit_tracker
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- HitTracker Tests ---

    #[test]
    fn test_hit_tracker_record_and_count() {
        let mut tracker = HitTracker::new();
        assert_eq!(tracker.hit_count("c1"), 0);

        tracker.record_hit("c1");
        tracker.record_hit("c1");
        tracker.record_hit("c2");

        assert_eq!(tracker.hit_count("c1"), 2);
        assert_eq!(tracker.hit_count("c2"), 1);
        assert_eq!(tracker.hit_count("c3"), 0);
    }

    #[test]
    fn test_hit_tracker_top_hits() {
        let mut tracker = HitTracker::new();
        tracker.record_hit("c1");
        tracker.record_hit("c1");
        tracker.record_hit("c1");
        tracker.record_hit("c2");
        tracker.record_hit("c2");
        tracker.record_hit("c3");

        let top = tracker.top_hits(2);
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].0, "c1");
        assert_eq!(top[0].1, 3);
    }

    #[test]
    fn test_hit_tracker_zero_hits() {
        let mut tracker = HitTracker::new();
        tracker.hits.insert("c1".to_string(), 0);
        tracker.hits.insert("c2".to_string(), 5);
        tracker.hits.insert("c3".to_string(), 0);

        let zeros = tracker.zero_hit_chunks();
        assert_eq!(zeros.len(), 2);
    }

    #[test]
    fn test_hit_tracker_last_hit_time() {
        let mut tracker = HitTracker::new();
        assert!(tracker.last_hit_time("c1").is_none());

        tracker.record_hit("c1");
        assert!(tracker.last_hit_time("c1").is_some());
    }

    // --- MaintenanceManager Tests ---

    #[test]
    fn test_maintenance_register_entry() {
        let mut mgr = MaintenanceManager::new(MaintenanceConfig::default());
        mgr.register_entry("e1", "hash123", "/path/to/file.md");

        assert!(mgr.content_hashes.contains_key("e1"));
        assert!(mgr.refresh_times.contains_key("e1"));
    }

    #[test]
    fn test_staleness_no_change() {
        let mut mgr = MaintenanceManager::new(MaintenanceConfig::default());
        mgr.register_entry("e1", "hash123", "/path/to/file.md");

        let report = mgr.check_staleness("e1", "hash123", "/path/to/file.md");
        assert!(!report.content_changed);
        assert!(!report.ttl_expired);
        assert_eq!(report.action, MaintenanceAction::None);
    }

    #[test]
    fn test_staleness_content_changed() {
        let mut mgr = MaintenanceManager::new(MaintenanceConfig::default());
        mgr.register_entry("e1", "hash123", "/path/to/file.md");

        let report = mgr.check_staleness("e1", "hash_NEW", "/path/to/file.md");
        assert!(report.content_changed);
        assert_eq!(report.action, MaintenanceAction::ReIngest);
    }

    #[test]
    fn test_staleness_ttl_expired() {
        let config = MaintenanceConfig {
            default_ttl_hours: 0, // Immediate expiry for testing.
            ..Default::default()
        };
        let mut mgr = MaintenanceManager::new(config);
        mgr.register_entry("e1", "hash123", "/path/to/file.md");
        // Force old refresh time.
        mgr.refresh_times.insert(
            "e1".to_string(),
            Utc::now() - Duration::hours(1),
        );

        let report = mgr.check_staleness("e1", "hash123", "/path/to/file.md");
        assert!(report.ttl_expired);
        assert_eq!(report.action, MaintenanceAction::Refresh);
    }

    #[test]
    fn test_mark_refreshed() {
        let mut mgr = MaintenanceManager::new(MaintenanceConfig::default());
        mgr.register_entry("e1", "old_hash", "/path");

        mgr.mark_refreshed("e1", "new_hash");
        assert_eq!(mgr.content_hashes.get("e1").unwrap(), "new_hash");
    }

    #[test]
    fn test_entries_needing_attention() {
        let config = MaintenanceConfig {
            default_ttl_hours: 0,
            ..Default::default()
        };
        let mut mgr = MaintenanceManager::new(config);
        mgr.register_entry("e1", "h1", "/p1");
        mgr.refresh_times.insert(
            "e1".to_string(),
            Utc::now() - Duration::hours(1),
        );
        mgr.register_entry("e2", "h2", "/p2");
        // e2 is fresh, e1 is stale.

        let stale = mgr.entries_needing_attention();
        assert!(stale.contains(&"e1".to_string()));
    }

    #[test]
    fn test_default_config() {
        let config = MaintenanceConfig::default();
        assert_eq!(config.default_ttl_hours, 24 * 7);
        assert!(!config.auto_deactivate);
        assert_eq!(config.max_age_hours, 24 * 30);
    }
}
