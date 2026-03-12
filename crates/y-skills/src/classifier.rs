//! Skill classifier: determines skill type from analysis report.
//!
//! Uses the classification hint from the content analyzer plus
//! rule-based heuristics to assign a definitive skill type.

use crate::analyzer::AnalysisReport;

// Re-export the enum from y-core (canonical definition).
pub use y_core::skill::SkillClassificationType;

/// Classifies skills based on analysis reports.
#[derive(Debug)]
pub struct SkillClassifier;

impl SkillClassifier {
    /// Create a new classifier.
    pub fn new() -> Self {
        Self
    }

    /// Classify a skill from an analysis report.
    pub fn classify(&self, report: &AnalysisReport) -> SkillClassificationType {
        // Use the hint as primary signal
        match report.classification_hint.as_str() {
            "llm_reasoning" => {
                // Double-check: if there are embedded tools or scripts, it's hybrid
                if !report.embedded_tools.is_empty() || !report.embedded_scripts.is_empty() {
                    SkillClassificationType::Hybrid
                } else {
                    SkillClassificationType::LlmReasoning
                }
            }
            "api_call" => {
                if report.capabilities.iter().any(|c| {
                    c.to_lowercase().contains("reason") || c.to_lowercase().contains("analyz")
                }) {
                    SkillClassificationType::Hybrid
                } else {
                    SkillClassificationType::ApiCall
                }
            }
            "tool_wrapper" => SkillClassificationType::ToolWrapper,
            "agent_behavior" => SkillClassificationType::AgentBehavior,
            "hybrid" => SkillClassificationType::Hybrid,
            _ => {
                // Fallback heuristics
                if !report.embedded_tools.is_empty() && !report.embedded_scripts.is_empty() {
                    SkillClassificationType::Hybrid
                } else if !report.embedded_tools.is_empty() {
                    SkillClassificationType::ApiCall
                } else if !report.embedded_scripts.is_empty() {
                    SkillClassificationType::ToolWrapper
                } else {
                    SkillClassificationType::LlmReasoning
                }
            }
        }
    }
}

impl Default for SkillClassifier {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::{AnalysisReport, SafetyFlags};

    fn base_report() -> AnalysisReport {
        AnalysisReport {
            purpose: "test".to_string(),
            classification_hint: "llm_reasoning".to_string(),
            capabilities: vec![],
            embedded_tools: vec![],
            embedded_scripts: vec![],
            quality_issues: vec![],
            token_estimate: 100,
            safety_flags: SafetyFlags::default(),
        }
    }

    /// T-SK-S4-03: Classifier assigns `LlmReasoning` for reasoning-only skills.
    #[test]
    fn test_classify_llm_reasoning() {
        let classifier = SkillClassifier::new();
        let report = base_report();
        assert_eq!(
            classifier.classify(&report),
            SkillClassificationType::LlmReasoning
        );
    }

    /// T-SK-S4-04: Classifier assigns `ApiCall` for API-description skills.
    #[test]
    fn test_classify_api_call() {
        let classifier = SkillClassifier::new();
        let mut report = base_report();
        report.classification_hint = "api_call".to_string();
        report.embedded_tools.push(crate::analyzer::EmbeddedTool {
            name: "api_tool".to_string(),
            tool_type: "api_endpoint".to_string(),
            description: "Calls an API".to_string(),
        });

        assert_eq!(
            classifier.classify(&report),
            SkillClassificationType::ApiCall
        );
    }

    /// Hybrid detection when reasoning + tools present.
    #[test]
    fn test_classify_hybrid() {
        let classifier = SkillClassifier::new();
        let mut report = base_report();
        report.classification_hint = "hybrid".to_string();
        assert_eq!(
            classifier.classify(&report),
            SkillClassificationType::Hybrid
        );
    }

    /// Agent behavior classification.
    #[test]
    fn test_classify_agent_behavior() {
        let classifier = SkillClassifier::new();
        let mut report = base_report();
        report.classification_hint = "agent_behavior".to_string();
        assert_eq!(
            classifier.classify(&report),
            SkillClassificationType::AgentBehavior
        );
    }
}
