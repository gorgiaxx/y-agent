//! Regression detection: monitors skill performance after version changes.
//!
//! Compares baseline metrics from the previous version against new version
//! metrics, triggering alerts and rollback proposals when regressions are
//! detected.

use crate::evolution::{ChangeType, EvolutionProposal, PatternType, ProposalStatus, SkillMetrics};

/// Thresholds for regression detection.
#[derive(Debug, Clone)]
pub struct RegressionThresholds {
    /// Maximum allowed drop in success rate (fraction, e.g., 0.15 = 15%).
    pub max_success_rate_drop: f64,
    /// Maximum allowed increase in failure rate (fraction, e.g., 0.10 = 10%).
    pub max_failure_rate_increase: f64,
}

impl Default for RegressionThresholds {
    fn default() -> Self {
        Self {
            max_success_rate_drop: 0.15,
            max_failure_rate_increase: 0.10,
        }
    }
}

/// Result of regression analysis.
#[derive(Debug, Clone)]
pub enum RegressionResult {
    /// No regression detected.
    NoRegression,
    /// Regression detected with details.
    RegressionDetected {
        /// What triggered the regression.
        reason: String,
        /// The success rate drop (positive = worse).
        success_rate_drop: f64,
        /// The failure rate increase (positive = worse).
        failure_rate_increase: f64,
    },
}

impl RegressionResult {
    /// Returns true if a regression was detected.
    pub fn is_regression(&self) -> bool {
        matches!(self, Self::RegressionDetected { .. })
    }
}

/// Detects regressions after skill version changes.
#[derive(Debug)]
pub struct RegressionDetector {
    thresholds: RegressionThresholds,
}

impl RegressionDetector {
    /// Create a detector with default thresholds (15% success drop, 10% failure increase).
    pub fn new() -> Self {
        Self {
            thresholds: RegressionThresholds::default(),
        }
    }

    /// Create a detector with custom thresholds.
    pub fn with_thresholds(thresholds: RegressionThresholds) -> Self {
        Self { thresholds }
    }

    /// Compare new version metrics against baseline metrics.
    pub fn check(&self, baseline: &SkillMetrics, current: &SkillMetrics) -> RegressionResult {
        // Need sufficient data
        if baseline.use_count < 5 || current.use_count < 5 {
            return RegressionResult::NoRegression;
        }

        let success_rate_drop = baseline.success_rate() - current.success_rate();
        let failure_rate_increase = current.failure_rate() - baseline.failure_rate();

        let success_regression = success_rate_drop > self.thresholds.max_success_rate_drop;
        let failure_regression = failure_rate_increase > self.thresholds.max_failure_rate_increase;

        if success_regression || failure_regression {
            let mut reasons = Vec::new();
            if success_regression {
                reasons.push(format!(
                    "success rate dropped {:.1}% (threshold: {:.1}%)",
                    success_rate_drop * 100.0,
                    self.thresholds.max_success_rate_drop * 100.0
                ));
            }
            if failure_regression {
                reasons.push(format!(
                    "failure rate increased {:.1}% (threshold: {:.1}%)",
                    failure_rate_increase * 100.0,
                    self.thresholds.max_failure_rate_increase * 100.0
                ));
            }

            RegressionResult::RegressionDetected {
                reason: reasons.join("; "),
                success_rate_drop,
                failure_rate_increase,
            }
        } else {
            RegressionResult::NoRegression
        }
    }

    /// Generate a rollback proposal when regression is detected.
    pub fn rollback_proposal(
        &self,
        skill_name: &str,
        current_version: &str,
        previous_version: &str,
        regression: &RegressionResult,
    ) -> Option<EvolutionProposal> {
        if let RegressionResult::RegressionDetected { reason, .. } = regression {
            Some(EvolutionProposal {
                id: format!("rollback-{}", uuid::Uuid::new_v4()),
                skill_name: skill_name.to_string(),
                current_version: current_version.to_string(),
                proposed_changes: format!("Rollback to version {previous_version}: {reason}"),
                patterns: vec![PatternType::CommonError],
                status: ProposalStatus::PendingApproval,
                proposed_version: Some(previous_version.to_string()),
                change_type: Some(ChangeType::ErrorWarning),
                patterns_referenced: vec![],
                diff_preview: String::new(),
                deferred_until: None,
            })
        } else {
            None
        }
    }
}

impl Default for RegressionDetector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_metrics(successes: u64, failures: u64) -> SkillMetrics {
        SkillMetrics {
            use_count: successes + failures,
            success_count: successes,
            partial_count: 0,
            failure_count: failures,
            avg_duration_ms: 1000.0,
            user_feedback_score: None,
            avg_token_usage: 500.0,
            injection_count: 0,
            actual_usage_count: 0,
        }
    }

    /// T-SK-S6-07: Regression detected when success_rate drops >15%.
    #[test]
    fn test_regression_success_rate_drop() {
        let detector = RegressionDetector::new();

        let baseline = make_metrics(18, 2); // 90% success
        let current = make_metrics(14, 6); // 70% success → 20% drop

        let result = detector.check(&baseline, &current);
        assert!(result.is_regression());

        if let RegressionResult::RegressionDetected {
            success_rate_drop, ..
        } = &result
        {
            assert!(*success_rate_drop > 0.15);
        }
    }

    /// No regression when metrics are stable.
    #[test]
    fn test_no_regression_stable() {
        let detector = RegressionDetector::new();

        let baseline = make_metrics(18, 2); // 90%
        let current = make_metrics(17, 3); // 85% → only 5% drop

        let result = detector.check(&baseline, &current);
        assert!(!result.is_regression());
    }

    /// T-SK-S6-08: Regression generates rollback proposal.
    #[test]
    fn test_regression_generates_rollback() {
        let detector = RegressionDetector::new();

        let baseline = make_metrics(18, 2);
        let current = make_metrics(14, 6);

        let result = detector.check(&baseline, &current);
        assert!(result.is_regression());

        let proposal = detector.rollback_proposal("my-skill", "v2", "v1", &result);
        assert!(proposal.is_some());
        let p = proposal.unwrap();
        assert_eq!(p.skill_name, "my-skill");
        assert_eq!(p.proposed_version, Some("v1".to_string()));
        assert!(p.proposed_changes.contains("Rollback"));
    }

    /// Insufficient data produces no regression.
    #[test]
    fn test_regression_insufficient_data() {
        let detector = RegressionDetector::new();

        let baseline = make_metrics(3, 1); // only 4 uses
        let current = make_metrics(1, 3); // only 4 uses

        let result = detector.check(&baseline, &current);
        assert!(!result.is_regression());
    }

    /// No rollback proposal when no regression.
    #[test]
    fn test_no_rollback_without_regression() {
        let detector = RegressionDetector::new();
        let result = RegressionResult::NoRegression;
        let proposal = detector.rollback_proposal("x", "v2", "v1", &result);
        assert!(proposal.is_none());
    }
}
