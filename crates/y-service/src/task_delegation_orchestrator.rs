//! Task delegation orchestrator -- performs actual agent delegation when the
//! `task` tool is called by the LLM.
//!
//! Intercepts `task` tool calls in `AgentService::execute_tool_call()` and
//! routes them through the `AgentDelegator` (same pattern as `ToolSearch`
//! / `ToolSearchOrchestrator`).

use tokio::sync::Mutex;
use y_agent::AgentRegistry;
use y_core::agent::{ContextStrategyHint, DelegationError};
use y_core::tool::{ToolError, ToolOutput};

/// Agent that receives task delegations whose requested agent is not
/// registered. Seeded as a user-tier built-in, so it is always resolvable.
const FALLBACK_AGENT: &str = "general-purpose";

// ---------------------------------------------------------------------------
// Schema-validated yield helpers
// ---------------------------------------------------------------------------

/// Result of validating an agent's text output against a JSON Schema.
struct ValidatedJson {
    /// The extracted JSON value (validated or fallback).
    json: serde_json::Value,
    /// Whether the JSON passed schema validation.
    is_valid: bool,
    /// Optional warning message when validation failed.
    warning: Option<String>,
}

/// Parse the agent's text output as JSON and validate it against `schema`.
///
/// Tries three strategies in order:
/// 1. Direct `serde_json::from_str` — model returned pure JSON.
/// 2. Extract JSON from a markdown code block (```json ... ```).
/// 3. Fallback: wrap the raw text in a JSON object with a `_warning` field.
///
/// Schema validation uses the `jsonschema` crate. A schema that fails to
/// compile is treated as "no validation" (the JSON is returned as-is).
fn validate_and_extract_json(text: &str, schema: &serde_json::Value) -> ValidatedJson {
    let compiled = jsonschema::validator_for(schema).ok();

    // Strategy 1: direct parse.
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(text) {
        let is_valid = compiled.as_ref().is_none_or(|v| v.is_valid(&value));
        if is_valid {
            return ValidatedJson {
                json: value,
                is_valid: true,
                warning: None,
            };
        }
    }

    // Strategy 2: extract from markdown code block.
    if let Some(json_str) = extract_json_from_codeblock(text) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&json_str) {
            let is_valid = compiled.as_ref().is_none_or(|v| v.is_valid(&value));
            if is_valid {
                return ValidatedJson {
                    json: value,
                    is_valid: true,
                    warning: None,
                };
            }
        }
    }

    // Strategy 3: fallback — wrap raw text with a warning.
    ValidatedJson {
        json: serde_json::json!({
            "_warning": "agent response did not match the requested schema",
            "raw_text": text,
        }),
        is_valid: false,
        warning: Some(
            "agent response did not match the requested result_schema; \
             raw text wrapped in fallback JSON"
                .into(),
        ),
    }
}

/// Extract the first JSON code block from a markdown string.
///
/// Handles ` ```json ` and bare ` ``` ` fenced blocks.
fn extract_json_from_codeblock(text: &str) -> Option<String> {
    let json_fence = "```json";
    let generic_fence = "```";

    // Try ```json fence first.
    if let Some(start) = text.find(json_fence) {
        let after_fence = start + json_fence.len();
        if let Some(end) = text[after_fence..].find(generic_fence) {
            return Some(text[after_fence..after_fence + end].trim().to_string());
        }
    }

    // Try generic ``` fence (only if the content looks like JSON).
    if let Some(start) = text.find(generic_fence) {
        let after_fence = start + generic_fence.len();
        if let Some(end) = text[after_fence..].find(generic_fence) {
            let inner = text[after_fence..after_fence + end].trim();
            if inner.starts_with('{') || inner.starts_with('[') {
                return Some(inner.to_string());
            }
        }
    }

    None
}

/// Orchestrates task delegation: parses the LLM's `task` tool arguments
/// and delegates to the appropriate agent via `AgentDelegator`.
pub struct TaskDelegationOrchestrator;

impl TaskDelegationOrchestrator {
    /// Handle a `task` tool call by delegating to the named agent.
    ///
    /// Parses `arguments` for `agent_name` (required), `prompt` (required),
    /// and optional `mode` / `context_strategy`. When the requested agent is
    /// not present in `agent_registry`, the delegation is routed to the
    /// `general-purpose` agent instead of failing the tool call. Delegates via
    /// the provided `AgentDelegator` and maps the result to a `ToolOutput`.
    pub async fn handle(
        arguments: &serde_json::Value,
        delegator: &dyn y_core::agent::AgentDelegator,
        agent_registry: &Mutex<AgentRegistry>,
        session_id: Option<uuid::Uuid>,
    ) -> Result<ToolOutput, ToolError> {
        let requested_agent = arguments
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

        // Resolve the effective agent. When the LLM requests an agent that is
        // not registered, route the delegation to the general-purpose agent
        // rather than failing the tool call. The registry is shared with the
        // delegator's pool, so a hit here guarantees the pool resolves the same
        // definition -- and the reroute produces a single clean subagent trace
        // instead of a spurious failed one.
        let effective_agent = {
            let registry = agent_registry.lock().await;
            if registry.get(requested_agent).is_some() {
                requested_agent.to_string()
            } else {
                FALLBACK_AGENT.to_string()
            }
        };

        let mut warnings = Vec::new();
        if effective_agent != requested_agent {
            tracing::warn!(
                requested_agent,
                fallback_agent = FALLBACK_AGENT,
                "requested agent is not registered; routing task to the general-purpose agent",
            );
            warnings.push(format!(
                "requested agent '{requested_agent}' is not registered; \
                 routed to '{FALLBACK_AGENT}'"
            ));
        }

        // Extract optional result_schema for structured output validation.
        let result_schema = arguments.get("result_schema").cloned();
        let workspace_isolation = arguments.get("workspace_isolation").cloned();
        let workspace_snapshot_id = arguments.get("workspace_snapshot_id").cloned();

        // Build structured input for the agent. When a result_schema is
        // provided, it is passed as `_result_schema` so the AgentPool can
        // set `response_format` on the agent's run config, enabling
        // API-level structured output enforcement for providers that
        // support it (OpenAI, Anthropic).
        let mut input = serde_json::json!({
            "task": prompt,
            "mode": mode,
        });
        if let Some(schema) = result_schema.as_ref() {
            input["_result_schema"] = schema.clone();
        }
        if let Some(isolation) = workspace_isolation {
            input["_workspace_isolation"] = isolation;
        }
        if let Some(snapshot_id) = workspace_snapshot_id {
            input["_workspace_snapshot_id"] = snapshot_id;
        }

        let result = delegator
            .delegate(&effective_agent, input, context_strategy, session_id)
            .await
            .map_err(|e| match &e {
                DelegationError::AgentNotFound { name } => {
                    ToolError::NotFound { name: name.clone() }
                }
                DelegationError::Timeout { duration_ms } => ToolError::Timeout {
                    timeout_secs: duration_ms / 1000,
                },
                DelegationError::DelegationFailed { message } => ToolError::RuntimeError {
                    name: effective_agent.clone(),
                    message: message.clone(),
                },
                DelegationError::DepthExhausted { depth } => ToolError::RuntimeError {
                    name: effective_agent.clone(),
                    message: format!("delegation depth exhausted at depth {depth}"),
                },
            })?;

        let workspace_isolation = result.workspace_isolation.clone();
        let workspace_isolation_summary = workspace_isolation.as_ref().map(|metadata| {
            let mut summary = serde_json::to_value(metadata).unwrap_or(serde_json::Value::Null);
            if let Some(object) = summary.as_object_mut() {
                object.remove("patch");
            }
            summary
        });

        // When a result_schema was requested, validate the agent's output
        // as JSON against the schema. This lets the parent agent consume
        // typed JSON directly instead of parsing prose.
        let mut content = if let Some(ref schema) = result_schema {
            let validated = validate_and_extract_json(&result.text, schema);
            if let Some(warning) = validated.warning {
                warnings.push(warning);
            }
            serde_json::json!({
                "agent_name": effective_agent,
                "output": validated.json,
                "schema_validated": validated.is_valid,
            })
        } else {
            // The content returned to the LLM is the delegated agent's
            // output only. Diagnostics fields (model, tokens, duration) are
            // recorded separately via the trace pipeline in
            // `DiagnosticsAgentDelegator`; surfacing them here would pollute
            // the conversation context with non-semantic noise.
            serde_json::json!({
                "agent_name": effective_agent,
                "output": result.text,
            })
        };
        if let Some(metadata) = workspace_isolation {
            let Some(content_object) = content.as_object_mut() else {
                return Err(ToolError::RuntimeError {
                    name: effective_agent,
                    message: "delegation result content was not an object".to_string(),
                });
            };
            content_object.insert(
                "workspace_isolation".to_string(),
                serde_json::to_value(metadata).unwrap_or(serde_json::Value::Null),
            );
        }

        Ok(ToolOutput {
            success: true,
            content,
            warnings,
            metadata: serde_json::json!({
                "action": "delegate",
                "model_used": result.model_used,
                "tokens_used": result.tokens_used,
                "input_tokens": result.input_tokens,
                "output_tokens": result.output_tokens,
                "duration_ms": result.duration_ms,
                "workspace_isolation": workspace_isolation_summary,
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use y_core::agent::{
        AgentDelegator, ContextStrategyHint, DelegationError, DelegationOutput,
        WorkspaceCleanupStatus, WorkspaceConflictStatus, WorkspaceIsolationMetadata,
        WorkspaceIsolationMode, WorkspaceIsolationPreference,
    };

    /// Registry seeded with the built-in agents (includes `agent-architect`,
    /// `tool-engineer`, and the `general-purpose` fallback).
    fn test_registry() -> Mutex<AgentRegistry> {
        Mutex::new(AgentRegistry::new())
    }

    /// Registry with no agents at all -- not even the fallback is resolvable.
    fn empty_registry() -> Mutex<AgentRegistry> {
        Mutex::new(AgentRegistry::empty())
    }

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
                    workspace_isolation: None,
                })
            } else {
                Err(DelegationError::AgentNotFound {
                    name: agent_name.to_string(),
                })
            }
        }
    }

    /// Mock delegator that always fails with `DelegationFailed`.
    #[derive(Debug)]
    struct FailingDelegator;

    #[derive(Debug)]
    struct IsolationMockDelegator;

    #[async_trait]
    impl AgentDelegator for IsolationMockDelegator {
        async fn delegate(
            &self,
            _agent_name: &str,
            _input: serde_json::Value,
            _context_strategy: ContextStrategyHint,
            _session_id: Option<uuid::Uuid>,
        ) -> Result<DelegationOutput, DelegationError> {
            Ok(DelegationOutput {
                text: "isolated result".to_string(),
                tokens_used: 10,
                input_tokens: 6,
                output_tokens: 4,
                model_used: "test-model".to_string(),
                duration_ms: 25,
                workspace_isolation: Some(WorkspaceIsolationMetadata {
                    preference: WorkspaceIsolationPreference::Auto,
                    mode: WorkspaceIsolationMode::Worktree,
                    worktree_id: Some("delegation-test".to_string()),
                    snapshot_id: Some("snapshot-test".to_string()),
                    workspace_path: Some("/tmp/worktree".to_string()),
                    base_revision: Some("abc123".to_string()),
                    changed_files: vec!["result.txt".to_string()],
                    patch: Some("diff --git a/result.txt b/result.txt".to_string()),
                    evidence_error: None,
                    cleanup_status: WorkspaceCleanupStatus::Cleaned,
                    cleanup_error: None,
                    conflict_status: WorkspaceConflictStatus::NotChecked,
                }),
            })
        }
    }

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

    /// Mock delegator that returns `DepthExhausted`.
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

        let registry = test_registry();
        let result = TaskDelegationOrchestrator::handle(&args, &delegator, &registry, None)
            .await
            .unwrap();

        assert!(result.success);
        assert_eq!(result.content["agent_name"], "agent-architect");
        assert!(result.content["output"]
            .as_str()
            .unwrap()
            .contains("completed the task"));
        // The LLM-facing content must not carry the echoed input or any
        // diagnostics telemetry -- those would only pollute the context.
        assert!(result.content.get("input").is_none());
        assert!(result.content.get("model_used").is_none());
        assert!(result.content.get("tokens_used").is_none());
        assert!(result.content.get("duration_ms").is_none());
        // Diagnostics telemetry lives in metadata (presentation/diagnostics
        // path only; never injected into the conversation).
        assert_eq!(result.metadata["model_used"], "test-model");
        assert_eq!(result.metadata["tokens_used"], 100);
        assert_eq!(result.metadata["input_tokens"], 60);
        assert_eq!(result.metadata["output_tokens"], 40);
        assert_eq!(result.metadata["duration_ms"], 500);
    }

    #[tokio::test]
    async fn delegated_workspace_evidence_is_visible_to_parent_and_diagnostics() {
        let args = serde_json::json!({
            "agent_name": "agent-architect",
            "prompt": "write a result"
        });

        let output = TaskDelegationOrchestrator::handle(
            &args,
            &IsolationMockDelegator,
            &test_registry(),
            None,
        )
        .await
        .expect("delegation should succeed");

        assert_eq!(
            output.content["workspace_isolation"]["changed_files"],
            serde_json::json!(["result.txt"])
        );
        assert!(output.content["workspace_isolation"]["patch"]
            .as_str()
            .is_some_and(|patch| patch.contains("result.txt")));
        assert_eq!(
            output.metadata["workspace_isolation"]["cleanup_status"],
            "cleaned"
        );
        assert!(output.metadata["workspace_isolation"]
            .get("patch")
            .is_none());
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

        let registry = test_registry();
        let result = TaskDelegationOrchestrator::handle(&args, &delegator, &registry, None)
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

        let registry = test_registry();
        let result = TaskDelegationOrchestrator::handle(&args, &delegator, &registry, None).await;

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

        let registry = test_registry();
        let result = TaskDelegationOrchestrator::handle(&args, &delegator, &registry, None).await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ToolError::ValidationError { .. }
        ));
    }

    #[tokio::test]
    async fn test_handle_unknown_agent_falls_back_to_general_purpose() {
        // The delegator only accepts the fallback agent, proving the
        // orchestrator rerouted the unregistered request to general-purpose.
        let delegator = MockDelegator {
            expected_agent: FALLBACK_AGENT.to_string(),
        };
        let args = serde_json::json!({
            "agent_name": "nonexistent-agent",
            "prompt": "do something"
        });

        // Built-in registry: "nonexistent-agent" is absent, general-purpose present.
        let registry = test_registry();
        let result = TaskDelegationOrchestrator::handle(&args, &delegator, &registry, None)
            .await
            .unwrap();

        assert!(result.success);
        assert_eq!(result.content["agent_name"], FALLBACK_AGENT);
        // The reroute is surfaced to the LLM as a warning so it can adjust.
        assert!(
            result
                .warnings
                .iter()
                .any(|w| w.contains("nonexistent-agent") && w.contains(FALLBACK_AGENT)),
            "expected a warning naming the requested and fallback agents, got {:?}",
            result.warnings
        );
    }

    #[tokio::test]
    async fn test_handle_registered_agent_no_fallback_warning() {
        let delegator = MockDelegator {
            expected_agent: "agent-architect".to_string(),
        };
        let args = serde_json::json!({
            "agent_name": "agent-architect",
            "prompt": "do something"
        });

        let registry = test_registry();
        let result = TaskDelegationOrchestrator::handle(&args, &delegator, &registry, None)
            .await
            .unwrap();

        assert!(result.success);
        assert_eq!(result.content["agent_name"], "agent-architect");
        assert!(result.warnings.is_empty());
    }

    #[tokio::test]
    async fn test_handle_agent_not_found_when_fallback_unresolvable() {
        // With an empty registry neither the requested agent nor the
        // general-purpose fallback resolves; the delegator then rejects the
        // fallback, so the tool call surfaces NotFound rather than succeeding.
        let delegator = MockDelegator {
            expected_agent: "agent-architect".to_string(),
        };
        let args = serde_json::json!({
            "agent_name": "nonexistent-agent",
            "prompt": "do something"
        });

        let registry = empty_registry();
        let result = TaskDelegationOrchestrator::handle(&args, &delegator, &registry, None).await;

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

        let registry = test_registry();
        let result = TaskDelegationOrchestrator::handle(&args, &delegator, &registry, None).await;

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

        let registry = test_registry();
        let result = TaskDelegationOrchestrator::handle(&args, &delegator, &registry, None).await;

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

        let registry = test_registry();
        let result = TaskDelegationOrchestrator::handle(&args, &delegator, &registry, None).await;

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

        let registry = test_registry();
        let result = TaskDelegationOrchestrator::handle(&args, &delegator, &registry, None).await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ToolError::ValidationError { .. }
        ));
    }

    // --- Schema-validated yield tests ---

    #[test]
    fn test_validate_and_extract_json_direct_parse() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "count": { "type": "integer" }
            },
            "required": ["name"]
        });
        let text = r#"{"name": "test", "count": 42}"#;
        let result = validate_and_extract_json(text, &schema);
        assert!(result.is_valid);
        assert!(result.warning.is_none());
        assert_eq!(result.json["name"], "test");
        assert_eq!(result.json["count"], 42);
    }

    #[test]
    fn test_validate_and_extract_json_codeblock() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "result": { "type": "string" }
            },
            "required": ["result"]
        });
        let text = "Here is the result:\n```json\n{\"result\": \"success\"}\n```\nDone.";
        let result = validate_and_extract_json(text, &schema);
        assert!(result.is_valid);
        assert!(result.warning.is_none());
        assert_eq!(result.json["result"], "success");
    }

    #[test]
    fn test_validate_and_extract_json_generic_codeblock() {
        let schema = serde_json::json!({
            "type": "array",
            "items": { "type": "string" }
        });
        let text = "```\n[\"a\", \"b\", \"c\"]\n```";
        let result = validate_and_extract_json(text, &schema);
        assert!(result.is_valid);
        assert_eq!(result.json.as_array().unwrap().len(), 3);
    }

    #[test]
    fn test_validate_and_extract_json_schema_mismatch() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" }
            },
            "required": ["name"]
        });
        let text = r#"{"wrong_field": 123}"#;
        let result = validate_and_extract_json(text, &schema);
        assert!(!result.is_valid);
        assert!(result.warning.is_some());
        assert!(result.json["_warning"].is_string());
        assert!(result.json["raw_text"].is_string());
    }

    #[test]
    fn test_validate_and_extract_json_not_json() {
        let schema = serde_json::json!({"type": "object"});
        let text = "This is just plain text, no JSON here.";
        let result = validate_and_extract_json(text, &schema);
        assert!(!result.is_valid);
        assert!(result.warning.is_some());
        assert!(result.json["raw_text"]
            .as_str()
            .unwrap()
            .contains("plain text"));
    }

    #[test]
    fn test_validate_and_extract_json_uncompilable_schema() {
        // A malformed schema that fails to compile -- validation is skipped,
        // and any valid JSON is accepted as-is.
        let bad_schema = serde_json::json!("not an object");
        let text = r#"{"any": "value"}"#;
        let result = validate_and_extract_json(text, &bad_schema);
        // Schema compilation fails, so is_valid is true (no validation applied).
        assert!(result.is_valid);
        assert_eq!(result.json["any"], "value");
    }

    #[test]
    fn test_extract_json_from_codeblock_json_fence() {
        let text = "```json\n{\"key\": \"value\"}\n```";
        let extracted = extract_json_from_codeblock(text);
        assert!(extracted.is_some());
        assert_eq!(extracted.unwrap(), r#"{"key": "value"}"#);
    }

    #[test]
    fn test_extract_json_from_codeblock_generic_fence() {
        let text = "```\n[1, 2, 3]\n```";
        let extracted = extract_json_from_codeblock(text);
        assert!(extracted.is_some());
        assert_eq!(extracted.unwrap(), "[1, 2, 3]");
    }

    #[test]
    fn test_extract_json_from_codeblock_no_fence() {
        let text = "just text";
        let extracted = extract_json_from_codeblock(text);
        assert!(extracted.is_none());
    }

    #[test]
    fn test_extract_json_from_codeblock_non_json_content() {
        let text = "```\nplain text\n```";
        let extracted = extract_json_from_codeblock(text);
        assert!(extracted.is_none());
    }

    #[tokio::test]
    async fn test_handle_with_result_schema_valid() {
        let delegator = SchemaMockDelegator {
            response: r#"{"name": "test", "value": 42}"#.to_string(),
        };
        let registry = test_registry();
        let args = serde_json::json!({
            "agent_name": "agent-architect",
            "prompt": "return structured data",
            "result_schema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "value": { "type": "integer" }
                },
                "required": ["name", "value"]
            }
        });

        let result = TaskDelegationOrchestrator::handle(&args, &delegator, &registry, None).await;
        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(output.success);
        assert!(output.content["schema_validated"].as_bool().unwrap());
        assert_eq!(output.content["output"]["name"], "test");
        assert_eq!(output.content["output"]["value"], 42);
    }

    #[tokio::test]
    async fn test_handle_with_result_schema_invalid() {
        let delegator = SchemaMockDelegator {
            response: "This is not JSON at all.".to_string(),
        };
        let registry = test_registry();
        let args = serde_json::json!({
            "agent_name": "agent-architect",
            "prompt": "return structured data",
            "result_schema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string" }
                },
                "required": ["name"]
            }
        });

        let result = TaskDelegationOrchestrator::handle(&args, &delegator, &registry, None).await;
        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(output.success);
        assert!(!output.content["schema_validated"].as_bool().unwrap());
        assert!(output.content["output"]["_warning"].is_string());
        assert!(!output.warnings.is_empty());
    }

    #[tokio::test]
    async fn test_handle_without_result_schema() {
        let delegator = SchemaMockDelegator {
            response: "Plain text response".to_string(),
        };
        let registry = test_registry();
        let args = serde_json::json!({
            "agent_name": "agent-architect",
            "prompt": "do something"
        });

        let result = TaskDelegationOrchestrator::handle(&args, &delegator, &registry, None).await;
        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(output.success);
        // Without schema, output is raw text string, not validated.
        assert_eq!(output.content["output"], "Plain text response");
        assert!(output.content.get("schema_validated").is_none());
    }

    /// Mock delegator that returns a configurable text response.
    #[derive(Debug)]
    struct SchemaMockDelegator {
        response: String,
    }

    #[async_trait]
    impl AgentDelegator for SchemaMockDelegator {
        async fn delegate(
            &self,
            _agent_name: &str,
            _input: serde_json::Value,
            _context_strategy: ContextStrategyHint,
            _session_id: Option<uuid::Uuid>,
        ) -> Result<DelegationOutput, DelegationError> {
            Ok(DelegationOutput {
                text: self.response.clone(),
                tokens_used: 10,
                input_tokens: 5,
                output_tokens: 5,
                model_used: "test-model".to_string(),
                duration_ms: 1,
                workspace_isolation: None,
            })
        }
    }
}
