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

pub mod capability_gap;
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
pub use config::{GuardrailConfig, PlanReviewConfig, PlanReviewMode};
pub use error::{GuardrailError, LoopPattern};
pub use hitl::{HitlHandler, HitlProtocol, HitlRequest, HitlResponse};
pub use loop_guard::{ActionRecord, LoopDetection, LoopGuard};
pub use middleware::llm_guard::LlmGuardMiddleware;
pub use middleware::loop_detector::LoopDetectorMiddleware;
pub use middleware::tool_guard::ToolGuardMiddleware;
pub use mode_manager::PermissionModeManager;
pub use permission::{PermissionAction, PermissionDecision, PermissionModel};
pub use permission_pipeline::evaluate_pipeline;
pub use risk::{RiskAssessment, RiskFactors, RiskScorer};
pub use rule_store::PermissionRuleStore;
pub use taint::{TaintCheckResult, TaintTag, TaintTracker};

/// Convenience builder that creates all guardrail middleware from a single config.
///
/// The inner config is wrapped in `RwLock` so it can be hot-reloaded at
/// runtime without restarting the application.
pub struct GuardrailManager {
    config: std::sync::RwLock<GuardrailConfig>,
}

impl std::fmt::Debug for GuardrailManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let cfg = self
            .config
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        f.debug_struct("GuardrailManager")
            .field("config", &*cfg)
            .finish()
    }
}

impl GuardrailManager {
    /// Create a new guardrail manager.
    pub fn new(config: GuardrailConfig) -> Self {
        Self {
            config: std::sync::RwLock::new(config),
        }
    }

    /// Get a snapshot of the current guardrail configuration.
    pub fn config(&self) -> GuardrailConfig {
        self.config
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
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
