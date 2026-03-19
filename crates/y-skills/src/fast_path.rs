//! Fast-path extraction: real-time pattern extraction after each interaction.
//!
//! Restricted to minor pattern types (`BetterPhrasing`, `CommonError`) with
//! evidence exclusively from `UserCorrection` or `UserStated`. No new skill
//! creation — auto-minor approval only.

use crate::evolution::PatternType;
use crate::experience::{EvidenceProvenance, ExperienceRecord};
use crate::extractor::ExtractedPattern;

/// Fast-path extractor for real-time, post-interaction pattern extraction.
#[derive(Debug)]
pub struct FastPathExtractor;

impl FastPathExtractor {
    /// Create a new fast-path extractor.
    pub fn new() -> Self {
        Self
    }

    /// Try to extract a fast-path pattern from a single experience.
    ///
    /// Returns `Some(pattern)` only if:
    /// - Evidence is from `UserCorrection` or `UserStated`
    /// - Pattern type is `BetterPhrasing` or `CommonError`
    pub fn try_extract(&self, experience: &ExperienceRecord) -> Option<ExtractedPattern> {
        // Must have a skill
        let skill_id = experience.skill_id.as_ref()?;

        // Filter: only user correction or user stated evidence
        let eligible_evidence: Vec<_> = experience
            .evidence
            .iter()
            .filter(|e| {
                matches!(
                    e.provenance,
                    EvidenceProvenance::UserCorrection | EvidenceProvenance::UserStated
                )
            })
            .collect();

        if eligible_evidence.is_empty() {
            return None;
        }

        // Determine pattern type from outcome and evidence
        let pattern_type = self.classify_fast_path(experience, &eligible_evidence);

        // Only minor types allowed
        if !matches!(
            pattern_type,
            PatternType::BetterPhrasing | PatternType::CommonError
        ) {
            return None;
        }

        let description = eligible_evidence
            .iter()
            .map(|e| e.content.as_str())
            .collect::<Vec<_>>()
            .join("; ");

        Some(ExtractedPattern {
            id: format!("fp-{}", experience.id),
            skill_id: skill_id.clone(),
            pattern_type,
            description,
            frequency: 1,
            evidence_ids: vec![experience.id.clone()],
        })
    }

    fn classify_fast_path(
        &self,
        experience: &ExperienceRecord,
        evidence: &[&crate::experience::EvidenceEntry],
    ) -> PatternType {
        // User corrections → better phrasing
        let has_correction = evidence
            .iter()
            .any(|e| e.provenance == EvidenceProvenance::UserCorrection);

        if has_correction {
            return PatternType::BetterPhrasing;
        }

        // Failure with user_stated evidence → common error
        if experience.outcome == crate::experience::ExperienceOutcome::Failure {
            return PatternType::CommonError;
        }

        // Default: better phrasing (from user_stated)
        PatternType::BetterPhrasing
    }
}

impl Default for FastPathExtractor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::experience::*;

    fn make_record(outcome: ExperienceOutcome, provenance: EvidenceProvenance) -> ExperienceRecord {
        ExperienceRecord {
            id: "exp-1".to_string(),
            timestamp: "2026-03-10T00:00:00Z".to_string(),
            skill_id: Some("test-skill".to_string()),
            skill_version: None,
            task_description: "test".to_string(),
            outcome,
            trajectory_summary: "done".to_string(),
            key_decisions: vec![],
            evidence: vec![EvidenceEntry {
                content: "user said to change approach".to_string(),
                provenance,
            }],
            tool_calls: vec![],
            error_messages: vec![],
            duration_ms: 1000,
            token_usage: TokenUsage::new(500, 200),
        }
    }

    /// T-SK-S7-04: Fast-path rejects non-minor changes.
    #[test]
    fn test_fast_path_rejects_non_eligible_evidence() {
        let fp = FastPathExtractor::new();

        // AgentObservation is not eligible
        let record = make_record(
            ExperienceOutcome::Success,
            EvidenceProvenance::AgentObservation,
        );
        assert!(fp.try_extract(&record).is_none());
    }

    /// T-SK-S7-05: Fast-path only accepts UserCorrection/UserStated evidence.
    #[test]
    fn test_fast_path_accepts_user_correction() {
        let fp = FastPathExtractor::new();

        let record = make_record(
            ExperienceOutcome::Success,
            EvidenceProvenance::UserCorrection,
        );
        let result = fp.try_extract(&record);
        assert!(result.is_some());
        let p = result.unwrap();
        assert_eq!(p.pattern_type, PatternType::BetterPhrasing);
        assert_eq!(p.skill_id, "test-skill");
    }

    /// Failure + UserStated produces CommonError.
    #[test]
    fn test_fast_path_failure_common_error() {
        let fp = FastPathExtractor::new();

        let record = make_record(ExperienceOutcome::Failure, EvidenceProvenance::UserStated);
        let result = fp.try_extract(&record);
        assert!(result.is_some());
        assert_eq!(result.unwrap().pattern_type, PatternType::CommonError);
    }

    /// No skill → no extraction.
    #[test]
    fn test_fast_path_no_skill() {
        let fp = FastPathExtractor::new();

        let mut record = make_record(
            ExperienceOutcome::Success,
            EvidenceProvenance::UserCorrection,
        );
        record.skill_id = None;
        assert!(fp.try_extract(&record).is_none());
    }
}
