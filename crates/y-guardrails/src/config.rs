//! Configuration for guardrail policies.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::permission::PermissionAction;

/// Top-level guardrail configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardrailConfig {
    /// Global default permission action for tools without explicit policy.
    #[serde(default = "default_global_permission")]
    pub default_permission: PermissionAction,

    /// Per-tool permission overrides (tool name → action).
    #[serde(default)]
    pub tool_permissions: HashMap<String, PermissionAction>,

    /// Whether dangerous tools (`is_dangerous=true`) auto-escalate to `Ask`.
    #[serde(default = "default_true")]
    pub dangerous_auto_ask: bool,

    /// Loop detection configuration.
    #[serde(default)]
    pub loop_guard: LoopGuardConfig,

    /// Risk scoring configuration.
    #[serde(default)]
    pub risk: RiskConfig,

    /// HITL configuration.
    #[serde(default)]
    pub hitl: HitlConfig,
}

impl Default for GuardrailConfig {
    fn default() -> Self {
        Self {
            default_permission: PermissionAction::Allow,
            tool_permissions: HashMap::new(),
            dangerous_auto_ask: true,
            loop_guard: LoopGuardConfig::default(),
            risk: RiskConfig::default(),
            hitl: HitlConfig::default(),
        }
    }
}

/// Loop detection thresholds and settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoopGuardConfig {
    /// Number of identical actions before Repetition detection fires.
    #[serde(default = "default_repetition_threshold")]
    pub repetition_threshold: usize,

    /// Minimum cycles for Oscillation detection (A→B→A→B = 2 cycles).
    #[serde(default = "default_oscillation_threshold")]
    pub oscillation_threshold: usize,

    /// Steps with no progress metric change for Drift detection.
    #[serde(default = "default_drift_threshold")]
    pub drift_threshold: usize,

    /// Number of identical tool+args calls for `RedundantToolCall` detection.
    #[serde(default = "default_redundant_threshold")]
    pub redundant_threshold: usize,

    /// Whether loop detection is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl Default for LoopGuardConfig {
    fn default() -> Self {
        Self {
            repetition_threshold: 5,
            oscillation_threshold: 3,
            drift_threshold: 10,
            redundant_threshold: 3,
            enabled: true,
        }
    }
}

/// Risk scoring configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskConfig {
    /// Risk score threshold above which actions escalate to `Ask`.
    #[serde(default = "default_risk_threshold")]
    pub escalation_threshold: f32,
}

impl Default for RiskConfig {
    fn default() -> Self {
        Self {
            escalation_threshold: 0.7,
        }
    }
}

/// HITL (Human-in-the-Loop) configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HitlConfig {
    /// Timeout in milliseconds for user response (default: 30 seconds).
    #[serde(default = "default_hitl_timeout_ms")]
    pub timeout_ms: u64,
}

impl Default for HitlConfig {
    fn default() -> Self {
        Self { timeout_ms: 30_000 }
    }
}

// Serde default helpers
fn default_global_permission() -> PermissionAction {
    PermissionAction::Allow
}

const fn default_true() -> bool {
    true
}

const fn default_repetition_threshold() -> usize {
    5
}

const fn default_oscillation_threshold() -> usize {
    3
}

const fn default_drift_threshold() -> usize {
    10
}

const fn default_redundant_threshold() -> usize {
    3
}

const fn default_risk_threshold() -> f32 {
    0.7
}

const fn default_hitl_timeout_ms() -> u64 {
    30_000
}
