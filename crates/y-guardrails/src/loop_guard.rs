//! `LoopGuard`: pattern detection for agent loop behavior.
//!
//! Detects 4 patterns:
//! - **Repetition**: Same action repeated N times consecutively
//! - **Oscillation**: A → B → A → B cyclic pattern
//! - **Drift**: No progress metric change over N steps
//! - **`RedundantToolCall`**: Same tool with same arguments called repeatedly

use std::collections::VecDeque;

use crate::config::LoopGuardConfig;
use crate::error::LoopPattern;

/// An action record for loop detection analysis.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ActionRecord {
    /// A fingerprint of the action (e.g., "`tool:read_file`" or "`llm_call`").
    pub action_key: String,
    /// Optional tool arguments hash for redundant-tool detection.
    pub args_hash: Option<String>,
}

/// Detection result from the `LoopGuard`.
#[derive(Debug, Clone)]
pub struct LoopDetection {
    /// The detected pattern type.
    pub pattern: LoopPattern,
    /// Human-readable description.
    pub details: String,
}

/// `LoopGuard` accumulates action history and checks for loop patterns.
#[derive(Debug)]
pub struct LoopGuard {
    config: LoopGuardConfig,
    /// Recent action history (bounded to max(all thresholds) * 2).
    history: VecDeque<ActionRecord>,
    /// Progress metric (monotonically increasing when progress is made).
    last_progress: f64,
    /// Steps since last progress change.
    steps_since_progress: usize,
}

impl LoopGuard {
    /// Create a new `LoopGuard` with the given config.
    pub fn new(config: LoopGuardConfig) -> Self {
        let max_history = (config
            .repetition_threshold
            .max(config.oscillation_threshold * 2)
            .max(config.drift_threshold)
            .max(config.redundant_threshold))
            * 2;

        Self {
            config,
            history: VecDeque::with_capacity(max_history),
            last_progress: 0.0,
            steps_since_progress: 0,
        }
    }

    /// Record an action and check for loop patterns.
    ///
    /// `progress_metric` is an optional monotonically-increasing value.
    /// If `None`, drift detection is not updated.
    pub fn record(
        &mut self,
        action: ActionRecord,
        progress_metric: Option<f64>,
    ) -> Option<LoopDetection> {
        if !self.config.enabled {
            return None;
        }

        // Update progress tracking
        if let Some(metric) = progress_metric {
            if metric > self.last_progress {
                self.last_progress = metric;
                self.steps_since_progress = 0;
                // Progress resets history for loop detection
                self.history.clear();
            } else {
                self.steps_since_progress += 1;
            }
        } else {
            self.steps_since_progress += 1;
        }

        // Add to history
        self.history.push_back(action);

        // Cap history size
        let max_history = (self
            .config
            .repetition_threshold
            .max(self.config.oscillation_threshold * 2)
            .max(self.config.drift_threshold)
            .max(self.config.redundant_threshold))
            * 2;
        while self.history.len() > max_history {
            self.history.pop_front();
        }

        // Check patterns in priority order
        if let Some(d) = self.check_redundant_tool() {
            return Some(d);
        }
        if let Some(d) = self.check_repetition() {
            return Some(d);
        }
        if let Some(d) = self.check_oscillation() {
            return Some(d);
        }
        self.check_drift()
    }

    /// Reset the guard (e.g., after progress is made externally).
    pub fn reset(&mut self) {
        self.history.clear();
        self.steps_since_progress = 0;
    }

    /// Check for Repetition: same `action_key` repeated N times at tail.
    fn check_repetition(&self) -> Option<LoopDetection> {
        let threshold = self.config.repetition_threshold;
        if self.history.len() < threshold {
            return None;
        }

        let tail = &self.history.as_slices().0;
        let full: Vec<_> = self.history.iter().collect();
        let last = full.last()?;

        let consecutive = full
            .iter()
            .rev()
            .take_while(|a| a.action_key == last.action_key)
            .count();

        if consecutive >= threshold {
            Some(LoopDetection {
                pattern: LoopPattern::Repetition,
                details: format!(
                    "action `{}` repeated {} times (threshold: {})",
                    last.action_key, consecutive, threshold
                ),
            })
        } else {
            // Also check via slices for correctness
            let _ = tail;
            None
        }
    }

    /// Check for Oscillation: A → B → A → B pattern.
    fn check_oscillation(&self) -> Option<LoopDetection> {
        let threshold = self.config.oscillation_threshold;
        let needed = threshold * 2; // Each cycle is 2 steps
        if self.history.len() < needed {
            return None;
        }

        let full: Vec<_> = self.history.iter().collect();
        let tail = &full[full.len() - needed..];

        // Check if tail alternates between exactly 2 actions
        let a = &tail[0].action_key;
        let b = &tail[1].action_key;

        if a == b {
            return None; // Not oscillation if first two are same
        }

        let is_oscillation = tail.iter().enumerate().all(|(i, action)| {
            if i % 2 == 0 {
                &action.action_key == a
            } else {
                &action.action_key == b
            }
        });

        if is_oscillation {
            Some(LoopDetection {
                pattern: LoopPattern::Oscillation,
                details: format!("oscillation between `{a}` and `{b}` for {threshold} cycles"),
            })
        } else {
            None
        }
    }

    /// Check for Drift: no progress for N steps.
    fn check_drift(&self) -> Option<LoopDetection> {
        if self.steps_since_progress >= self.config.drift_threshold {
            Some(LoopDetection {
                pattern: LoopPattern::Drift,
                details: format!(
                    "no progress for {} steps (threshold: {})",
                    self.steps_since_progress, self.config.drift_threshold
                ),
            })
        } else {
            None
        }
    }

    /// Check for `RedundantToolCall`: same tool + same args N times at tail.
    fn check_redundant_tool(&self) -> Option<LoopDetection> {
        let threshold = self.config.redundant_threshold;
        if self.history.len() < threshold {
            return None;
        }

        let full: Vec<_> = self.history.iter().collect();
        let last = full.last()?;

        // Only applies when args_hash is present
        let last_hash = last.args_hash.as_ref()?;

        let consecutive = full
            .iter()
            .rev()
            .take_while(|a| {
                a.action_key == last.action_key && a.args_hash.as_ref() == Some(last_hash)
            })
            .count();

        if consecutive >= threshold {
            Some(LoopDetection {
                pattern: LoopPattern::RedundantToolCall,
                details: format!(
                    "tool `{}` called with same args {} times (threshold: {})",
                    last.action_key, consecutive, threshold
                ),
            })
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_config() -> LoopGuardConfig {
        LoopGuardConfig::default()
    }

    fn action(key: &str) -> ActionRecord {
        ActionRecord {
            action_key: key.to_string(),
            args_hash: None,
        }
    }

    fn tool_action(key: &str, args: &str) -> ActionRecord {
        ActionRecord {
            action_key: key.to_string(),
            args_hash: Some(args.to_string()),
        }
    }

    /// T-GUARD-002-01: Same action 5 times → Repetition detected.
    #[test]
    fn test_loop_detect_repetition() {
        let mut guard = LoopGuard::new(default_config());

        for i in 0..5 {
            let result = guard.record(action("do_thing"), None);
            if i < 4 {
                // Should not detect in first 4
                assert!(
                    result.is_none() || result.as_ref().unwrap().pattern != LoopPattern::Repetition,
                    "unexpected early detection at step {i}"
                );
            } else {
                let detection = result.expect("should detect at step 5");
                assert_eq!(detection.pattern, LoopPattern::Repetition);
            }
        }
    }

    /// T-GUARD-002-02: A → B → A → B → A → B → Oscillation detected.
    #[test]
    fn test_loop_detect_oscillation() {
        let mut guard = LoopGuard::new(default_config()); // oscillation_threshold = 3

        let actions = ["A", "B", "A", "B", "A", "B"];
        let mut detected = None;
        for a in &actions {
            let result = guard.record(action(a), None);
            if let Some(d) = result {
                if d.pattern == LoopPattern::Oscillation {
                    detected = Some(d);
                }
            }
        }

        assert!(detected.is_some(), "oscillation should be detected");
        assert_eq!(detected.unwrap().pattern, LoopPattern::Oscillation);
    }

    /// T-GUARD-002-03: 10 steps with no progress → Drift detected.
    #[test]
    fn test_loop_detect_drift() {
        let mut guard = LoopGuard::new(default_config()); // drift_threshold = 10

        let mut detected = None;
        for i in 0..10 {
            let result = guard.record(action(&format!("step_{i}")), Some(0.0));
            if let Some(d) = result {
                if d.pattern == LoopPattern::Drift {
                    detected = Some(d);
                }
            }
        }

        assert!(detected.is_some(), "drift should be detected");
        assert_eq!(detected.unwrap().pattern, LoopPattern::Drift);
    }

    /// T-GUARD-002-04: Same tool, same args, 3 times → `RedundantToolCall`.
    #[test]
    fn test_loop_detect_redundant_tool() {
        let mut guard = LoopGuard::new(default_config()); // redundant_threshold = 3

        let mut detected = None;
        for _ in 0..3 {
            let result = guard.record(tool_action("read_file", "hash_abc"), None);
            if let Some(d) = result {
                if d.pattern == LoopPattern::RedundantToolCall {
                    detected = Some(d);
                }
            }
        }

        assert!(detected.is_some(), "redundant tool call should be detected");
        assert_eq!(detected.unwrap().pattern, LoopPattern::RedundantToolCall);
    }

    /// T-GUARD-002-05: Varied actions with clear progress → no false positive.
    #[test]
    fn test_loop_no_false_positive() {
        let mut guard = LoopGuard::new(default_config());

        for i in 0..20 {
            let result = guard.record(action(&format!("action_{}", i % 7)), Some(f64::from(i)));
            assert!(result.is_none(), "false positive at step {i}");
        }
    }

    /// T-GUARD-002-06: Custom repetition threshold of 3 detects at 3.
    #[test]
    fn test_loop_configurable_threshold() {
        let config = LoopGuardConfig {
            repetition_threshold: 3,
            ..Default::default()
        };
        let mut guard = LoopGuard::new(config);

        for i in 0..3 {
            let result = guard.record(action("repeat"), None);
            if i < 2 {
                assert!(result.is_none(), "unexpected early detection at step {i}");
            } else {
                let detection = result.expect("should detect at step 3");
                assert_eq!(detection.pattern, LoopPattern::Repetition);
            }
        }
    }

    /// T-GUARD-002-07: Progress resets detection window.
    #[test]
    fn test_loop_detection_resets_on_progress() {
        let config = LoopGuardConfig {
            repetition_threshold: 5,
            ..Default::default()
        };
        let mut guard = LoopGuard::new(config);

        // 3 repeats
        for _ in 0..3 {
            let result = guard.record(action("repeat"), None);
            assert!(result.is_none());
        }

        // Progress! This should reset.
        guard.record(action("progress"), Some(10.0));

        // 3 more repeats — should NOT trigger (only 3, not 5)
        for _ in 0..3 {
            let result = guard.record(action("repeat"), None);
            assert!(result.is_none(), "should not detect after progress reset");
        }
    }
}
