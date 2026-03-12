//! Error types for the guardrails module.

use std::fmt;

/// Errors from guardrail operations.
#[derive(Debug, thiserror::Error)]
pub enum GuardrailError {
    /// Tool execution blocked by permission policy.
    #[error("permission denied for tool `{tool}`: policy is `{policy}`")]
    PermissionDenied { tool: String, policy: String },

    /// Loop pattern detected in agent behavior.
    #[error("loop detected: {pattern} ({details})")]
    LoopDetected {
        pattern: LoopPattern,
        details: String,
    },

    /// Tainted data reached a dangerous sink.
    #[error("taint violation: tainted data `{tag}` reached sink `{sink}`")]
    TaintViolation { tag: String, sink: String },

    /// HITL escalation timed out with no user response.
    #[error("HITL timeout after {timeout_ms}ms — defaulting to deny")]
    HitlTimeout { timeout_ms: u64 },

    /// HITL user denied the action.
    #[error("HITL denied by user: {reason}")]
    HitlDenied { reason: String },

    /// Configuration error.
    #[error("guardrail config error: {message}")]
    ConfigError { message: String },

    /// Generic error.
    #[error("{message}")]
    Other { message: String },
}

/// Loop pattern types detected by `LoopGuard`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LoopPattern {
    /// Same action repeated N times.
    Repetition,
    /// A → B → A → B oscillation.
    Oscillation,
    /// No progress metric change over N steps.
    Drift,
    /// Same tool with same arguments called repeatedly.
    RedundantToolCall,
}

impl fmt::Display for LoopPattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Repetition => write!(f, "Repetition"),
            Self::Oscillation => write!(f, "Oscillation"),
            Self::Drift => write!(f, "Drift"),
            Self::RedundantToolCall => write!(f, "RedundantToolCall"),
        }
    }
}
