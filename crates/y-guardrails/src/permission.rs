//! Unified permission model for tool execution.
//!
//! Four actions: `Allow`, `Notify`, `Ask`, `Deny`.
//! Per-tool overrides take precedence over global defaults.
//! Dangerous tools auto-escalate to `Ask` unless explicitly overridden.

use serde::{Deserialize, Serialize};

use crate::config::GuardrailConfig;

/// Permission action for a tool execution request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionAction {
    /// Execute immediately, no restrictions.
    Allow,
    /// Execute, but log/emit an event for auditing.
    Notify,
    /// Pause execution and ask the user via HITL.
    Ask,
    /// Block execution entirely.
    Deny,
}

/// Permission decision with context.
#[derive(Debug, Clone)]
pub struct PermissionDecision {
    /// The resolved action.
    pub action: PermissionAction,
    /// Why this action was chosen (e.g., "per-tool override", "dangerous auto-ask").
    pub reason: String,
}

/// Evaluates permission for a given tool based on the guardrail config.
#[derive(Debug)]
pub struct PermissionModel {
    config: GuardrailConfig,
}

impl PermissionModel {
    /// Create a new permission model with the given config.
    pub fn new(config: GuardrailConfig) -> Self {
        Self { config }
    }

    /// Evaluate the permission for a tool.
    ///
    /// Resolution order:
    /// 1. Per-tool override (if present) — always wins
    /// 2. Dangerous auto-ask (if `is_dangerous` and `dangerous_auto_ask` enabled)
    /// 3. Global default
    pub fn evaluate(&self, tool_name: &str, is_dangerous: bool) -> PermissionDecision {
        // 1. Per-tool override
        if let Some(&action) = self.config.tool_permissions.get(tool_name) {
            return PermissionDecision {
                action,
                reason: format!("per-tool override for `{tool_name}`"),
            };
        }

        // 2. Dangerous auto-ask
        if is_dangerous && self.config.dangerous_auto_ask {
            return PermissionDecision {
                action: PermissionAction::Ask,
                reason: format!("`{tool_name}` is dangerous — auto-escalated to ask"),
            };
        }

        // 3. Global default
        PermissionDecision {
            action: self.config.default_permission,
            reason: "global default policy".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_with_defaults() -> GuardrailConfig {
        GuardrailConfig::default()
    }

    /// T-GUARD-001-01: Tool with `allow` policy passes.
    #[test]
    fn test_permission_allow_passes() {
        let mut config = config_with_defaults();
        config
            .tool_permissions
            .insert("read_file".to_string(), PermissionAction::Allow);

        let model = PermissionModel::new(config);
        let decision = model.evaluate("read_file", false);

        assert_eq!(decision.action, PermissionAction::Allow);
    }

    /// T-GUARD-001-02: Tool with `deny` policy blocks.
    #[test]
    fn test_permission_deny_blocks() {
        let mut config = config_with_defaults();
        config
            .tool_permissions
            .insert("rm_rf".to_string(), PermissionAction::Deny);

        let model = PermissionModel::new(config);
        let decision = model.evaluate("rm_rf", false);

        assert_eq!(decision.action, PermissionAction::Deny);
        assert!(decision.reason.contains("per-tool override"));
    }

    /// T-GUARD-001-03: Tool with `ask` policy triggers HITL.
    #[test]
    fn test_permission_ask_triggers_hitl() {
        let mut config = config_with_defaults();
        config
            .tool_permissions
            .insert("deploy".to_string(), PermissionAction::Ask);

        let model = PermissionModel::new(config);
        let decision = model.evaluate("deploy", false);

        assert_eq!(decision.action, PermissionAction::Ask);
    }

    /// T-GUARD-001-04: Tool with `notify` policy executes but emits event.
    #[test]
    fn test_permission_notify_logs_and_continues() {
        let mut config = config_with_defaults();
        config
            .tool_permissions
            .insert("write_file".to_string(), PermissionAction::Notify);

        let model = PermissionModel::new(config);
        let decision = model.evaluate("write_file", false);

        assert_eq!(decision.action, PermissionAction::Notify);
    }

    /// T-GUARD-001-05: Dangerous tool with no explicit policy defaults to `ask`.
    #[test]
    fn test_permission_dangerous_tool_requires_ask() {
        let config = config_with_defaults();
        let model = PermissionModel::new(config);
        let decision = model.evaluate("ShellExec", true);

        assert_eq!(decision.action, PermissionAction::Ask);
        assert!(decision.reason.contains("dangerous"));
    }

    /// T-GUARD-001-06: Per-tool override wins over global default.
    #[test]
    fn test_permission_per_tool_override() {
        let mut config = config_with_defaults();
        config.default_permission = PermissionAction::Allow;
        config
            .tool_permissions
            .insert("special_tool".to_string(), PermissionAction::Deny);

        let model = PermissionModel::new(config);
        let decision = model.evaluate("special_tool", false);

        assert_eq!(decision.action, PermissionAction::Deny);
        assert!(decision.reason.contains("per-tool override"));
    }
}
