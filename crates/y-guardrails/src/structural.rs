//! Structural guardrails: config-time validation of agent/workflow structure.
//!
//! Validates agent and workflow configurations before runtime to catch
//! structural issues early. Checks include:
//! - Tool capability alignment (tools must support required capabilities)
//! - Circular dependency detection in workflows
//! - Permission consistency (tools used must have permission entries)
//! - Resource budget validation (total cost/token limits)
//!
//! Design reference: guardrails-design.md §Structural Validation

use std::collections::{HashMap, HashSet};

/// A structural validation rule.
#[derive(Debug, Clone)]
pub struct StructuralRule {
    /// Unique rule identifier.
    pub id: String,
    /// Human-readable description.
    pub description: String,
    /// Severity of the issue if rule is violated.
    pub severity: Severity,
}

/// Severity level for structural violations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Informational — no action needed.
    Info,
    /// Warning — should be addressed but not blocking.
    Warning,
    /// Error — must be resolved before execution.
    Error,
}

/// A structural violation found during validation.
#[derive(Debug, Clone)]
pub struct StructuralViolation {
    /// Rule that was violated.
    pub rule_id: String,
    /// Severity of the violation.
    pub severity: Severity,
    /// Human-readable violation message.
    pub message: String,
    /// Resource or component that caused the violation.
    pub source: String,
}

/// Result of structural validation.
#[derive(Debug, Clone)]
pub struct ValidationResult {
    /// All violations found.
    pub violations: Vec<StructuralViolation>,
}

impl ValidationResult {
    /// Create an empty (passing) result.
    pub fn ok() -> Self {
        Self {
            violations: Vec::new(),
        }
    }

    /// Returns true if there are no error-level violations.
    pub fn is_valid(&self) -> bool {
        !self
            .violations
            .iter()
            .any(|v| v.severity == Severity::Error)
    }

    /// Returns true if there are no violations at all.
    pub fn is_clean(&self) -> bool {
        self.violations.is_empty()
    }

    /// Get violations filtered by severity.
    pub fn by_severity(&self, severity: Severity) -> Vec<&StructuralViolation> {
        self.violations
            .iter()
            .filter(|v| v.severity == severity)
            .collect()
    }

    /// Number of error-level violations.
    pub fn error_count(&self) -> usize {
        self.by_severity(Severity::Error).len()
    }

    /// Number of warning-level violations.
    pub fn warning_count(&self) -> usize {
        self.by_severity(Severity::Warning).len()
    }
}

/// Structural guardrails validator.
///
/// Performs config-time validation of agent and workflow structures.
#[derive(Debug, Default)]
pub struct StructuralValidator {
    rules: Vec<StructuralRule>,
}

impl StructuralValidator {
    /// Create a new validator with default rules.
    pub fn new() -> Self {
        Self { rules: Vec::new() }
    }

    /// Add a custom validation rule.
    pub fn add_rule(&mut self, rule: StructuralRule) {
        self.rules.push(rule);
    }

    /// Validate a workflow DAG for circular dependencies.
    ///
    /// `edges` maps each step ID to the step IDs it depends on.
    pub fn validate_dag(&self, edges: &HashMap<String, Vec<String>>) -> ValidationResult {
        let mut result = ValidationResult::ok();

        // Detect cycles using DFS with coloring.
        let mut visited = HashSet::new();
        let mut in_stack = HashSet::new();

        for node in edges.keys() {
            if !visited.contains(node.as_str()) {
                if let Some(cycle) =
                    Self::dfs_detect_cycle(node, edges, &mut visited, &mut in_stack)
                {
                    result.violations.push(StructuralViolation {
                        rule_id: "dag_no_cycles".into(),
                        severity: Severity::Error,
                        message: format!("Circular dependency detected: {}", cycle.join(" → ")),
                        source: node.clone(),
                    });
                }
            }
        }

        result
    }

    /// DFS cycle detection. Returns the cycle path if found.
    fn dfs_detect_cycle(
        node: &str,
        edges: &HashMap<String, Vec<String>>,
        visited: &mut HashSet<String>,
        in_stack: &mut HashSet<String>,
    ) -> Option<Vec<String>> {
        visited.insert(node.to_string());
        in_stack.insert(node.to_string());

        if let Some(neighbors) = edges.get(node) {
            for neighbor in neighbors {
                if !visited.contains(neighbor.as_str()) {
                    if let Some(mut cycle) =
                        Self::dfs_detect_cycle(neighbor, edges, visited, in_stack)
                    {
                        cycle.insert(0, node.to_string());
                        return Some(cycle);
                    }
                } else if in_stack.contains(neighbor.as_str()) {
                    return Some(vec![node.to_string(), neighbor.clone()]);
                }
            }
        }

        in_stack.remove(node);
        None
    }

    /// Validate that all referenced tools exist in the registry.
    pub fn validate_tool_references(
        &self,
        used_tools: &[String],
        available_tools: &HashSet<String>,
    ) -> ValidationResult {
        let mut result = ValidationResult::ok();

        for tool in used_tools {
            if !available_tools.contains(tool) {
                result.violations.push(StructuralViolation {
                    rule_id: "tool_exists".into(),
                    severity: Severity::Error,
                    message: format!("Referenced tool '{tool}' not found in registry"),
                    source: tool.clone(),
                });
            }
        }

        result
    }

    /// Validate that all used tools have corresponding permission entries.
    pub fn validate_permission_coverage(
        &self,
        used_tools: &[String],
        permission_entries: &HashSet<String>,
    ) -> ValidationResult {
        let mut result = ValidationResult::ok();

        for tool in used_tools {
            if !permission_entries.contains(tool) {
                result.violations.push(StructuralViolation {
                    rule_id: "permission_coverage".into(),
                    severity: Severity::Warning,
                    message: format!(
                        "Tool '{tool}' has no explicit permission entry; default policy will apply"
                    ),
                    source: tool.clone(),
                });
            }
        }

        result
    }

    /// Validate a token budget configuration.
    pub fn validate_token_budget(&self, budget: &TokenBudget) -> ValidationResult {
        let mut result = ValidationResult::ok();

        if budget.total_limit > 0 {
            let sum = budget.system_tokens
                + budget.tools_tokens
                + budget.history_tokens
                + budget.response_tokens;
            if sum > budget.total_limit {
                result.violations.push(StructuralViolation {
                    rule_id: "budget_sum".into(),
                    severity: Severity::Error,
                    message: format!(
                        "Token budget categories ({sum}) exceed total limit ({})",
                        budget.total_limit
                    ),
                    source: "token_budget".into(),
                });
            }
        }

        if budget.response_tokens == 0 {
            result.violations.push(StructuralViolation {
                rule_id: "budget_response".into(),
                severity: Severity::Warning,
                message: "Response token budget is 0; LLM may not generate output".into(),
                source: "token_budget".into(),
            });
        }

        result
    }
}

/// Token budget configuration for validation.
#[derive(Debug, Clone, Default)]
pub struct TokenBudget {
    pub total_limit: u32,
    pub system_tokens: u32,
    pub tools_tokens: u32,
    pub history_tokens: u32,
    pub response_tokens: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_dag_no_cycles() {
        let validator = StructuralValidator::new();
        let mut edges = HashMap::new();
        edges.insert("a".into(), vec!["b".into()]);
        edges.insert("b".into(), vec!["c".into()]);
        edges.insert("c".into(), vec![]);

        let result = validator.validate_dag(&edges);
        assert!(result.is_valid());
        assert!(result.is_clean());
    }

    #[test]
    fn test_validate_dag_with_cycle() {
        let validator = StructuralValidator::new();
        let mut edges = HashMap::new();
        edges.insert("a".into(), vec!["b".into()]);
        edges.insert("b".into(), vec!["c".into()]);
        edges.insert("c".into(), vec!["a".into()]);

        let result = validator.validate_dag(&edges);
        assert!(!result.is_valid());
        assert_eq!(result.error_count(), 1);
        assert!(result.violations[0].message.contains("Circular"));
    }

    #[test]
    fn test_validate_tool_references_all_present() {
        let validator = StructuralValidator::new();
        let used = vec!["WebSearch".into(), "FileRead".into()];
        let available: HashSet<String> = ["WebSearch", "FileRead", "calculator"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();

        let result = validator.validate_tool_references(&used, &available);
        assert!(result.is_valid());
    }

    #[test]
    fn test_validate_tool_references_missing() {
        let validator = StructuralValidator::new();
        let used = vec!["WebSearch".into(), "nonexistent_tool".into()];
        let available: HashSet<String> = ["WebSearch"].iter().map(|s| (*s).to_string()).collect();

        let result = validator.validate_tool_references(&used, &available);
        assert!(!result.is_valid());
        assert_eq!(result.error_count(), 1);
        assert!(result.violations[0].message.contains("nonexistent_tool"));
    }

    #[test]
    fn test_validate_permission_coverage() {
        let validator = StructuralValidator::new();
        let used = vec!["WebSearch".into(), "FileWrite".into()];
        let permissions: HashSet<String> = ["WebSearch"].iter().map(|s| (*s).to_string()).collect();

        let result = validator.validate_permission_coverage(&used, &permissions);
        assert!(result.is_valid()); // Warnings don't block.
        assert_eq!(result.warning_count(), 1);
        assert!(result.violations[0].message.contains("FileWrite"));
    }

    #[test]
    fn test_validate_token_budget_valid() {
        let validator = StructuralValidator::new();
        let budget = TokenBudget {
            total_limit: 4000,
            system_tokens: 500,
            tools_tokens: 500,
            history_tokens: 2000,
            response_tokens: 1000,
        };

        let result = validator.validate_token_budget(&budget);
        assert!(result.is_valid());
    }

    #[test]
    fn test_validate_token_budget_exceeds() {
        let validator = StructuralValidator::new();
        let budget = TokenBudget {
            total_limit: 4000,
            system_tokens: 2000,
            tools_tokens: 2000,
            history_tokens: 2000,
            response_tokens: 1000,
        };

        let result = validator.validate_token_budget(&budget);
        assert!(!result.is_valid());
        assert_eq!(result.error_count(), 1);
    }

    #[test]
    fn test_validate_token_budget_zero_response() {
        let validator = StructuralValidator::new();
        let budget = TokenBudget {
            total_limit: 4000,
            system_tokens: 500,
            tools_tokens: 500,
            history_tokens: 2000,
            response_tokens: 0,
        };

        let result = validator.validate_token_budget(&budget);
        assert!(result.is_valid()); // Warning, not error.
        assert_eq!(result.warning_count(), 1);
    }

    #[test]
    fn test_validation_result_ok() {
        let result = ValidationResult::ok();
        assert!(result.is_valid());
        assert!(result.is_clean());
        assert_eq!(result.error_count(), 0);
        assert_eq!(result.warning_count(), 0);
    }

    #[test]
    fn test_severity_ordering() {
        assert!(Severity::Info < Severity::Warning);
        assert!(Severity::Warning < Severity::Error);
    }

    #[test]
    fn test_add_custom_rule() {
        let mut validator = StructuralValidator::new();
        validator.add_rule(StructuralRule {
            id: "custom_check".into(),
            description: "Custom validation".into(),
            severity: Severity::Error,
        });
        assert_eq!(validator.rules.len(), 1);
    }
}
