//! Agent delegation traits for cross-module invocation.
//!
//! Design reference: `multi-agent-design.md` §Cross-Module Invocation Protocol,
//! `AGENT_AUTONOMY.md` §2.4
//!
//! This module defines the `AgentDelegator` trait that enables any module
//! to request agent delegation without depending on the `y-agent` crate directly. At runtime,
//! `y-agent`'s `AgentPool` implements this trait and is injected into
//! modules that need it (e.g., `y-context`, `y-session`, `y-skills`).

use std::fmt;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::trust::TrustTier;

// ---------------------------------------------------------------------------
// Context strategy hint
// ---------------------------------------------------------------------------

/// Lightweight hint for context sharing strategy across crate boundaries.
///
/// Mirrors `ContextStrategy` from `y-agent` without creating a dependency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ContextStrategyHint {
    /// Only the delegation prompt is provided.
    #[default]
    None,
    /// LLM-generated summary of relevant conversation context.
    Summary,
    /// Specific messages matching a filter (by role, recency, keyword).
    Filtered,
    /// Complete conversation history up to token limit.
    Full,
}

// ---------------------------------------------------------------------------
// Delegation output
// ---------------------------------------------------------------------------

/// Result of a successful agent delegation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DelegationOutput {
    /// The text output produced by the delegated agent.
    pub text: String,
    /// Approximate tokens consumed during the delegation.
    pub tokens_used: u32,
    /// Input tokens consumed (for diagnostics breakdown).
    pub input_tokens: u64,
    /// Output tokens generated (for diagnostics breakdown).
    pub output_tokens: u64,
    /// Model that was actually used.
    pub model_used: String,
    /// Wall-clock duration of the delegation in milliseconds.
    pub duration_ms: u64,
}

// ---------------------------------------------------------------------------
// Delegation error
// ---------------------------------------------------------------------------

/// Errors that can occur during agent delegation.
#[derive(Debug, thiserror::Error)]
pub enum DelegationError {
    /// The requested agent name was not found in the registry.
    #[error("agent not found: '{name}'")]
    AgentNotFound { name: String },

    /// The delegation failed during execution.
    #[error("delegation failed: {message}")]
    DelegationFailed { message: String },

    /// The delegation timed out.
    #[error("delegation timed out after {duration_ms}ms")]
    Timeout { duration_ms: u64 },

    /// The delegation depth has been exhausted (no further nesting allowed).
    #[error("delegation depth exhausted at depth {depth}")]
    DepthExhausted { depth: u32 },
}

// ---------------------------------------------------------------------------
// AgentDelegator trait
// ---------------------------------------------------------------------------

/// Trait for modules to request agent delegation without depending on `y-agent`.
///
/// Modules pass structured input data — the agent controls its own prompt.
/// The agent's system prompt and reasoning strategy are defined in its `AgentDefinition`;
/// the caller only provides the data to be processed.
///
/// # Example
///
/// ```rust,ignore
/// // In y-context (or any other crate):
/// async fn compact_context(
///     delegator: &dyn AgentDelegator,
///     messages: serde_json::Value,
///     session_id: Option<uuid::Uuid>,
/// ) -> Result<String, DelegationError> {
///     let result = delegator
///         .delegate("compaction-summarizer", messages, ContextStrategyHint::None, session_id)
///         .await?;
///     Ok(result.text)
/// }
/// ```
#[async_trait]
pub trait AgentDelegator: Send + Sync + fmt::Debug {
    /// Delegate a task to a named agent with structured input data.
    ///
    /// `input` is the raw data the agent needs to process (e.g., messages to
    /// summarize, experience records to analyze). The agent's own prompt template
    /// determines how this data is presented to the LLM.
    ///
    /// `session_id` optionally associates the delegation trace with a specific
    /// user session so that the subagent call appears in session-level
    /// diagnostics. Pass `None` for session-independent operations.
    ///
    /// # Errors
    ///
    /// Returns `DelegationError` if the agent is not found, the delegation fails,
    /// times out, or the delegation depth is exhausted.
    async fn delegate(
        &self,
        agent_name: &str,
        input: serde_json::Value,
        context_strategy: ContextStrategyHint,
        session_id: Option<uuid::Uuid>,
    ) -> Result<DelegationOutput, DelegationError>;
}

// ---------------------------------------------------------------------------
// AgentRunner trait
// ---------------------------------------------------------------------------

/// Configuration for a single agent execution.
///
/// Built from an `AgentDefinition` and the caller's input data.
/// Passed to [`AgentRunner::run`] to execute the agent's LLM reasoning.
#[derive(Debug, Clone)]
pub struct AgentRunConfig {
    /// Agent name (for routing and logging).
    pub agent_name: String,
    /// The agent's system prompt (from its TOML definition).
    pub system_prompt: String,
    /// Structured input data from the caller.
    pub input: serde_json::Value,
    /// Preferred models (tried in order).
    pub preferred_models: Vec<String>,
    /// Fallback models if preferred are unavailable.
    pub fallback_models: Vec<String>,
    /// Provider routing tags (e.g. `["general"]`, `["title"]`).
    /// Used as `required_tags` in `RouteRequest` for provider selection.
    pub provider_tags: Vec<String>,
    /// Sampling temperature override (from agent definition).
    pub temperature: Option<f64>,
    /// Maximum tokens to generate.
    pub max_tokens: Option<u32>,
    /// Timeout for the entire run in seconds.
    pub timeout_secs: u64,
    /// Tools the agent is allowed to use (from `AgentDefinition`).
    /// Empty = no tool calling (single-turn mode).
    pub allowed_tools: Vec<String>,
    /// Tools explicitly denied (from `AgentDefinition`).
    pub denied_tools: Vec<String>,
    /// Maximum agent loop iterations (tool-call loop limit).
    pub max_iterations: usize,
    /// Trust tier of the agent (for permission bypass decisions).
    ///
    /// When `Some(TrustTier::BuiltIn)`, the runner may auto-allow tools
    /// listed in `allowed_tools` without consulting the global permission
    /// policy.
    pub trust_tier: Option<TrustTier>,
    /// Optional pre-created trace ID from the diagnostics delegator.
    ///
    /// When set, the runner should forward this to the execution engine so
    /// that per-iteration observations are recorded under this trace instead
    /// of creating a new one.
    pub trace_id: Option<uuid::Uuid>,
    /// Whether to prune historical tool call pairs from `working_history`.
    ///
    /// When `true`, old assistant+tool message pairs are removed between
    /// iterations, keeping only the most recent batch.
    pub prune_tool_history: bool,
}

/// Output from a single agent execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRunOutput {
    /// Text output produced by the agent.
    pub text: String,
    /// Tokens consumed during execution (input + output combined).
    pub tokens_used: u32,
    /// Input tokens consumed (for diagnostics breakdown).
    pub input_tokens: u64,
    /// Output tokens generated (for diagnostics breakdown).
    pub output_tokens: u64,
    /// Model that was actually used.
    pub model_used: String,
    /// Wall-clock duration in milliseconds.
    pub duration_ms: u64,
}

/// Executes an agent's LLM reasoning given its configuration and input.
///
/// Implementations bridge agent definitions to actual `ProviderPool` calls.
/// Injected into `AgentPool` at startup via dependency injection.
///
/// # Implementations
///
/// - `SingleTurnRunner` (in `y-provider`): `system_prompt` + input → single
///   `ProviderPool::chat_completion()` call. Suitable for system agents
///   (title-generator, compaction-summarizer, etc.).
#[async_trait]
pub trait AgentRunner: Send + Sync {
    /// Run a single-turn agent: `system_prompt` + input → text output.
    ///
    /// The runner builds the appropriate `ChatRequest` from the config,
    /// routes it to an available provider, and returns the result.
    async fn run(&self, config: AgentRunConfig) -> Result<AgentRunOutput, DelegationError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// T-MA-P2-01: `AgentDelegator` trait is object-safe (can be used as `Arc<dyn AgentDelegator>`).
    #[test]
    fn test_agent_delegator_trait_object_safe() {
        // This test verifies the trait is object-safe by compiling.
        fn _assert_object_safe(_delegator: std::sync::Arc<dyn AgentDelegator>) {}
    }

    /// T-MA-P2-02: `ContextStrategyHint` serde roundtrip.
    #[test]
    fn test_context_strategy_hint_serde() {
        let strategies = [
            (ContextStrategyHint::None, "\"none\""),
            (ContextStrategyHint::Summary, "\"summary\""),
            (ContextStrategyHint::Filtered, "\"filtered\""),
            (ContextStrategyHint::Full, "\"full\""),
        ];

        for (hint, expected_json) in strategies {
            let json = serde_json::to_string(&hint).unwrap();
            assert_eq!(json, expected_json);
            let parsed: ContextStrategyHint = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, hint);
        }
    }

    /// T-MA-P2-03: `DelegationOutput` fields are accessible and constructible.
    #[test]
    fn test_delegation_output_fields() {
        let output = DelegationOutput {
            text: "summarized content".to_string(),
            tokens_used: 150,
            input_tokens: 100,
            output_tokens: 50,
            model_used: "gpt-4o".to_string(),
            duration_ms: 1200,
        };

        assert_eq!(output.text, "summarized content");
        assert_eq!(output.tokens_used, 150);
        assert_eq!(output.input_tokens, 100);
        assert_eq!(output.output_tokens, 50);
        assert_eq!(output.model_used, "gpt-4o");
        assert_eq!(output.duration_ms, 1200);
    }

    /// T-MA-P2-04: `DelegationError` variants have correct Display output.
    #[test]
    fn test_delegation_error_variants() {
        let errors: Vec<(DelegationError, &str)> = vec![
            (
                DelegationError::AgentNotFound {
                    name: "test-agent".to_string(),
                },
                "agent not found: 'test-agent'",
            ),
            (
                DelegationError::DelegationFailed {
                    message: "LLM call failed".to_string(),
                },
                "delegation failed: LLM call failed",
            ),
            (
                DelegationError::Timeout { duration_ms: 5000 },
                "delegation timed out after 5000ms",
            ),
            (
                DelegationError::DepthExhausted { depth: 0 },
                "delegation depth exhausted at depth 0",
            ),
        ];

        for (error, expected_msg) in errors {
            assert_eq!(error.to_string(), expected_msg);
        }
    }

    /// T-MA-P2-03b: `DelegationOutput` serde roundtrip.
    #[test]
    fn test_delegation_output_serde() {
        let output = DelegationOutput {
            text: "test output".to_string(),
            tokens_used: 42,
            input_tokens: 30,
            output_tokens: 12,
            model_used: "gpt-4o".to_string(),
            duration_ms: 500,
        };
        let json = serde_json::to_string(&output).unwrap();
        let parsed: DelegationOutput = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.text, output.text);
        assert_eq!(parsed.tokens_used, output.tokens_used);
        assert_eq!(parsed.input_tokens, output.input_tokens);
        assert_eq!(parsed.output_tokens, output.output_tokens);
        assert_eq!(parsed.model_used, output.model_used);
        assert_eq!(parsed.duration_ms, output.duration_ms);
    }

    /// `AgentRunner` trait is object-safe.
    #[test]
    fn test_agent_runner_trait_object_safe() {
        fn _assert_object_safe(_runner: std::sync::Arc<dyn AgentRunner>) {}
    }

    /// `AgentRunConfig` is constructible with all fields.
    #[test]
    fn test_agent_run_config_fields() {
        let config = AgentRunConfig {
            agent_name: "title-generator".to_string(),
            system_prompt: "Generate a title.".to_string(),
            input: serde_json::json!({"messages": ["hello"]}),
            preferred_models: vec!["gpt-4o-mini".to_string()],
            fallback_models: vec!["gpt-4o".to_string()],
            provider_tags: vec!["title".to_string()],
            temperature: Some(0.3),
            max_tokens: Some(30),
            timeout_secs: 30,
            allowed_tools: vec![],
            denied_tools: vec![],
            max_iterations: 1,
            trust_tier: None,
            trace_id: None,
            prune_tool_history: false,
        };
        assert_eq!(config.agent_name, "title-generator");
        assert_eq!(config.preferred_models.len(), 1);
    }

    /// `AgentRunOutput` serde roundtrip.
    #[test]
    fn test_agent_run_output_serde() {
        let output = AgentRunOutput {
            text: "My Title".to_string(),
            tokens_used: 15,
            input_tokens: 10,
            output_tokens: 5,
            model_used: "gpt-4o-mini".to_string(),
            duration_ms: 200,
        };
        let json = serde_json::to_string(&output).unwrap();
        let parsed: AgentRunOutput = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.text, output.text);
        assert_eq!(parsed.model_used, output.model_used);
    }
}
