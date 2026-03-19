//! Freeze/thaw manager with adaptive durations.

use std::sync::RwLock;
use std::time::{Duration, Instant};

/// Tracks freeze state and adaptive backoff for a single provider.
///
/// Freeze durations increase exponentially on repeated transient failures,
/// capped at a configurable maximum. Permanent errors (auth, quota) cause
/// indefinite freeze (no automatic thaw).
#[derive(Debug)]
pub struct FreezeManager {
    state: RwLock<FreezeState>,
    base_duration_secs: u64,
    max_duration_secs: u64,
}

#[derive(Debug)]
struct FreezeState {
    frozen: bool,
    reason: Option<String>,
    frozen_at: Option<Instant>,
    thaw_at: Option<Instant>,
    /// Number of consecutive freezes for exponential backoff.
    consecutive_freezes: u32,
    /// Permanent freeze (e.g., auth failure) — no auto-thaw.
    permanent: bool,
}

impl FreezeManager {
    /// Create a new freeze manager.
    pub fn new(base_duration_secs: u64, max_duration_secs: u64) -> Self {
        Self {
            state: RwLock::new(FreezeState {
                frozen: false,
                reason: None,
                frozen_at: None,
                thaw_at: None,
                consecutive_freezes: 0,
                permanent: false,
            }),
            base_duration_secs,
            max_duration_secs,
        }
    }

    /// Check if the provider is currently frozen.
    ///
    /// # Panics
    ///
    /// Panics if the internal state lock is poisoned.
    pub fn is_frozen(&self) -> bool {
        let state = self.state.read().expect("lock poisoned");

        if !state.frozen {
            return false;
        }

        // Check if thaw time has passed (for non-permanent freezes).
        if let Some(thaw_at) = state.thaw_at {
            if Instant::now() >= thaw_at {
                // Time to thaw — but we need a write lock for that.
                // For now, report as still frozen; thaw will happen on next action.
                drop(state);
                return self.try_auto_thaw();
            }
        }

        true
    }

    /// Freeze the provider with a reason and optional explicit duration.
    ///
    /// If duration is `None`, the adaptive duration is calculated based
    /// on consecutive freezes.
    ///
    /// # Panics
    ///
    /// Panics if the internal state lock is poisoned.
    pub fn freeze(&self, reason: String, duration: Option<Duration>) {
        let mut state = self.state.write().expect("lock poisoned");
        state.frozen = true;
        state.reason = Some(reason);
        state.frozen_at = Some(Instant::now());
        state.consecutive_freezes += 1;

        if let Some(d) = duration {
            state.thaw_at = Some(Instant::now() + d);
        } else {
            let adaptive = self.adaptive_duration(state.consecutive_freezes);
            state.thaw_at = Some(Instant::now() + adaptive);
        }

        state.permanent = false;
    }

    /// Freeze permanently (e.g., authentication failure).
    ///
    /// # Panics
    ///
    /// Panics if the internal state lock is poisoned.
    pub fn freeze_permanent(&self, reason: String) {
        let mut state = self.state.write().expect("lock poisoned");
        state.frozen = true;
        state.reason = Some(reason);
        state.frozen_at = Some(Instant::now());
        state.thaw_at = None;
        state.permanent = true;
    }

    /// Manually thaw the provider.
    ///
    /// # Panics
    ///
    /// Panics if the internal state lock is poisoned.
    pub fn thaw(&self) {
        let mut state = self.state.write().expect("lock poisoned");
        state.frozen = false;
        state.reason = None;
        state.frozen_at = None;
        state.thaw_at = None;
        state.consecutive_freezes = 0;
        state.permanent = false;
    }

    /// Freeze with a specific retry-after duration (from rate limit headers).
    pub fn freeze_with_retry_after(&self, reason: String, retry_after: Duration) {
        self.freeze(reason, Some(retry_after));
    }

    /// Get freeze status information.
    ///
    /// # Panics
    ///
    /// Panics if the internal state lock is poisoned.
    pub fn status(&self) -> FreezeStatus {
        let state = self.state.read().expect("lock poisoned");
        FreezeStatus {
            is_frozen: state.frozen,
            reason: state.reason.clone(),
            frozen_since: state.frozen_at,
            thaw_at: state.thaw_at,
            consecutive_freezes: state.consecutive_freezes,
            permanent: state.permanent,
        }
    }

    /// Calculate adaptive freeze duration with exponential backoff.
    fn adaptive_duration(&self, consecutive: u32) -> Duration {
        let multiplier = 2u64.saturating_pow(consecutive.saturating_sub(1));
        let secs = (self.base_duration_secs * multiplier).min(self.max_duration_secs);
        Duration::from_secs(secs)
    }

    /// Try to auto-thaw if the freeze duration has expired.
    /// Returns `false` if thawed, `true` if still frozen.
    fn try_auto_thaw(&self) -> bool {
        let mut state = self.state.write().expect("lock poisoned");
        if state.permanent {
            return true;
        }
        if let Some(thaw_at) = state.thaw_at {
            if Instant::now() >= thaw_at {
                state.frozen = false;
                state.reason = None;
                state.frozen_at = None;
                state.thaw_at = None;
                // Don't reset consecutive_freezes — that happens on manual thaw.
                return false;
            }
        }
        true
    }
}

/// Snapshot of freeze status.
#[derive(Debug, Clone)]
pub struct FreezeStatus {
    pub is_frozen: bool,
    pub reason: Option<String>,
    pub frozen_since: Option<Instant>,
    pub thaw_at: Option<Instant>,
    pub consecutive_freezes: u32,
    pub permanent: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_freeze_on_rate_limit() {
        let fm = FreezeManager::new(30, 3600);
        fm.freeze_with_retry_after("rate limited".into(), Duration::from_secs(60));

        assert!(fm.is_frozen());
        let status = fm.status();
        assert!(status.is_frozen);
        assert_eq!(status.reason, Some("rate limited".into()));
        assert!(status.thaw_at.is_some());
    }

    #[test]
    fn test_freeze_on_auth_failure() {
        let fm = FreezeManager::new(30, 3600);
        fm.freeze_permanent("auth failed".into());

        assert!(fm.is_frozen());
        let status = fm.status();
        assert!(status.permanent);
        assert!(status.thaw_at.is_none());
    }

    #[test]
    fn test_freeze_duration_adaptive() {
        let fm = FreezeManager::new(30, 3600);

        // First freeze: 30s
        let d1 = fm.adaptive_duration(1);
        assert_eq!(d1, Duration::from_secs(30));

        // Second: 60s
        let d2 = fm.adaptive_duration(2);
        assert_eq!(d2, Duration::from_secs(60));

        // Third: 120s
        let d3 = fm.adaptive_duration(3);
        assert_eq!(d3, Duration::from_secs(120));

        // Eventually capped at max
        let d10 = fm.adaptive_duration(10);
        assert_eq!(d10, Duration::from_secs(3600));
    }

    #[test]
    fn test_thaw_after_duration() {
        let fm = FreezeManager::new(30, 3600);
        // Freeze for 0 seconds (should auto-thaw immediately).
        fm.freeze("test".into(), Some(Duration::from_secs(0)));

        // Give a tiny bit of time for the instant to pass.
        std::thread::sleep(Duration::from_millis(1));
        assert!(!fm.is_frozen(), "should auto-thaw after duration");
    }

    #[test]
    fn test_manual_freeze() {
        let fm = FreezeManager::new(30, 3600);
        fm.freeze("manual freeze".into(), None);

        assert!(fm.is_frozen());
        let status = fm.status();
        assert_eq!(status.reason, Some("manual freeze".into()));
    }

    #[test]
    fn test_manual_thaw() {
        let fm = FreezeManager::new(30, 3600);
        fm.freeze("test".into(), None);
        assert!(fm.is_frozen());

        fm.thaw();
        assert!(!fm.is_frozen());
        let status = fm.status();
        assert_eq!(status.consecutive_freezes, 0);
    }

    #[test]
    fn test_freeze_status_reporting() {
        let fm = FreezeManager::new(30, 3600);
        let status = fm.status();
        assert!(!status.is_frozen);
        assert!(status.reason.is_none());

        fm.freeze("test".into(), None);
        let status = fm.status();
        assert!(status.is_frozen);
        assert!(status.frozen_since.is_some());
        assert!(status.thaw_at.is_some());
    }

    #[test]
    fn test_consecutive_freezes_tracked() {
        let fm = FreezeManager::new(30, 3600);
        fm.freeze("first".into(), Some(Duration::from_secs(0)));
        std::thread::sleep(Duration::from_millis(1));
        fm.freeze("second".into(), Some(Duration::from_secs(0)));

        let status = fm.status();
        assert_eq!(status.consecutive_freezes, 2);
    }
}
