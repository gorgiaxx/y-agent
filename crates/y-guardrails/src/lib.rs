//! y-guardrails: Safety validators, `LoopGuard`, taint tracking, risk scoring, HITL.
//!
//! All guardrails are implemented as [`Middleware`](y_core::hook::Middleware) in the
//! y-hooks chains (Tool and LLM), not as a parallel system.
//!
//! # Components
//!
//! - [`permission_pipeline`] — authoritative allow/notify/ask/deny evaluation
//! - [`permission::PermissionModel`] — compatibility evaluator for config-only callers
//! - [`loop_guard::LoopGuard`] — 4 pattern detectors (repetition, oscillation, drift, redundant)
//! - [`taint::TaintTracker`] — data flow taint propagation and sink blocking
//! - [`risk::RiskScorer`] — composite risk assessment from tool properties
//! - [`hitl::HitlProtocol`] — human-in-the-loop escalation with timeout
//!
//! # Middleware
//!
//! - [`middleware::tool_guard::ToolGuardMiddleware`] — pre-execution permission + risk (priority 10)
//! - [`middleware::llm_guard::LlmGuardMiddleware`] — output safety validation (priority 900)
//! - [`middleware::loop_detector::LoopDetectorMiddleware`] — loop detection (priority 20)

pub mod exec_policy;
// Re-export primary exec_policy types.
pub use exec_policy::{
    ExecDecision, ExecPolicyError, ExecPolicyManager, ExecPolicyResult, Policy, PolicyParser,
};
pub mod config;
pub mod error;
pub mod hitl;
pub mod loop_guard;
pub mod middleware;
pub mod mode_manager;
pub mod permission;
pub mod permission_pipeline;
pub mod risk;
pub mod rule_store;
pub mod taint;

// Re-export primary types.
pub use config::{ExecPolicyConfig, GuardrailConfig, PlanReviewConfig, PlanReviewMode};
pub use error::{GuardrailError, LoopPattern};
pub use hitl::{HitlHandler, HitlProtocol, HitlRequest, HitlResponse};
pub use loop_guard::{ActionRecord, LoopDetection, LoopGuard};
pub use middleware::llm_guard::LlmGuardMiddleware;
pub use middleware::loop_detector::LoopDetectorMiddleware;
pub use middleware::tool_guard::ToolGuardMiddleware;
pub use mode_manager::PermissionModeManager;
pub use permission::{PermissionAction, PermissionDecision, PermissionModel};
pub use permission_pipeline::{evaluate_pipeline, ToolPermissionRequest};
pub use risk::{RiskAssessment, RiskFactors, RiskScorer};
pub use rule_store::PermissionRuleStore;
pub use taint::{TaintCheckResult, TaintTag, TaintTracker};

/// Convenience builder that creates all guardrail middleware from a single config.
///
/// The inner config is wrapped in `RwLock` so it can be hot-reloaded at
/// runtime without restarting the application.
pub struct GuardrailManager {
    config: std::sync::RwLock<GuardrailConfig>,
    rule_store: std::sync::Arc<std::sync::RwLock<PermissionRuleStore>>,
}

impl std::fmt::Debug for GuardrailManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let cfg = self
            .config
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        f.debug_struct("GuardrailManager")
            .field("config", &*cfg)
            .field("rule_store", &self.rule_store)
            .finish()
    }
}

impl GuardrailManager {
    /// Create a new guardrail manager.
    pub fn new(config: GuardrailConfig) -> Self {
        Self {
            config: std::sync::RwLock::new(config),
            rule_store: std::sync::Arc::new(std::sync::RwLock::new(PermissionRuleStore::new())),
        }
    }

    /// Create a guardrail manager with an existing layered permission rule store.
    pub fn with_rule_store(
        config: GuardrailConfig,
        rule_store: std::sync::Arc<std::sync::RwLock<PermissionRuleStore>>,
    ) -> Self {
        Self {
            config: std::sync::RwLock::new(config),
            rule_store,
        }
    }

    /// Get a snapshot of the current guardrail configuration.
    pub fn config(&self) -> GuardrailConfig {
        self.config
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Evaluate one tool invocation through the authoritative permission pipeline.
    pub fn evaluate_tool_permission(
        &self,
        request: ToolPermissionRequest<'_>,
    ) -> y_core::permission_types::PermissionResult {
        let context = self.permission_context(request.mode);

        permission_pipeline::evaluate_pipeline_with_exec_policy(
            request.tool_name,
            request.input_content,
            request.is_dangerous,
            request.tool_result,
            &context,
            request.exec_policy,
        )
    }

    /// Build the immutable permission context for a session-scoped evaluation.
    pub fn permission_context(
        &self,
        mode: Option<y_core::permission_types::PermissionMode>,
    ) -> y_core::permission_types::PermissionContext {
        use y_core::permission_types::{
            PermissionBehavior, PermissionRule, PermissionRuleSource, PermissionRuleTarget,
        };

        let config = self.config();
        let config_rules = config
            .tool_permissions
            .iter()
            .map(|(tool_name, action)| {
                let behavior = match action {
                    PermissionAction::Allow => PermissionBehavior::Allow,
                    PermissionAction::Notify => PermissionBehavior::Notify,
                    PermissionAction::Ask => PermissionBehavior::Ask,
                    PermissionAction::Deny => PermissionBehavior::Deny,
                };
                PermissionRule::new(
                    PermissionRuleSource::GlobalSettings,
                    behavior,
                    PermissionRuleTarget::tool(tool_name),
                )
            })
            .collect::<Vec<_>>();
        let mut context = self
            .rule_store
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .build_context(mode);
        context.rules.extend(config_rules);
        context.default_behavior = match config.default_permission {
            PermissionAction::Allow => PermissionBehavior::Allow,
            PermissionAction::Notify => PermissionBehavior::Notify,
            PermissionAction::Ask => PermissionBehavior::Ask,
            PermissionAction::Deny => PermissionBehavior::Deny,
        };
        context.dangerous_auto_ask = config.dangerous_auto_ask;
        context
    }

    /// Get the shared layered permission rule store.
    pub fn rule_store(&self) -> &std::sync::Arc<std::sync::RwLock<PermissionRuleStore>> {
        &self.rule_store
    }

    /// Hot-reload the guardrail configuration.
    ///
    /// Atomically replaces the current config. Subsequent calls to
    /// `config()` and middleware constructors will use the new values.
    pub fn reload_config(&self, new_config: GuardrailConfig) {
        let mut guard = self
            .config
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *guard = new_config;
        tracing::info!("Guardrail config hot-reloaded");
    }

    /// Create the `ToolGuardMiddleware`.
    pub fn tool_guard(&self) -> ToolGuardMiddleware {
        ToolGuardMiddleware::new(self.config())
    }

    /// Create the `LoopDetectorMiddleware`.
    pub fn loop_detector(&self) -> LoopDetectorMiddleware {
        LoopDetectorMiddleware::new(self.config().loop_guard.clone())
    }

    /// Create the `LlmGuardMiddleware`.
    pub fn llm_guard(&self) -> LlmGuardMiddleware {
        LlmGuardMiddleware::new()
    }

    /// Create a new `PermissionModel` bound to this config.
    pub fn permission_model(&self) -> PermissionModel {
        PermissionModel::new(self.config())
    }

    /// Create a new `RiskScorer` bound to this config.
    pub fn risk_scorer(&self) -> RiskScorer {
        RiskScorer::new(self.config().risk.clone())
    }

    /// Create a new HITL protocol pair.
    pub fn hitl_protocol(&self) -> (HitlProtocol, HitlHandler) {
        HitlProtocol::new(self.config().hitl.clone())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, RwLock};

    use y_core::permission_types::{
        PermissionBehavior, PermissionMode, PermissionResult, PermissionRule, PermissionRuleSource,
        PermissionRuleTarget,
    };

    use super::*;

    #[test]
    fn guardrail_manager_bypass_does_not_override_configured_deny() {
        let mut config = GuardrailConfig::default();
        config
            .tool_permissions
            .insert("ShellExec".to_string(), PermissionAction::Deny);
        let manager = GuardrailManager::new(config);
        let tool_result = PermissionResult::passthrough();

        let result = manager.evaluate_tool_permission(
            ToolPermissionRequest::new("ShellExec", false, &tool_result)
                .with_mode(PermissionMode::BypassPermissions),
        );

        assert_eq!(result.behavior, PermissionBehavior::Deny);
    }

    #[test]
    fn guardrail_manager_preserves_configured_default_allow() {
        let manager = GuardrailManager::new(GuardrailConfig::default());
        let tool_result = PermissionResult::passthrough();

        let result = manager.evaluate_tool_permission(ToolPermissionRequest::new(
            "FileWrite",
            false,
            &tool_result,
        ));

        assert_eq!(result.behavior, PermissionBehavior::Allow);
    }

    #[test]
    fn guardrail_manager_bypass_does_not_override_default_deny() {
        let config = GuardrailConfig {
            default_permission: PermissionAction::Deny,
            ..GuardrailConfig::default()
        };
        let manager = GuardrailManager::new(config);
        let tool_result = PermissionResult::passthrough();

        let result = manager.evaluate_tool_permission(
            ToolPermissionRequest::new("FileWrite", false, &tool_result)
                .with_mode(PermissionMode::BypassPermissions),
        );

        assert_eq!(result.behavior, PermissionBehavior::Deny);
    }

    #[test]
    fn guardrail_manager_preserves_configured_default_notify() {
        let config = GuardrailConfig {
            default_permission: PermissionAction::Notify,
            ..GuardrailConfig::default()
        };
        let manager = GuardrailManager::new(config);
        let tool_result = PermissionResult::passthrough();

        let result = manager.evaluate_tool_permission(ToolPermissionRequest::new(
            "FileWrite",
            false,
            &tool_result,
        ));

        assert_eq!(result.behavior, PermissionBehavior::Notify);
    }

    #[test]
    fn guardrail_manager_respects_disabled_dangerous_auto_ask() {
        let config = GuardrailConfig {
            dangerous_auto_ask: false,
            ..GuardrailConfig::default()
        };
        let manager = GuardrailManager::new(config);
        let tool_result = PermissionResult::passthrough();

        let result = manager.evaluate_tool_permission(ToolPermissionRequest::new(
            "ShellExec",
            true,
            &tool_result,
        ));

        assert_eq!(result.behavior, PermissionBehavior::Allow);
    }

    #[test]
    fn guardrail_manager_escalates_dangerous_tool_when_enabled() {
        let manager = GuardrailManager::new(GuardrailConfig::default());
        let tool_result = PermissionResult::passthrough();

        let result = manager.evaluate_tool_permission(ToolPermissionRequest::new(
            "ShellExec",
            true,
            &tool_result,
        ));

        assert_eq!(result.behavior, PermissionBehavior::Ask);
    }

    #[test]
    fn guardrail_manager_evaluates_rules_from_rule_store() {
        let mut store = PermissionRuleStore::new();
        store.add_session_rule(PermissionRule::new(
            PermissionRuleSource::Session,
            PermissionBehavior::Deny,
            PermissionRuleTarget::tool("FileWrite"),
        ));
        let manager = GuardrailManager::with_rule_store(
            GuardrailConfig::default(),
            Arc::new(RwLock::new(store)),
        );
        let tool_result = PermissionResult::passthrough();

        let result = manager.evaluate_tool_permission(ToolPermissionRequest::new(
            "FileWrite",
            false,
            &tool_result,
        ));

        assert_eq!(result.behavior, PermissionBehavior::Deny);
    }
}
