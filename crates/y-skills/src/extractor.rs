//! Pattern extractor: identifies recurring patterns from experience records.
//!
//! Groups experiences by skill, detects patterns from failures, corrections,
//! and recurring decisions. Currently deterministic; designed for future
//! LLM-assisted pattern extraction via `y-provider`.

use std::collections::HashMap;

use crate::evolution::PatternType;

/// An extracted pattern from experience data.
#[derive(Debug, Clone)]
pub struct ExtractedPattern {
    /// Unique pattern ID.
    pub id: String,
    /// Skill this pattern relates to.
    pub skill_id: String,
    /// Type of pattern.
    pub pattern_type: PatternType,
    /// Human-readable description.
    pub description: String,
    /// How many times this pattern was observed.
    pub frequency: u32,
    /// Experience record IDs that provided evidence.
    pub evidence_ids: Vec<String>,
}

/// Registry of extracted patterns with deduplication.
#[derive(Debug, Default)]
pub struct PatternRegistry {
    /// Patterns indexed by ID.
    patterns: HashMap<String, ExtractedPattern>,
}

impl PatternRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a new pattern or increment frequency of existing one.
    ///
    /// Deduplication is by `(skill_id, pattern_type, description)`.
    pub fn register(&mut self, pattern: ExtractedPattern) {
        let dedup_key = format!(
            "{}:{}:{}",
            pattern.skill_id, pattern.pattern_type, pattern.description
        );

        if let Some(existing) = self.patterns.get_mut(&dedup_key) {
            existing.frequency += pattern.frequency;
            for eid in &pattern.evidence_ids {
                if !existing.evidence_ids.contains(eid) {
                    existing.evidence_ids.push(eid.clone());
                }
            }
        } else {
            self.patterns.insert(dedup_key, pattern);
        }
    }

    /// Get all patterns for a skill.
    pub fn by_skill(&self, skill_id: &str) -> Vec<&ExtractedPattern> {
        self.patterns
            .values()
            .filter(|p| p.skill_id == skill_id)
            .collect()
    }

    /// Get all patterns.
    pub fn all(&self) -> Vec<&ExtractedPattern> {
        self.patterns.values().collect()
    }

    /// Total pattern count.
    pub fn count(&self) -> usize {
        self.patterns.len()
    }
}

/// Extracts patterns from experience records.
#[derive(Debug)]
pub struct PatternExtractor {
    /// Minimum experiences required before extracting patterns.
    min_experiences: usize,
}

impl PatternExtractor {
    /// Create a new extractor with default minimum (3 experiences).
    pub fn new() -> Self {
        Self { min_experiences: 3 }
    }

    /// Create an extractor with a custom minimum.
    pub fn with_min_experiences(min: usize) -> Self {
        Self {
            min_experiences: min,
        }
    }

    /// Extract patterns from a set of experience records grouped by skill.
    ///
    /// Returns newly extracted patterns. Call `PatternRegistry::register`
    /// to store them with deduplication.
    pub fn extract(
        &self,
        experiences: &[crate::experience::ExperienceRecord],
    ) -> Vec<ExtractedPattern> {
        let mut by_skill: HashMap<String, Vec<&crate::experience::ExperienceRecord>> =
            HashMap::new();

        for exp in experiences {
            if let Some(ref sid) = exp.skill_id {
                by_skill.entry(sid.clone()).or_default().push(exp);
            }
        }

        let mut patterns = Vec::new();

        for (skill_id, records) in &by_skill {
            if records.len() < self.min_experiences {
                continue;
            }

            // Pattern: recurring failures
            let failures: Vec<_> = records
                .iter()
                .filter(|r| r.outcome == crate::experience::ExperienceOutcome::Failure)
                .collect();

            if failures.len() >= 2 {
                let evidence_ids: Vec<String> = failures.iter().map(|r| r.id.clone()).collect();
                patterns.push(ExtractedPattern {
                    id: format!("pat-fail-{skill_id}"),
                    skill_id: skill_id.clone(),
                    pattern_type: PatternType::CommonError,
                    description: format!(
                        "Recurring failure: {}/{} executions failed",
                        failures.len(),
                        records.len()
                    ),
                    frequency: failures.len() as u32,
                    evidence_ids,
                });
            }

            // Pattern: user corrections suggest better phrasing
            let corrections: Vec<_> = records
                .iter()
                .filter(|r| {
                    r.evidence.iter().any(|e| {
                        e.provenance == crate::experience::EvidenceProvenance::UserCorrection
                    })
                })
                .collect();

            if corrections.len() >= 2 {
                let evidence_ids: Vec<String> = corrections.iter().map(|r| r.id.clone()).collect();
                patterns.push(ExtractedPattern {
                    id: format!("pat-phrase-{skill_id}"),
                    skill_id: skill_id.clone(),
                    pattern_type: PatternType::BetterPhrasing,
                    description: format!(
                        "Multiple user corrections ({}) suggest phrasing improvements",
                        corrections.len()
                    ),
                    frequency: corrections.len() as u32,
                    evidence_ids,
                });
            }

            // Pattern: low usage rate detected from error messages
            let with_errors: Vec<_> = records
                .iter()
                .filter(|r| !r.error_messages.is_empty())
                .collect();

            if with_errors.len() >= 2 {
                let evidence_ids: Vec<String> = with_errors.iter().map(|r| r.id.clone()).collect();
                patterns.push(ExtractedPattern {
                    id: format!("pat-err-{skill_id}"),
                    skill_id: skill_id.clone(),
                    pattern_type: PatternType::EdgeCase,
                    description: format!(
                        "Recurring errors in {}/{} executions",
                        with_errors.len(),
                        records.len()
                    ),
                    frequency: with_errors.len() as u32,
                    evidence_ids,
                });
            }
        }

        patterns
    }

    /// Extract patterns from skillless experiences (`skill_id` = None).
    ///
    /// Groups successful skillless experiences by task description similarity
    /// and proposes `WorkflowDiscovery` patterns for clusters ≥ 3.
    pub fn extract_skillless(
        &self,
        experiences: &[crate::experience::ExperienceRecord],
    ) -> Vec<ExtractedPattern> {
        let skillless: Vec<_> = experiences
            .iter()
            .filter(|e| {
                e.skill_id.is_none() && e.outcome == crate::experience::ExperienceOutcome::Success
            })
            .collect();

        if skillless.len() < self.min_experiences {
            return vec![];
        }

        // Simple clustering: group by word overlap in task_description
        let mut clusters: Vec<Vec<&crate::experience::ExperienceRecord>> = Vec::new();

        for exp in &skillless {
            let exp_words: std::collections::HashSet<String> = exp
                .task_description
                .to_lowercase()
                .split_whitespace()
                .filter(|w| w.len() > 3)
                .map(String::from)
                .collect();

            let mut placed = false;
            for cluster in &mut clusters {
                if let Some(first) = cluster.first() {
                    let first_words: std::collections::HashSet<String> = first
                        .task_description
                        .to_lowercase()
                        .split_whitespace()
                        .filter(|w| w.len() > 3)
                        .map(String::from)
                        .collect();

                    let intersection = exp_words.intersection(&first_words).count();
                    let smaller = exp_words.len().min(first_words.len());
                    if smaller > 0 && intersection * 2 >= smaller {
                        cluster.push(exp);
                        placed = true;
                        break;
                    }
                }
            }

            if !placed {
                clusters.push(vec![exp]);
            }
        }

        clusters
            .into_iter()
            .filter(|c| c.len() >= 3)
            .enumerate()
            .map(|(i, cluster)| {
                let evidence_ids: Vec<String> = cluster.iter().map(|r| r.id.clone()).collect();
                let desc = cluster
                    .first()
                    .map(|c| c.task_description.clone())
                    .unwrap_or_default();
                ExtractedPattern {
                    id: format!("pat-workflow-{i}"),
                    skill_id: String::new(),
                    pattern_type: PatternType::WorkflowDiscovery,
                    description: format!(
                        "Recurring skillless workflow ({} occurrences): {}",
                        cluster.len(),
                        desc
                    ),
                    frequency: cluster.len() as u32,
                    evidence_ids,
                }
            })
            .collect()
    }
}

impl Default for PatternExtractor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::experience::*;

    fn make_exp(
        skill_id: &str,
        outcome: ExperienceOutcome,
        has_correction: bool,
        errors: &[&str],
    ) -> ExperienceRecord {
        let mut evidence = vec![EvidenceEntry {
            content: "test".to_string(),
            provenance: EvidenceProvenance::TaskOutcome,
        }];
        if has_correction {
            evidence.push(EvidenceEntry {
                content: "user said do it differently".to_string(),
                provenance: EvidenceProvenance::UserCorrection,
            });
        }

        ExperienceRecord {
            id: uuid::Uuid::new_v4().to_string(),
            timestamp: "2026-03-10T00:00:00Z".to_string(),
            skill_id: Some(skill_id.to_string()),
            skill_version: None,
            task_description: "test task".to_string(),
            outcome,
            trajectory_summary: "did stuff".to_string(),
            key_decisions: vec![],
            evidence,
            tool_calls: vec![],
            error_messages: errors.iter().map(|s| (*s).to_string()).collect(),
            duration_ms: 1000,
            token_usage: TokenUsage::new(500, 200),
        }
    }

    /// T-SK-S6-03: Pattern extractor groups experiences by skill.
    #[test]
    fn test_extractor_groups_by_skill() {
        let extractor = PatternExtractor::with_min_experiences(2);

        let experiences = vec![
            make_exp("skill-a", ExperienceOutcome::Failure, false, &[]),
            make_exp("skill-a", ExperienceOutcome::Failure, false, &[]),
            make_exp("skill-a", ExperienceOutcome::Success, false, &[]),
            make_exp("skill-b", ExperienceOutcome::Success, false, &[]),
        ];

        let patterns = extractor.extract(&experiences);

        // skill-a has 2 failures out of 3 → CommonError pattern
        assert!(!patterns.is_empty());
        assert!(patterns
            .iter()
            .any(|p| p.skill_id == "skill-a" && p.pattern_type == PatternType::CommonError));
        // skill-b only has 1 experience → below threshold
        assert!(!patterns.iter().any(|p| p.skill_id == "skill-b"));
    }

    /// T-SK-S6-04: Pattern deduplication across extraction cycles.
    #[test]
    fn test_pattern_deduplication() {
        let mut registry = PatternRegistry::new();

        let p1 = ExtractedPattern {
            id: "pat-1".to_string(),
            skill_id: "skill-a".to_string(),
            pattern_type: PatternType::CommonError,
            description: "some error".to_string(),
            frequency: 2,
            evidence_ids: vec!["e1".to_string()],
        };

        let p2 = ExtractedPattern {
            id: "pat-2".to_string(),
            skill_id: "skill-a".to_string(),
            pattern_type: PatternType::CommonError,
            description: "some error".to_string(),
            frequency: 3,
            evidence_ids: vec!["e2".to_string()],
        };

        registry.register(p1);
        registry.register(p2);

        // Deduplicated: same (skill_id, type, description)
        assert_eq!(registry.count(), 1);
        let patterns = registry.by_skill("skill-a");
        assert_eq!(patterns[0].frequency, 5); // 2 + 3
        assert_eq!(patterns[0].evidence_ids.len(), 2); // e1 + e2
    }

    /// User corrections trigger BetterPhrasing patterns.
    #[test]
    fn test_extractor_detects_corrections() {
        let extractor = PatternExtractor::with_min_experiences(2);

        let experiences = vec![
            make_exp("skill-c", ExperienceOutcome::Success, true, &[]),
            make_exp("skill-c", ExperienceOutcome::Success, true, &[]),
            make_exp("skill-c", ExperienceOutcome::Success, false, &[]),
        ];

        let patterns = extractor.extract(&experiences);
        assert!(patterns
            .iter()
            .any(|p| p.pattern_type == PatternType::BetterPhrasing));
    }

    /// T-SK-S7-03: Skillless analysis clusters similar tasks.
    #[test]
    fn test_skillless_analysis() {
        let extractor = PatternExtractor::with_min_experiences(3);

        // 3 similar skillless experiences
        let mut exps: Vec<ExperienceRecord> = (0..3)
            .map(|i| {
                let mut e = make_exp("unused", ExperienceOutcome::Success, false, &[]);
                e.skill_id = None;
                e.task_description = format!("deploy application to kubernetes cluster {i}");
                e
            })
            .collect();

        // 1 different skillless experience (should not cluster)
        let mut diff = make_exp("unused", ExperienceOutcome::Success, false, &[]);
        diff.skill_id = None;
        diff.task_description = "write a haiku about cats".to_string();
        exps.push(diff);

        let patterns = extractor.extract_skillless(&exps);
        assert_eq!(patterns.len(), 1);
        assert_eq!(patterns[0].pattern_type, PatternType::WorkflowDiscovery);
        assert!(patterns[0].description.contains("3 occurrences"));
    }
}
