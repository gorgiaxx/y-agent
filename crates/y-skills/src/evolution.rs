//! Skill evolution: proposals, approval gates, and metrics.
//!
//! Design reference: skill-versioning-evolution-design.md §Evolution Pipeline
//!
//! After accumulating enough experience, the system can propose changes
//! to skill documents. Proposals go through an approval gate that
//! checks the configured policy before applying changes.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Pattern types
// ---------------------------------------------------------------------------

/// Types of patterns extracted from experience records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

#[allow(clippy::cast_precision_loss)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalStatus {
    PendingApproval,
    Approved,
    Rejected,
    Deferred,
}

/// Type of change proposed by the refiner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

// ---------------------------------------------------------------------------
// Approval gate
// ---------------------------------------------------------------------------

/// Approval policy for a skill.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalPolicy {
    /// All changes require human approval.
    Supervised,
    /// Minor changes (`BetterPhrasing`, `ObsoleteRule`) auto-approve.
    AutoMinor,
    /// Auto-approve if evaluation passes.
    AutoEvaluated,
    /// Fully autonomous — all proposals auto-approve.
    Autonomous,
    /// No changes allowed.
    Frozen,
}

/// Approval gate: decides whether a proposal should be auto-approved.
#[derive(Debug)]
pub struct ApprovalGate {
    policy: ApprovalPolicy,
}

impl ApprovalGate {
    /// Create a new approval gate with the given policy.
    pub fn new(policy: ApprovalPolicy) -> Self {
        Self { policy }
    }

    /// Check a proposal against the policy.
    ///
    /// Returns `true` if the proposal should be auto-approved.
    pub fn check(&self, proposal: &EvolutionProposal) -> bool {
        match &self.policy {
            ApprovalPolicy::Frozen | ApprovalPolicy::Supervised | ApprovalPolicy::AutoEvaluated => {
                // AutoEvaluated stub: in production, would run an LLM evaluation.
                false
            }
            ApprovalPolicy::Autonomous => true,
            ApprovalPolicy::AutoMinor => {
                // Auto-approve if all patterns are minor.
                proposal
                    .patterns
                    .iter()
                    .all(|p| matches!(p, PatternType::BetterPhrasing | PatternType::ObsoleteRule))
            }
        }
    }

    /// Get the current policy.
    pub fn policy(&self) -> &ApprovalPolicy {
        &self.policy
    }
}

// ---------------------------------------------------------------------------
// Skill refiner
// ---------------------------------------------------------------------------

use crate::extractor::ExtractedPattern;

/// Generates evolution proposals from extracted patterns.
#[derive(Debug)]
pub struct SkillRefiner;

impl SkillRefiner {
    /// Create a new refiner.
    pub fn new() -> Self {
        Self
    }

    /// Generate a proposal from a set of extracted patterns for a skill.
    pub fn propose(
        &self,
        skill_name: &str,
        current_version: &str,
        patterns: &[ExtractedPattern],
    ) -> Option<EvolutionProposal> {
        if patterns.is_empty() {
            return None;
        }

        let change_type = Self::determine_change_type(patterns);
        let pattern_types: Vec<PatternType> =
            patterns.iter().map(|p| p.pattern_type.clone()).collect();
        let pattern_ids: Vec<String> = patterns.iter().map(|p| p.id.clone()).collect();

        let description = patterns
            .iter()
            .map(|p| p.description.as_str())
            .collect::<Vec<_>>()
            .join("; ");

        Some(EvolutionProposal {
            id: format!("prop-{}", uuid::Uuid::new_v4()),
            skill_name: skill_name.to_string(),
            current_version: current_version.to_string(),
            proposed_changes: description,
            patterns: pattern_types,
            status: ProposalStatus::PendingApproval,
            proposed_version: None,
            change_type: Some(change_type),
            patterns_referenced: pattern_ids,
            diff_preview: String::new(),
            deferred_until: None,
        })
    }

    fn determine_change_type(patterns: &[ExtractedPattern]) -> ChangeType {
        // Use the most significant pattern type to determine change type.
        for p in patterns {
            match p.pattern_type {
                PatternType::EdgeCase => return ChangeType::EdgeCaseAddition,
                PatternType::CommonError => return ChangeType::ErrorWarning,
                PatternType::NewCapability => return ChangeType::CapabilitySplit,
                PatternType::WorkflowDiscovery => return ChangeType::WorkflowDiscovery,
                PatternType::ObsoleteRule => return ChangeType::RuleRemoval,
                PatternType::BetterPhrasing => {}
            }
        }
        ChangeType::PhrasingUpdate
    }
}

impl Default for SkillRefiner {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minor_proposal() -> EvolutionProposal {
        EvolutionProposal {
            id: "prop-1".to_string(),
            skill_name: "code-review".to_string(),
            current_version: "abc123".to_string(),
            proposed_changes: "Improve phrasing of review instructions".to_string(),
            patterns: vec![PatternType::BetterPhrasing],
            status: ProposalStatus::PendingApproval,
            proposed_version: None,
            change_type: Some(ChangeType::PhrasingUpdate),
            patterns_referenced: vec![],
            diff_preview: String::new(),
            deferred_until: None,
        }
    }

    fn major_proposal() -> EvolutionProposal {
        EvolutionProposal {
            id: "prop-2".to_string(),
            skill_name: "code-review".to_string(),
            current_version: "abc123".to_string(),
            proposed_changes: "Add new security analysis capability".to_string(),
            patterns: vec![PatternType::NewCapability],
            status: ProposalStatus::PendingApproval,
            proposed_version: None,
            change_type: Some(ChangeType::CapabilitySplit),
            patterns_referenced: vec![],
            diff_preview: String::new(),
            deferred_until: None,
        }
    }

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

    /// T-SK-S6-05: SkillRefiner generates proposal from patterns.
    #[test]
    fn test_skill_refiner_generates_proposal() {
        let refiner = SkillRefiner::new();
        let patterns = vec![ExtractedPattern {
            id: "pat-1".to_string(),
            skill_id: "code-review".to_string(),
            pattern_type: PatternType::EdgeCase,
            description: "Edge case: large files".to_string(),
            frequency: 5,
            evidence_ids: vec!["exp-1".to_string()],
        }];

        let proposal = refiner.propose("code-review", "v1", &patterns);
        assert!(proposal.is_some());
        let p = proposal.unwrap();
        assert_eq!(p.skill_name, "code-review");
        assert_eq!(p.change_type, Some(ChangeType::EdgeCaseAddition));
        assert!(!p.patterns_referenced.is_empty());
    }

    /// T-SK-S6-06: Empty patterns produce no proposal.
    #[test]
    fn test_skill_refiner_no_patterns() {
        let refiner = SkillRefiner::new();
        assert!(refiner.propose("test", "v1", &[]).is_none());
    }

    /// T-P3-39-06: Supervised gate rejects all proposals.
    #[test]
    fn test_approval_gate_supervised() {
        let gate = ApprovalGate::new(ApprovalPolicy::Supervised);
        assert!(!gate.check(&minor_proposal()));
        assert!(!gate.check(&major_proposal()));
    }

    /// T-P3-39-07: Autonomous gate approves all proposals.
    #[test]
    fn test_approval_gate_autonomous() {
        let gate = ApprovalGate::new(ApprovalPolicy::Autonomous);
        assert!(gate.check(&minor_proposal()));
        assert!(gate.check(&major_proposal()));
    }

    /// T-P3-39-08: AutoMinor gate approves minor but rejects major.
    #[test]
    fn test_approval_gate_auto_minor() {
        let gate = ApprovalGate::new(ApprovalPolicy::AutoMinor);
        assert!(gate.check(&minor_proposal()));
        assert!(!gate.check(&major_proposal()));
    }

    /// T-P3-39-09: Frozen gate rejects everything.
    #[test]
    fn test_approval_gate_frozen() {
        let gate = ApprovalGate::new(ApprovalPolicy::Frozen);
        assert!(!gate.check(&minor_proposal()));
    }
}
