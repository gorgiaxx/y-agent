//! Filter gate: decides whether a skill should be accepted, rejected, or partially accepted.
//!
//! Combines analysis report, classification, and security verdict to produce
//! a final decision with optional redirect suggestions.

use serde::{Deserialize, Serialize};

use crate::analyzer::AnalysisReport;
use crate::classifier::SkillClassificationType;
use crate::security::SecurityVerdict;

/// Where to redirect non-skill content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RedirectTarget {
    /// Should be registered as a tool instead.
    ToolSystem,
    /// Should be an agent behavior/workflow.
    AgentFramework,
}

impl std::fmt::Display for RedirectTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ToolSystem => f.write_str("Tool System"),
            Self::AgentFramework => f.write_str("Agent Framework"),
        }
    }
}

/// Filter decision for a skill submission.
#[derive(Debug, Clone)]
pub enum FilterDecision {
    /// Skill accepted for processing.
    Accepted,
    /// Skill rejected entirely.
    Rejected {
        /// Why it was rejected.
        reason: String,
        /// Where the content could be redirected.
        redirect: Option<RedirectTarget>,
    },
    /// Partially accepted: LLM reasoning portion kept, non-skill parts redirected.
    PartialAccept {
        /// Fraction of content that is LLM-instruction-only (0.0-1.0).
        llm_portion: f64,
        /// Non-skill parts and where to redirect them.
        redirect_for: Vec<RedirectTarget>,
    },
}

impl FilterDecision {
    /// Returns true if the decision is `Accepted`.
    pub fn is_accepted(&self) -> bool {
        matches!(self, Self::Accepted)
    }

    /// Returns true if the decision is `Rejected`.
    pub fn is_rejected(&self) -> bool {
        matches!(self, Self::Rejected { .. })
    }
}

/// Filter gate configuration.
#[derive(Debug, Clone)]
pub struct FilterConfig {
    /// Maximum quality issues before rejection.
    pub max_quality_issues: usize,
    /// Maximum token estimate before quality warning.
    pub max_tokens: u32,
}

impl Default for FilterConfig {
    fn default() -> Self {
        Self {
            max_quality_issues: 5,
            max_tokens: 10_000,
        }
    }
}

/// Applies filter rules to decide whether to accept a skill.
#[derive(Debug)]
pub struct FilterGate {
    config: FilterConfig,
}

impl FilterGate {
    /// Create a new filter gate with default configuration.
    pub fn new() -> Self {
        Self {
            config: FilterConfig::default(),
        }
    }

    /// Create a filter gate with custom configuration.
    pub fn with_config(config: FilterConfig) -> Self {
        Self { config }
    }

    /// Apply filter rules and produce a decision.
    pub fn filter(
        &self,
        report: &AnalysisReport,
        classification: SkillClassificationType,
        security: &SecurityVerdict,
    ) -> FilterDecision {
        // Rule 1: Security blocks are absolute
        if let SecurityVerdict::Blocked { reason, .. } = security {
            return FilterDecision::Rejected {
                reason: format!("Security blocked: {reason}"),
                redirect: None,
            };
        }

        // Rule 2: API-only skills should be tools
        if classification == SkillClassificationType::ApiCall && report.embedded_scripts.is_empty()
        {
            return FilterDecision::Rejected {
                reason: "Pure API call content should be registered as a tool".to_string(),
                redirect: Some(RedirectTarget::ToolSystem),
            };
        }

        // Rule 3: Pure tool wrappers should be tools
        if classification == SkillClassificationType::ToolWrapper && report.capabilities.is_empty()
        {
            return FilterDecision::Rejected {
                reason: "Tool wrapper without reasoning capabilities should be a tool".to_string(),
                redirect: Some(RedirectTarget::ToolSystem),
            };
        }

        // Rule 4: Quality check (too many issues = reject)
        if report.quality_issues.len() > self.config.max_quality_issues {
            return FilterDecision::Rejected {
                reason: format!(
                    "Too many quality issues ({} found, max {})",
                    report.quality_issues.len(),
                    self.config.max_quality_issues
                ),
                redirect: None,
            };
        }

        // Rule 5: Oversized content rejection
        if report.token_estimate > self.config.max_tokens {
            return FilterDecision::Rejected {
                reason: format!(
                    "Content too large ({} tokens, max {})",
                    report.token_estimate, self.config.max_tokens
                ),
                redirect: None,
            };
        }

        // Rule 6: Hybrid content gets partial acceptance
        if classification == SkillClassificationType::Hybrid {
            let total_items = report.embedded_tools.len() + report.embedded_scripts.len();
            let llm_portion = if total_items > 0 {
                // Rough estimate: ratio of reasoning content
                let items_f = total_items as f64;
                1.0 - (items_f * 0.1).min(0.5)
            } else {
                1.0
            };

            let mut redirects = Vec::new();
            if !report.embedded_tools.is_empty() {
                redirects.push(RedirectTarget::ToolSystem);
            }
            if !report.embedded_scripts.is_empty() {
                redirects.push(RedirectTarget::ToolSystem);
            }

            if !redirects.is_empty() {
                return FilterDecision::PartialAccept {
                    llm_portion,
                    redirect_for: redirects,
                };
            }
        }

        // Rule 7: Everything else accepted
        FilterDecision::Accepted
    }
}

impl Default for FilterGate {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::{AnalysisReport, EmbeddedTool, SecurityFlags};
    use crate::security::{SecurityFinding, SecurityFindingType};

    fn base_report() -> AnalysisReport {
        AnalysisReport {
            purpose: "test skill".to_string(),
            classification_hint: "llm_reasoning".to_string(),
            capabilities: vec!["reasoning".to_string()],
            embedded_tools: vec![],
            embedded_scripts: vec![],
            quality_issues: vec![],
            token_estimate: 500,
            security_flags: SecurityFlags::default(),
        }
    }

    /// T-SK-S4-07: Filter gate accepts `LlmReasoning` + secure skills.
    #[test]
    fn test_filter_accepts_secure_llm_reasoning() {
        let gate = FilterGate::new();
        let report = base_report();

        let decision = gate.filter(
            &report,
            SkillClassificationType::LlmReasoning,
            &SecurityVerdict::Pass,
        );
        assert!(decision.is_accepted());
    }

    /// T-SK-S4-08: Filter gate rejects `ApiCall` with redirect message.
    #[test]
    fn test_filter_rejects_api_call_with_redirect() {
        let gate = FilterGate::new();
        let mut report = base_report();
        report.embedded_tools.push(EmbeddedTool {
            name: "api".to_string(),
            tool_type: "api_endpoint".to_string(),
            description: "An API call".to_string(),
        });

        let decision = gate.filter(
            &report,
            SkillClassificationType::ApiCall,
            &SecurityVerdict::Pass,
        );

        assert!(decision.is_rejected());
        if let FilterDecision::Rejected { redirect, .. } = &decision {
            assert_eq!(*redirect, Some(RedirectTarget::ToolSystem));
        }
    }

    /// T-SK-S4-09: Filter gate handles hybrid: partial accept + redirect.
    #[test]
    fn test_filter_partial_accept_hybrid() {
        let gate = FilterGate::new();
        let mut report = base_report();
        report.embedded_tools.push(EmbeddedTool {
            name: "helper_api".to_string(),
            tool_type: "api_endpoint".to_string(),
            description: "Helper".to_string(),
        });

        let decision = gate.filter(
            &report,
            SkillClassificationType::Hybrid,
            &SecurityVerdict::Pass,
        );

        if let FilterDecision::PartialAccept {
            llm_portion,
            redirect_for,
        } = &decision
        {
            assert!(*llm_portion > 0.0 && *llm_portion < 1.0);
            assert!(redirect_for.contains(&RedirectTarget::ToolSystem));
        } else {
            panic!("expected PartialAccept, got {decision:?}");
        }
    }

    /// T-SK-S4-10: Quality block triggers for oversized + low-quality skills.
    #[test]
    fn test_filter_rejects_oversized() {
        let config = FilterConfig {
            max_quality_issues: 2,
            max_tokens: 1000,
        };
        let gate = FilterGate::with_config(config);
        let mut report = base_report();
        report.token_estimate = 5000;

        let decision = gate.filter(
            &report,
            SkillClassificationType::LlmReasoning,
            &SecurityVerdict::Pass,
        );

        assert!(decision.is_rejected());
    }

    /// Security block overrides all other rules.
    #[test]
    fn test_filter_security_block_overrides() {
        let gate = FilterGate::new();
        let report = base_report();

        let blocked = SecurityVerdict::Blocked {
            reason: "prompt injection detected".to_string(),
            finding_type: SecurityFindingType::PromptInjection,
            findings: vec![SecurityFinding {
                finding_type: SecurityFindingType::PromptInjection,
                description: "test".to_string(),
                severity: 5,
                line: Some(1),
            }],
        };

        let decision = gate.filter(&report, SkillClassificationType::LlmReasoning, &blocked);

        assert!(decision.is_rejected());
    }
}
