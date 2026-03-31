//! Task delegation orchestrator -- performs actual agent delegation when the
//! `task` tool is called by the LLM.
//!
//! Intercepts `task` tool calls in `AgentService::execute_tool_call()` and
//! routes them through the `AgentDelegator` (same pattern as `ToolSearch`
//! / `ToolSearchOrchestrator`).

use y_core::agent::{ContextStrategyHint, DelegationError};
use y_core::tool::{ToolError, ToolOutput};

/// Orchestrates task delegation: parses the LLM's `task` tool arguments
/// and delegates to the appropriate agent via `AgentDelegator`.
pub struct TaskDelegationOrchestrator;

impl TaskDelegationOrchestrator {
    /// Handle a `task` tool call by delegating to the named agent.
    ///
    /// Parses `arguments` for `agent_name` (required), `prompt` (required),
    /// and optional `mode` / `context_strategy`. Delegates via the provided
    /// `AgentDelegator` and maps the result to a `ToolOutput`.
    pub async fn handle(
        arguments: &serde_json::Value,
        delegator: &dyn y_core::agent::AgentDelegator,
        session_id: Option<uuid::Uuid>,
    ) -> Result<ToolOutput, ToolError> {
        let agent_name = arguments
            .get("agent_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ValidationError {
                message: "'agent_name' is required".into(),
            })?;

        let prompt = arguments
            .get("prompt")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ValidationError {
                message: "'prompt' is required".into(),
            })?;

        let mode = arguments.get("mode").and_then(|v| v.as_str());

        let context_strategy = arguments
            .get("context_strategy")
            .and_then(|v| v.as_str())
            .map(|s| {
                serde_json::from_value::<ContextStrategyHint>(serde_json::Value::String(
                    s.to_string(),
                ))
            })
            .transpose()
            .map_err(|e| ToolError::ValidationError {
                message: format!("invalid context_strategy: {e}"),
            })?
            .unwrap_or_default();

        // Build structured input for the agent.
        let input = serde_json::json!({
            "task": prompt,
            "mode": mode,
        });
        let input_snapshot = input.clone();

        let result = delegator
            .delegate(agent_name, input, context_strategy, session_id)
            .await
            .map_err(|e| match e {
                DelegationError::AgentNotFound { name } => ToolError::NotFound { name },
                DelegationError::Timeout { duration_ms } => ToolError::Timeout {
                    timeout_secs: duration_ms / 1000,
                },
                DelegationError::DelegationFailed { message } => ToolError::RuntimeError {
                    name: agent_name.to_string(),
                    message,
                },
                DelegationError::DepthExhausted { depth } => ToolError::RuntimeError {
                    name: agent_name.to_string(),
                    message: format!("delegation depth exhausted at depth {depth}"),
                },
            })?;

        Ok(ToolOutput {
            success: true,
            content: serde_json::json!({
                "agent_name": agent_name,
                "input": input_snapshot,
                "output": result.text,
                "model_used": result.model_used,
                "tokens_used": result.tokens_used,
                "duration_ms": result.duration_ms,
            }),
            warnings: vec![],
            metadata: serde_json::json!({
                "action": "delegate",
                "input_tokens": result.input_tokens,
                "output_tokens": result.output_tokens,
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use y_core::agent::{AgentDelegator, ContextStrategyHint, DelegationError, DelegationOutput};

    /// Mock delegator that returns a fixed successful response.
    #[derive(Debug)]
    struct MockDelegator {
        expected_agent: String,
    }

    #[async_trait]
    impl AgentDelegator for MockDelegator {
        async fn delegate(
            &self,
            agent_name: &str,
            _input: serde_json::Value,
            _context_strategy: ContextStrategyHint,
            _session_id: Option<uuid::Uuid>,
        ) -> Result<DelegationOutput, DelegationError> {
            if agent_name == self.expected_agent {
                Ok(DelegationOutput {
                    text: format!("Agent {agent_name} completed the task."),
                    tokens_used: 100,
                    input_tokens: 60,
                    output_tokens: 40,
                    model_used: "test-model".to_string(),
                    duration_ms: 500,
                })
            } else {
                Err(DelegationError::AgentNotFound {
                    name: agent_name.to_string(),
                })
            }
        }
    }

    /// Mock delegator that always fails with DelegationFailed.
    #[derive(Debug)]
    struct FailingDelegator;

    #[async_trait]
    impl AgentDelegator for FailingDelegator {
        async fn delegate(
            &self,
            _agent_name: &str,
            _input: serde_json::Value,
            _context_strategy: ContextStrategyHint,
            _session_id: Option<uuid::Uuid>,
        ) -> Result<DelegationOutput, DelegationError> {
            Err(DelegationError::DelegationFailed {
                message: "LLM call failed".to_string(),
            })
        }
    }

    /// Mock delegator that always times out.
    #[derive(Debug)]
    struct TimeoutDelegator;

    #[async_trait]
    impl AgentDelegator for TimeoutDelegator {
        async fn delegate(
            &self,
            _agent_name: &str,
            _input: serde_json::Value,
            _context_strategy: ContextStrategyHint,
            _session_id: Option<uuid::Uuid>,
        ) -> Result<DelegationOutput, DelegationError> {
            Err(DelegationError::Timeout { duration_ms: 30000 })
        }
    }

    /// Mock delegator that returns DepthExhausted.
    #[derive(Debug)]
    struct DepthExhaustedDelegator;

    #[async_trait]
    impl AgentDelegator for DepthExhaustedDelegator {
        async fn delegate(
            &self,
            _agent_name: &str,
            _input: serde_json::Value,
            _context_strategy: ContextStrategyHint,
            _session_id: Option<uuid::Uuid>,
        ) -> Result<DelegationOutput, DelegationError> {
            Err(DelegationError::DepthExhausted { depth: 3 })
        }
    }

    #[tokio::test]
    async fn test_handle_valid_delegation() {
        let delegator = MockDelegator {
            expected_agent: "agent-architect".to_string(),
        };
        let args = serde_json::json!({
            "agent_name": "agent-architect",
            "prompt": "Design a disk info agent"
        });

        let result = TaskDelegationOrchestrator::handle(&args, &delegator, None)
            .await
            .unwrap();

        assert!(result.success);
        assert_eq!(result.content["agent_name"], "agent-architect");
        assert!(result.content["input"]["task"]
            .as_str()
            .unwrap()
            .contains("Design a disk info agent"));
        assert!(result.content["output"]
            .as_str()
            .unwrap()
            .contains("completed the task"));
        assert_eq!(result.content["model_used"], "test-model");
        assert_eq!(result.content["tokens_used"], 100);
        assert_eq!(result.content["duration_ms"], 500);
    }

    #[tokio::test]
    async fn test_handle_with_optional_params() {
        let delegator = MockDelegator {
            expected_agent: "tool-engineer".to_string(),
        };
        let args = serde_json::json!({
            "agent_name": "tool-engineer",
            "prompt": "Build a search tool",
            "mode": "build",
            "context_strategy": "summary"
        });

        let result = TaskDelegationOrchestrator::handle(&args, &delegator, None)
            .await
            .unwrap();

        assert!(result.success);
        assert_eq!(result.content["agent_name"], "tool-engineer");
    }

    #[tokio::test]
    async fn test_handle_missing_agent_name() {
        let delegator = MockDelegator {
            expected_agent: "any".to_string(),
        };
        let args = serde_json::json!({"prompt": "do something"});

        let result = TaskDelegationOrchestrator::handle(&args, &delegator, None).await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ToolError::ValidationError { .. }
        ));
    }

    #[tokio::test]
    async fn test_handle_missing_prompt() {
        let delegator = MockDelegator {
            expected_agent: "any".to_string(),
        };
        let args = serde_json::json!({"agent_name": "agent-architect"});

        let result = TaskDelegationOrchestrator::handle(&args, &delegator, None).await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ToolError::ValidationError { .. }
        ));
    }

    #[tokio::test]
    async fn test_handle_agent_not_found() {
        let delegator = MockDelegator {
            expected_agent: "agent-architect".to_string(),
        };
        let args = serde_json::json!({
            "agent_name": "nonexistent-agent",
            "prompt": "do something"
        });

        let result = TaskDelegationOrchestrator::handle(&args, &delegator, None).await;

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ToolError::NotFound { .. }));
    }

    #[tokio::test]
    async fn test_handle_delegation_failed() {
        let delegator = FailingDelegator;
        let args = serde_json::json!({
            "agent_name": "agent-architect",
            "prompt": "do something"
        });

        let result = TaskDelegationOrchestrator::handle(&args, &delegator, None).await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ToolError::RuntimeError { .. }
        ));
    }

    #[tokio::test]
    async fn test_handle_timeout() {
        let delegator = TimeoutDelegator;
        let args = serde_json::json!({
            "agent_name": "agent-architect",
            "prompt": "do something"
        });

        let result = TaskDelegationOrchestrator::handle(&args, &delegator, None).await;

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ToolError::Timeout { .. }));
    }

    #[tokio::test]
    async fn test_handle_depth_exhausted() {
        let delegator = DepthExhaustedDelegator;
        let args = serde_json::json!({
            "agent_name": "agent-architect",
            "prompt": "do something"
        });

        let result = TaskDelegationOrchestrator::handle(&args, &delegator, None).await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ToolError::RuntimeError { .. }
        ));
    }

    #[tokio::test]
    async fn test_handle_invalid_context_strategy() {
        let delegator = MockDelegator {
            expected_agent: "agent-architect".to_string(),
        };
        let args = serde_json::json!({
            "agent_name": "agent-architect",
            "prompt": "do something",
            "context_strategy": "invalid_value"
        });

        let result = TaskDelegationOrchestrator::handle(&args, &delegator, None).await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ToolError::ValidationError { .. }
        ));
    }
}
