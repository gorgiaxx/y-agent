//! Skill usage audit: tracks which injected skills the LLM actually used.
//!
//! Provides keyword-overlap-based fallback judgment and updates
//! `SkillMetrics.injection_count` / `actual_usage_count`. Designed
//! for future LLM-assisted judgment via `y-provider`.

use std::collections::HashSet;

use crate::evolution::SkillMetrics;

/// A skill that was injected into a task context.
#[derive(Debug, Clone)]
pub struct InjectedSkill {
    /// Skill name/ID.
    pub name: String,
    /// The skill content that was injected.
    pub content: String,
}

/// Judgment on whether a skill was used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UsageJudgment {
    /// Skill was relevant and used.
    Used,
    /// Skill was injected but not used.
    NotUsed,
}

/// Result of auditing one skill's usage.
#[derive(Debug, Clone)]
pub struct SkillUsageResult {
    /// Skill name.
    pub skill_name: String,
    /// Whether it was used.
    pub judgment: UsageJudgment,
    /// Keyword overlap score (0.0–1.0).
    pub overlap_score: f64,
}

/// Tracks skill usage via keyword overlap analysis.
///
/// In production, a hook middleware would call `audit()` after each task,
/// passing the agent output and injected skills. This updates
/// `SkillMetrics` and can flag low-usage skills for obsolescence.
#[derive(Debug)]
pub struct SkillUsageAudit {
    /// Overlap threshold to consider a skill "used".
    usage_threshold: f64,
    /// Usage rate below which a skill is considered obsolete.
    obsolete_threshold: f64,
}

impl SkillUsageAudit {
    /// Create with default thresholds (0.15 overlap for usage, 0.1 for obsolete).
    pub fn new() -> Self {
        Self {
            usage_threshold: 0.15,
            obsolete_threshold: 0.1,
        }
    }

    /// Audit which injected skills were actually used based on output content.
    pub fn audit(
        &self,
        agent_output: &str,
        injected_skills: &[InjectedSkill],
        metrics: &mut std::collections::HashMap<String, SkillMetrics>,
    ) -> Vec<SkillUsageResult> {
        let output_words = Self::extract_words(agent_output);
        let mut results = Vec::new();

        for skill in injected_skills {
            let skill_words = Self::extract_words(&skill.content);
            let overlap = Self::word_overlap(&output_words, &skill_words);

            let judgment = if overlap >= self.usage_threshold {
                UsageJudgment::Used
            } else {
                UsageJudgment::NotUsed
            };

            // Update metrics
            let m = metrics.entry(skill.name.clone()).or_default();
            m.record_injection();
            if judgment == UsageJudgment::Used {
                m.record_actual_usage();
            }

            results.push(SkillUsageResult {
                skill_name: skill.name.clone(),
                judgment,
                overlap_score: overlap,
            });
        }

        results
    }

    /// Check which skills have fallen below the obsolete threshold.
    pub fn detect_obsolete(
        &self,
        metrics: &std::collections::HashMap<String, SkillMetrics>,
    ) -> Vec<String> {
        metrics
            .iter()
            .filter(|(_, m)| m.injection_count >= 10 && m.usage_rate() < self.obsolete_threshold)
            .map(|(name, _)| name.clone())
            .collect()
    }

    fn extract_words(text: &str) -> HashSet<String> {
        text.to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| w.len() > 3)
            .map(String::from)
            .collect()
    }

    #[allow(clippy::cast_precision_loss)]
    fn word_overlap(output_words: &HashSet<String>, skill_words: &HashSet<String>) -> f64 {
        if skill_words.is_empty() {
            return 0.0;
        }
        let intersection = output_words.intersection(skill_words).count();
        intersection as f64 / skill_words.len() as f64
    }
}

impl Default for SkillUsageAudit {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// T-SK-S7-01: Usage audit middleware updates injection/usage counts.
    #[test]
    fn test_usage_audit_updates_counts() {
        let audit = SkillUsageAudit::new();
        let mut metrics = std::collections::HashMap::new();

        let injected = vec![
            InjectedSkill {
                name: "code-review".to_string(),
                content: "Review code for bugs, security issues, and style problems".to_string(),
            },
            InjectedSkill {
                name: "essay-writer".to_string(),
                content: "Write essays with clear structure and grammar".to_string(),
            },
        ];

        // Output mentions code review terms but not essay writing
        let output = "I reviewed the code and found several bugs and security issues \
                       in the authentication module. The style problems were minor.";

        let results = audit.audit(output, &injected, &mut metrics);

        assert_eq!(results.len(), 2);

        // code-review should be Used (high overlap)
        let cr = results
            .iter()
            .find(|r| r.skill_name == "code-review")
            .unwrap();
        assert_eq!(cr.judgment, UsageJudgment::Used);

        // essay-writer should be NotUsed (no overlap)
        let ew = results
            .iter()
            .find(|r| r.skill_name == "essay-writer")
            .unwrap();
        assert_eq!(ew.judgment, UsageJudgment::NotUsed);

        // Metrics updated
        assert_eq!(metrics["code-review"].injection_count, 1);
        assert_eq!(metrics["code-review"].actual_usage_count, 1);
        assert_eq!(metrics["essay-writer"].injection_count, 1);
        assert_eq!(metrics["essay-writer"].actual_usage_count, 0);
    }

    /// T-SK-S7-02: Low usage_rate triggers ObsoleteRule detection.
    #[test]
    fn test_low_usage_rate_obsolete() {
        let audit = SkillUsageAudit::new();
        let mut metrics = std::collections::HashMap::new();

        // Simulate a skill injected 20 times but used only once
        let mut m = SkillMetrics::default();
        for _ in 0..20 {
            m.record_injection();
        }
        m.record_actual_usage(); // usage_rate = 1/20 = 0.05
        metrics.insert("unused-skill".to_string(), m);

        let obsolete = audit.detect_obsolete(&metrics);
        assert!(obsolete.contains(&"unused-skill".to_string()));
    }

    /// Skill with good usage rate is not flagged.
    #[test]
    fn test_good_usage_not_obsolete() {
        let audit = SkillUsageAudit::new();
        let mut metrics = std::collections::HashMap::new();

        let mut m = SkillMetrics::default();
        for _ in 0..10 {
            m.record_injection();
            m.record_actual_usage();
        }
        metrics.insert("good-skill".to_string(), m);

        let obsolete = audit.detect_obsolete(&metrics);
        assert!(obsolete.is_empty());
    }
}
