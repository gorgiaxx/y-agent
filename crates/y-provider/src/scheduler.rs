//! Priority scheduler for provider requests.
//!
//! Design reference: providers-design.md §Concurrency Model
//!
//! Implements a three-tier priority queue (Critical / Normal / Idle) that
//! ensures Critical requests always get through by reserving capacity.

use std::sync::atomic::{AtomicU64, Ordering};

use y_core::provider::RoutePriority;

/// Tracks per-priority request statistics and enforces scheduling rules.
///
/// The scheduler doesn't queue requests itself; instead, it provides
/// `should_admit` checks that the pool evaluates before routing.
#[derive(Debug)]
pub struct PriorityScheduler {
    /// Counter of currently active Critical requests.
    active_critical: AtomicU64,
    /// Counter of currently active Normal requests.
    active_normal: AtomicU64,
    /// Counter of currently active Idle requests.
    active_idle: AtomicU64,
    /// Total capacity across all providers.
    total_capacity: usize,
    /// Percentage of capacity reserved for Critical (0-100).
    critical_reserve_pct: u8,
}

impl PriorityScheduler {
    /// Create a new scheduler.
    ///
    /// # Arguments
    /// * `total_capacity` — Total concurrent request slots across all providers.
    /// * `critical_reserve_pct` — Percentage of capacity reserved for Critical
    ///   requests (default: 20).
    pub fn new(total_capacity: usize, critical_reserve_pct: u8) -> Self {
        Self {
            active_critical: AtomicU64::new(0),
            active_normal: AtomicU64::new(0),
            active_idle: AtomicU64::new(0),
            total_capacity,
            critical_reserve_pct: critical_reserve_pct.min(100),
        }
    }

    /// Check whether a request with the given priority should be admitted.
    ///
    /// Returns `true` if the request can proceed, `false` if it should be
    /// rejected or deferred.
    pub fn should_admit(&self, priority: RoutePriority) -> bool {
        let active_total = self.active_total();
        let reserved = self.reserved_capacity();

        match priority {
            RoutePriority::Critical => {
                // Critical requests can always be admitted up to total capacity.
                active_total < self.total_capacity as u64
            }
            RoutePriority::Normal => {
                // Normal requests cannot use reserved capacity.
                active_total + reserved < self.total_capacity as u64
            }
            RoutePriority::Idle => {
                // Idle requests only admitted when there's plenty of room
                // (below 50% usage and not eating into reserved).
                let half_cap = self.total_capacity as u64 / 2;
                active_total < half_cap && active_total + reserved < self.total_capacity as u64
            }
        }
    }

    /// Record that a request has started.
    pub fn record_start(&self, priority: RoutePriority) {
        match priority {
            RoutePriority::Critical => {
                self.active_critical.fetch_add(1, Ordering::Relaxed);
            }
            RoutePriority::Normal => {
                self.active_normal.fetch_add(1, Ordering::Relaxed);
            }
            RoutePriority::Idle => {
                self.active_idle.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Record that a request has completed.
    pub fn record_complete(&self, priority: RoutePriority) {
        match priority {
            RoutePriority::Critical => {
                self.active_critical.fetch_sub(1, Ordering::Relaxed);
            }
            RoutePriority::Normal => {
                self.active_normal.fetch_sub(1, Ordering::Relaxed);
            }
            RoutePriority::Idle => {
                self.active_idle.fetch_sub(1, Ordering::Relaxed);
            }
        }
    }

    /// Get the total number of active requests across all priorities.
    pub fn active_total(&self) -> u64 {
        self.active_critical.load(Ordering::Relaxed)
            + self.active_normal.load(Ordering::Relaxed)
            + self.active_idle.load(Ordering::Relaxed)
    }

    /// Get the number of slots reserved for Critical requests.
    fn reserved_capacity(&self) -> u64 {
        (self.total_capacity as u64 * u64::from(self.critical_reserve_pct)) / 100
    }

    /// Get a snapshot of the scheduler state.
    pub fn snapshot(&self) -> SchedulerSnapshot {
        SchedulerSnapshot {
            active_critical: self.active_critical.load(Ordering::Relaxed),
            active_normal: self.active_normal.load(Ordering::Relaxed),
            active_idle: self.active_idle.load(Ordering::Relaxed),
            total_capacity: self.total_capacity,
            critical_reserve_pct: self.critical_reserve_pct,
        }
    }
}

/// Immutable snapshot of scheduler state.
#[derive(Debug, Clone)]
pub struct SchedulerSnapshot {
    pub active_critical: u64,
    pub active_normal: u64,
    pub active_idle: u64,
    pub total_capacity: usize,
    pub critical_reserve_pct: u8,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_critical_always_admitted_when_capacity_available() {
        let scheduler = PriorityScheduler::new(10, 20);
        assert!(scheduler.should_admit(RoutePriority::Critical));

        // Fill up 9 slots.
        for _ in 0..9 {
            scheduler.record_start(RoutePriority::Normal);
        }
        // Critical should still be admitted (using the last slot).
        assert!(scheduler.should_admit(RoutePriority::Critical));
    }

    #[test]
    fn test_critical_rejected_at_full_capacity() {
        let scheduler = PriorityScheduler::new(2, 20);
        scheduler.record_start(RoutePriority::Critical);
        scheduler.record_start(RoutePriority::Normal);
        // At full capacity, even Critical is rejected.
        assert!(!scheduler.should_admit(RoutePriority::Critical));
    }

    #[test]
    fn test_normal_cannot_use_reserved_capacity() {
        let scheduler = PriorityScheduler::new(10, 20); // 2 reserved
                                                        // Fill 8 normal slots.
        for _ in 0..8 {
            scheduler.record_start(RoutePriority::Normal);
        }
        // Normal should be rejected (8 + 2 reserved = 10 total).
        assert!(!scheduler.should_admit(RoutePriority::Normal));
        // But Critical should still be admitted.
        assert!(scheduler.should_admit(RoutePriority::Critical));
    }

    #[test]
    fn test_idle_limited_to_half_capacity() {
        let scheduler = PriorityScheduler::new(10, 20);
        // Fill 4 idle slots.
        for _ in 0..4 {
            scheduler.record_start(RoutePriority::Idle);
        }
        // Idle should still be admitted (4 < 5 = half capacity).
        assert!(scheduler.should_admit(RoutePriority::Idle));

        // Fill to 5.
        scheduler.record_start(RoutePriority::Idle);
        // Idle should now be rejected (5 >= 5 = half capacity).
        assert!(!scheduler.should_admit(RoutePriority::Idle));
    }

    #[test]
    fn test_record_complete_frees_capacity() {
        let scheduler = PriorityScheduler::new(2, 20);
        scheduler.record_start(RoutePriority::Normal);
        scheduler.record_start(RoutePriority::Normal);
        assert!(!scheduler.should_admit(RoutePriority::Critical));

        scheduler.record_complete(RoutePriority::Normal);
        assert!(scheduler.should_admit(RoutePriority::Critical));
    }

    #[test]
    fn test_snapshot_reflects_state() {
        let scheduler = PriorityScheduler::new(20, 25);
        scheduler.record_start(RoutePriority::Critical);
        scheduler.record_start(RoutePriority::Normal);
        scheduler.record_start(RoutePriority::Normal);
        scheduler.record_start(RoutePriority::Idle);

        let snap = scheduler.snapshot();
        assert_eq!(snap.active_critical, 1);
        assert_eq!(snap.active_normal, 2);
        assert_eq!(snap.active_idle, 1);
        assert_eq!(snap.total_capacity, 20);
        assert_eq!(snap.critical_reserve_pct, 25);
    }

    #[test]
    fn test_zero_capacity() {
        let scheduler = PriorityScheduler::new(0, 20);
        assert!(!scheduler.should_admit(RoutePriority::Critical));
        assert!(!scheduler.should_admit(RoutePriority::Normal));
        assert!(!scheduler.should_admit(RoutePriority::Idle));
    }

    #[test]
    fn test_active_total() {
        let scheduler = PriorityScheduler::new(10, 20);
        assert_eq!(scheduler.active_total(), 0);

        scheduler.record_start(RoutePriority::Critical);
        scheduler.record_start(RoutePriority::Normal);
        scheduler.record_start(RoutePriority::Idle);
        assert_eq!(scheduler.active_total(), 3);

        scheduler.record_complete(RoutePriority::Normal);
        assert_eq!(scheduler.active_total(), 2);
    }
}
