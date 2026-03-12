//! Risk scoring: composite risk assessment from tool properties.
//!
//! Risk score is a 0.0–1.0 float, composed from multiple factors:
//! - Tool category (shell/network higher than filesystem/search)
//! - `is_dangerous` flag
//! - Runtime capability requirements
//! - Custom risk factors

use crate::config::RiskConfig;

/// Risk factors for a tool execution request.
#[derive(Debug, Clone)]
pub struct RiskFactors {
    /// Whether the tool is marked as dangerous.
    pub is_dangerous: bool,
    /// Tool category name (e.g., "shell", "filesystem", "search").
    pub category: String,
    /// Whether the tool requires network access.
    pub requires_network: bool,
    /// Whether the tool requires filesystem write access.
    pub requires_fs_write: bool,
    /// Optional custom risk weight (0.0–1.0).
    pub custom_risk: Option<f32>,
}

/// Result of risk assessment.
#[derive(Debug, Clone)]
pub struct RiskAssessment {
    /// Composite risk score (0.0–1.0).
    pub score: f32,
    /// Whether the score exceeds the escalation threshold.
    pub requires_escalation: bool,
    /// Breakdown of contributing factors.
    pub factors: Vec<String>,
}

/// Computes composite risk scores for tool execution.
#[derive(Debug)]
pub struct RiskScorer {
    config: RiskConfig,
}

impl RiskScorer {
    /// Create a new risk scorer with the given config.
    pub fn new(config: RiskConfig) -> Self {
        Self { config }
    }

    /// Score the risk of a tool execution based on its factors.
    pub fn score(&self, factors: &RiskFactors) -> RiskAssessment {
        let mut score: f32 = 0.0;
        let mut breakdown = Vec::new();

        // Factor 1: is_dangerous flag (weight: 0.4)
        if factors.is_dangerous {
            score += 0.4;
            breakdown.push("dangerous tool (+0.4)".to_string());
        }

        // Factor 2: category risk (weight: 0.0–0.3)
        let category_risk = match factors.category.as_str() {
            "shell" => 0.3,
            "network" => 0.2,
            "agent" => 0.15,
            "filesystem" | "workflow" => 0.1,
            "search" | "memory" | "knowledge" => 0.0,
            _ => 0.05, // Unknown category gets a moderate-low score
        };
        if category_risk > 0.0 {
            score += category_risk;
            breakdown.push(format!(
                "category `{}` (+{category_risk})",
                factors.category
            ));
        }

        // Factor 3: network access (weight: 0.1)
        if factors.requires_network {
            score += 0.1;
            breakdown.push("requires network (+0.1)".to_string());
        }

        // Factor 4: filesystem write (weight: 0.1)
        if factors.requires_fs_write {
            score += 0.1;
            breakdown.push("requires fs write (+0.1)".to_string());
        }

        // Factor 5: custom risk
        if let Some(custom) = factors.custom_risk {
            score += custom;
            breakdown.push(format!("custom risk (+{custom})"));
        }

        // Clamp to 0.0–1.0
        score = score.clamp(0.0, 1.0);

        let requires_escalation = score >= self.config.escalation_threshold;

        RiskAssessment {
            score,
            requires_escalation,
            factors: breakdown,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_config() -> RiskConfig {
        RiskConfig::default()
    }

    /// T-GUARD-004-01: Read-only tool gets low risk score.
    #[test]
    fn test_risk_score_safe_tool() {
        let scorer = RiskScorer::new(default_config());
        let factors = RiskFactors {
            is_dangerous: false,
            category: "search".to_string(),
            requires_network: false,
            requires_fs_write: false,
            custom_risk: None,
        };

        let assessment = scorer.score(&factors);
        assert!(
            assessment.score < 0.3,
            "safe tool should have low risk: {}",
            assessment.score
        );
        assert!(!assessment.requires_escalation);
    }

    /// T-GUARD-004-02: Shell execution tool gets high risk score.
    #[test]
    fn test_risk_score_dangerous_tool() {
        let scorer = RiskScorer::new(default_config());
        let factors = RiskFactors {
            is_dangerous: true,
            category: "shell".to_string(),
            requires_network: false,
            requires_fs_write: true,
            custom_risk: None,
        };

        let assessment = scorer.score(&factors);
        assert!(
            assessment.score >= 0.7,
            "dangerous shell tool should have high risk: {}",
            assessment.score
        );
        assert!(assessment.requires_escalation);
    }

    /// T-GUARD-004-03: Multiple factors combine correctly.
    #[test]
    fn test_risk_score_composite() {
        let scorer = RiskScorer::new(default_config());
        let factors = RiskFactors {
            is_dangerous: false,
            category: "network".to_string(),
            requires_network: true,
            requires_fs_write: false,
            custom_risk: Some(0.1),
        };

        let assessment = scorer.score(&factors);
        // network (0.2) + requires_network (0.1) + custom (0.1) = 0.4
        let expected = 0.4;
        assert!(
            (assessment.score - expected).abs() < 0.01,
            "composite score should be ~{expected}: got {}",
            assessment.score
        );
    }

    /// T-GUARD-004-04: Score exceeding threshold triggers escalation.
    #[test]
    fn test_risk_threshold_escalation() {
        let config = RiskConfig {
            escalation_threshold: 0.5,
        };
        let scorer = RiskScorer::new(config);
        let factors = RiskFactors {
            is_dangerous: true,
            category: "shell".to_string(),
            requires_network: false,
            requires_fs_write: false,
            custom_risk: None,
        };

        let assessment = scorer.score(&factors);
        assert!(assessment.score >= 0.5);
        assert!(assessment.requires_escalation);
    }
}
