//! `ToolGuardMiddleware`: pre-execution permission + risk check.
//!
//! This middleware sits in the Tool chain and evaluates permission
//! and risk before each tool execution. If the tool is denied or
//! requires HITL escalation, the chain is aborted.

use async_trait::async_trait;
use y_core::hook::{ChainType, Middleware, MiddlewareContext, MiddlewareError, MiddlewareResult};

use crate::config::GuardrailConfig;
use crate::permission::{PermissionAction, PermissionModel};
use crate::risk::{RiskFactors, RiskScorer};

/// Middleware that enforces permission and risk checks before tool execution.
///
/// Registered as a `ToolMiddleware` at priority 10 (runs early in the chain).
pub struct ToolGuardMiddleware {
    permission_model: PermissionModel,
    risk_scorer: RiskScorer,
}

impl ToolGuardMiddleware {
    /// Create a new tool guard middleware with the given config.
    pub fn new(config: GuardrailConfig) -> Self {
        let risk_scorer = RiskScorer::new(config.risk.clone());
        let permission_model = PermissionModel::new(config);
        Self {
            permission_model,
            risk_scorer,
        }
    }

    /// Extract tool name and danger flag from the middleware context payload.
    fn extract_tool_info(ctx: &MiddlewareContext) -> Option<(String, bool, String)> {
        let payload = &ctx.payload;
        let tool_name = payload.get("tool_name")?.as_str()?.to_string();
        let is_dangerous = payload
            .get("is_dangerous")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let category = payload
            .get("category")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("custom")
            .to_string();
        Some((tool_name, is_dangerous, category))
    }
}

impl std::fmt::Debug for ToolGuardMiddleware {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolGuardMiddleware").finish()
    }
}

#[async_trait]
impl Middleware for ToolGuardMiddleware {
    async fn execute(
        &self,
        ctx: &mut MiddlewareContext,
    ) -> Result<MiddlewareResult, MiddlewareError> {
        let (tool_name, is_dangerous, category) = Self::extract_tool_info(ctx)
            .unwrap_or_else(|| ("unknown".to_string(), false, "custom".to_string()));

        // 1. Check permission
        let decision = self.permission_model.evaluate(&tool_name, is_dangerous);

        match decision.action {
            PermissionAction::Deny => {
                ctx.abort(format!(
                    "Permission denied for tool `{tool_name}`: {}",
                    decision.reason
                ));
                return Ok(MiddlewareResult::ShortCircuit);
            }
            PermissionAction::Ask => {
                // In a real implementation, this would trigger HITL.
                // For now, we record the need for escalation in metadata.
                if let Some(meta) = ctx.metadata.as_object_mut() {
                    meta.insert("hitl_required".to_string(), serde_json::Value::Bool(true));
                    meta.insert(
                        "hitl_reason".to_string(),
                        serde_json::Value::String(decision.reason),
                    );
                }
            }
            PermissionAction::Notify => {
                // Record notification in metadata for event emission.
                if let Some(meta) = ctx.metadata.as_object_mut() {
                    meta.insert(
                        "permission_notify".to_string(),
                        serde_json::Value::Bool(true),
                    );
                }
            }
            PermissionAction::Allow => {} // No action needed
        }

        // 2. Check risk
        let risk_factors = RiskFactors {
            is_dangerous,
            category,
            requires_network: ctx
                .payload
                .get("requires_network")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
            requires_fs_write: ctx
                .payload
                .get("requires_fs_write")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
            custom_risk: None,
        };

        let assessment = self.risk_scorer.score(&risk_factors);
        if let Some(meta) = ctx.metadata.as_object_mut() {
            meta.insert(
                "risk_score".to_string(),
                serde_json::json!(assessment.score),
            );
        }

        if assessment.requires_escalation && decision.action != PermissionAction::Ask {
            // Risk threshold exceeded — escalate
            if let Some(meta) = ctx.metadata.as_object_mut() {
                meta.insert("hitl_required".to_string(), serde_json::Value::Bool(true));
                meta.insert(
                    "hitl_reason".to_string(),
                    serde_json::Value::String(format!(
                        "risk score {:.2} exceeds threshold",
                        assessment.score
                    )),
                );
            }
        }

        Ok(MiddlewareResult::Continue)
    }

    fn chain_type(&self) -> ChainType {
        ChainType::Tool
    }

    fn priority(&self) -> u32 {
        10 // Run very early in the tool chain
    }

    fn name(&self) -> &'static str {
        "ToolGuardMiddleware"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use y_core::hook::MiddlewareContext;

    fn make_tool_context(tool_name: &str, is_dangerous: bool) -> MiddlewareContext {
        MiddlewareContext::new(
            ChainType::Tool,
            serde_json::json!({
                "tool_name": tool_name,
                "is_dangerous": is_dangerous,
                "category": "shell",
            }),
        )
    }

    #[tokio::test]
    async fn test_tool_guard_allows_safe_tool() {
        let config = GuardrailConfig::default();
        let mw = ToolGuardMiddleware::new(config);
        let mut ctx = make_tool_context("read_file", false);

        let result = mw.execute(&mut ctx).await.unwrap();
        assert!(matches!(result, MiddlewareResult::Continue));
        assert!(!ctx.aborted);
    }

    #[tokio::test]
    async fn test_tool_guard_denies_blocked_tool() {
        let mut config = GuardrailConfig::default();
        config
            .tool_permissions
            .insert("rm_rf".to_string(), PermissionAction::Deny);

        let mw = ToolGuardMiddleware::new(config);
        let mut ctx = make_tool_context("rm_rf", false);

        let result = mw.execute(&mut ctx).await.unwrap();
        assert!(matches!(result, MiddlewareResult::ShortCircuit));
        assert!(ctx.aborted);
        assert!(ctx
            .abort_reason
            .as_ref()
            .unwrap()
            .contains("Permission denied"));
    }

    #[tokio::test]
    async fn test_tool_guard_dangerous_triggers_hitl() {
        let config = GuardrailConfig::default();
        let mw = ToolGuardMiddleware::new(config);
        let mut ctx = make_tool_context("shell_exec", true);

        let result = mw.execute(&mut ctx).await.unwrap();
        assert!(matches!(result, MiddlewareResult::Continue));
        assert!(
            ctx.metadata
                .get("hitl_required")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
            "dangerous tool should require HITL"
        );
    }
}
