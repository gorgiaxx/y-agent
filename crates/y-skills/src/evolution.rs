//! Skill evolution: proposals and metrics.
//!
//! Standard reference: `docs/standards/SKILLS_STANDARD.md`
//!
//! After accumulating enough experience, the system can propose changes
//! to skill documents. This module defines the proposal and metrics types
//! consumed by the regression detector.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Pattern types
// ---------------------------------------------------------------------------

/// Types of patterns extracted from experience records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatternType {
    /// An edge case not covered by current instructions.
    EdgeCase,
    /// A commonly occurring error pattern.
    CommonError,
    /// A better way to phrase an existing instruction.
    BetterPhrasing,
    /// A new capability that could be added.
    NewCapability,
    /// A rule that is no longer relevant.
    ObsoleteRule,
    /// A recurring workflow that could become a sub-skill.
    WorkflowDiscovery,
}

impl std::fmt::Display for PatternType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::EdgeCase => "edge_case",
            Self::CommonError => "common_error",
            Self::BetterPhrasing => "better_phrasing",
            Self::NewCapability => "new_capability",
            Self::ObsoleteRule => "obsolete_rule",
            Self::WorkflowDiscovery => "workflow_discovery",
        };
        f.write_str(s)
    }
}

// ---------------------------------------------------------------------------
// Skill metrics
// ---------------------------------------------------------------------------

/// Aggregated performance metrics for a skill.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillMetrics {
    /// Total number of times this skill was invoked.
    pub use_count: u64,
    /// Number of successful executions.
    pub success_count: u64,
    /// Number of partial completions.
    pub partial_count: u64,
    /// Number of failures.
    pub failure_count: u64,
    /// Average execution duration in milliseconds.
    pub avg_duration_ms: f64,
    /// Average user feedback score (0.0–1.0, None if no feedback).
    pub user_feedback_score: Option<f64>,
    /// Average tokens consumed per use.
    pub avg_token_usage: f64,
    /// Times skill was injected into LLM context.
    pub injection_count: u64,
    /// Times LLM actually used the skill.
    pub actual_usage_count: u64,
}

impl SkillMetrics {
    /// Success rate as a fraction (0.0–1.0).
    pub fn success_rate(&self) -> f64 {
        if self.use_count == 0 {
            return 0.0;
        }
        self.success_count as f64 / self.use_count as f64
    }

    /// Failure rate as a fraction (0.0–1.0).
    pub fn failure_rate(&self) -> f64 {
        if self.use_count == 0 {
            return 0.0;
        }
        self.failure_count as f64 / self.use_count as f64
    }

    /// Usage rate: fraction of injections where the skill was actually used.
    pub fn usage_rate(&self) -> f64 {
        if self.injection_count == 0 {
            return 0.0;
        }
        self.actual_usage_count as f64 / self.injection_count as f64
    }

    /// Record one execution outcome with token usage.
    pub fn record(&mut self, success: bool, partial: bool, duration_ms: u64, token_usage: u64) {
        self.use_count += 1;
        if success {
            self.success_count += 1;
        } else if partial {
            self.partial_count += 1;
        } else {
            self.failure_count += 1;
        }

        // Running averages.
        let n = self.use_count as f64;
        self.avg_duration_ms = self.avg_duration_ms * (n - 1.0) / n + duration_ms as f64 / n;
        self.avg_token_usage = self.avg_token_usage * (n - 1.0) / n + token_usage as f64 / n;
    }

    /// Record that this skill was injected into LLM context.
    pub fn record_injection(&mut self) {
        self.injection_count += 1;
    }

    /// Record that the LLM actually used this skill.
    pub fn record_actual_usage(&mut self) {
        self.actual_usage_count += 1;
    }
}

// ---------------------------------------------------------------------------
// Evolution proposals
// ---------------------------------------------------------------------------

/// Status of an evolution proposal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalStatus {
    PendingApproval,
    Approved,
    Rejected,
    Deferred,
}

/// Type of change proposed by the refiner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeType {
    /// Adding handling for a discovered edge case.
    EdgeCaseAddition,
    /// Adding a warning about a common error.
    ErrorWarning,
    /// Improving phrasing of existing instructions.
    PhrasingUpdate,
    /// Splitting a capability into sub-skills.
    CapabilitySplit,
    /// Removing an obsolete rule.
    RuleRemoval,
    /// Discovering a new workflow from experience.
    WorkflowDiscovery,
}

/// A proposed change to a skill document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvolutionProposal {
    /// Unique proposal ID.
    pub id: String,
    /// Target skill name.
    pub skill_name: String,
    /// Current skill version hash.
    pub current_version: String,
    /// Summary of proposed changes.
    pub proposed_changes: String,
    /// Pattern types that motivated this proposal.
    pub patterns: Vec<PatternType>,
    /// Current status.
    pub status: ProposalStatus,
    /// Proposed new version identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposed_version: Option<String>,
    /// Type of change being proposed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change_type: Option<ChangeType>,
    /// Pattern IDs that this proposal references.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub patterns_referenced: Vec<String>,
    /// Preview of the diff this proposal would produce.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub diff_preview: String,
    /// Deferred until this date (if deferred).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deferred_until: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// T-P3-39-04: Skill metrics track success/failure rates.
    #[test]
    fn test_skill_metrics_rates() {
        let mut metrics = SkillMetrics::default();
        metrics.record(true, false, 1000, 500);
        metrics.record(true, false, 2000, 600);
        metrics.record(false, false, 500, 200);

        assert_eq!(metrics.use_count, 3);
        assert!((metrics.success_rate() - 2.0 / 3.0).abs() < f64::EPSILON);
        assert!((metrics.failure_rate() - 1.0 / 3.0).abs() < f64::EPSILON);
    }

    /// T-P3-39-05: Metrics running average duration.
    #[test]
    fn test_skill_metrics_duration() {
        let mut metrics = SkillMetrics::default();
        metrics.record(true, false, 1000, 500);
        metrics.record(true, false, 3000, 700);
        // avg = (1000 + 3000) / 2 = 2000
        assert!((metrics.avg_duration_ms - 2000.0).abs() < 1.0);
    }

    /// T-SK-S6-01: Enhanced SkillMetrics tracks usage_rate.
    #[test]
    fn test_skill_metrics_usage_rate() {
        let mut metrics = SkillMetrics::default();
        metrics.record_injection();
        metrics.record_injection();
        metrics.record_injection();
        metrics.record_actual_usage();
        metrics.record_actual_usage();

        assert!((metrics.usage_rate() - 2.0 / 3.0).abs() < f64::EPSILON);
        assert_eq!(metrics.injection_count, 3);
        assert_eq!(metrics.actual_usage_count, 2);
    }

    /// T-SK-S6-01b: avg_token_usage tracks running average.
    #[test]
    fn test_skill_metrics_avg_token_usage() {
        let mut metrics = SkillMetrics::default();
        metrics.record(true, false, 100, 1000);
        metrics.record(true, false, 100, 3000);
        // avg = (1000 + 3000) / 2 = 2000
        assert!((metrics.avg_token_usage - 2000.0).abs() < 1.0);
    }
}
