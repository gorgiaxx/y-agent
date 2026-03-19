//! y-guardrails: Safety validators, `LoopGuard`, taint tracking, risk scoring, HITL.
//!
//! All guardrails are implemented as [`Middleware`](y_core::hook::Middleware) in the
//! y-hooks chains (Tool and LLM), not as a parallel system.
//!
//! # Components
//!
//! - [`permission::PermissionModel`] — unified allow/notify/ask/deny per tool
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
//!
//! # Structural Validation
//!
//! - [`structural::StructuralValidator`] — config-time workflow/tool/budget validation

pub mod capability_gap;
pub mod config;
pub mod error;
pub mod hitl;
pub mod loop_guard;
pub mod middleware;
pub mod permission;
pub mod risk;
pub mod structural;
pub mod taint;

// Re-export primary types.
pub use config::GuardrailConfig;
pub use error::{GuardrailError, LoopPattern};
pub use hitl::{HitlHandler, HitlProtocol, HitlRequest, HitlResponse};
pub use loop_guard::{ActionRecord, LoopDetection, LoopGuard};
pub use middleware::llm_guard::LlmGuardMiddleware;
pub use middleware::loop_detector::LoopDetectorMiddleware;
pub use middleware::tool_guard::ToolGuardMiddleware;
pub use permission::{PermissionAction, PermissionDecision, PermissionModel};
pub use risk::{RiskAssessment, RiskFactors, RiskScorer};
pub use structural::{
    Severity, StructuralValidator, StructuralViolation, TokenBudget, ValidationResult,
};
pub use taint::{TaintCheckResult, TaintTag, TaintTracker};

/// Convenience builder that creates all guardrail middleware from a single config.
#[derive(Debug)]
pub struct GuardrailManager {
    config: GuardrailConfig,
}

impl GuardrailManager {
    /// Create a new guardrail manager.
    pub fn new(config: GuardrailConfig) -> Self {
        Self { config }
    }

    /// Read-only access to the underlying guardrail configuration.
    pub fn config(&self) -> &GuardrailConfig {
        &self.config
    }

    /// Create the `ToolGuardMiddleware`.
    pub fn tool_guard(&self) -> ToolGuardMiddleware {
        ToolGuardMiddleware::new(self.config.clone())
    }

    /// Create the `LoopDetectorMiddleware`.
    pub fn loop_detector(&self) -> LoopDetectorMiddleware {
        LoopDetectorMiddleware::new(self.config.loop_guard.clone())
    }

    /// Create the `LlmGuardMiddleware`.
    pub fn llm_guard(&self) -> LlmGuardMiddleware {
        LlmGuardMiddleware::new()
    }

    /// Create a new `PermissionModel` bound to this config.
    pub fn permission_model(&self) -> PermissionModel {
        PermissionModel::new(self.config.clone())
    }

    /// Create a new `RiskScorer` bound to this config.
    pub fn risk_scorer(&self) -> RiskScorer {
        RiskScorer::new(self.config.risk.clone())
    }

    /// Create a new HITL protocol pair.
    pub fn hitl_protocol(&self) -> (HitlProtocol, HitlHandler) {
        HitlProtocol::new(self.config.hitl.clone())
    }
}
