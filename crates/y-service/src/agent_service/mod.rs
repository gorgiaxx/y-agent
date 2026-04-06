//! Unified Agent Service -- single execution path for all agents.
//!
//! Every agent (interactive chat, sub-agents, system agents) runs through
//! the same [`AgentService::execute`] loop. The agent's capabilities (tools,
//! knowledge, iteration limits) are controlled by its [`AgentExecutionConfig`],
//! not by separate code paths.
//!
//! When `max_iterations=1` and `allowed_tools` is empty, the loop naturally
//! degrades to a single LLM call (equivalent to the old `SingleTurnRunner`).
//!
//! ## Module Structure
//!
//! - `executor` -- main execution loop (`execute_inner`)
//! - `llm` -- LLM call dispatch (streaming + non-streaming)
//! - `tool_dispatch` -- tool execution, permission gating, HITL flow
//! - `tool_handling` -- native/prompt-based tool call handling, dynamic tool sync
//! - `pruning` -- mid-loop context pruning, tool history pruning, thinking strip
//! - `result` -- result building, progress events, diagnostics recording
//! - `subagent` -- `ServiceAgentRunner`, sub-agent prompt construction

mod executor;
mod llm;
mod pruning;
mod result;
mod subagent;
mod tool_dispatch;
mod tool_handling;

use uuid::Uuid;

use y_core::provider::ToolCallingMode;
use y_core::trust::TrustTier;
use y_core::types::{Message, SessionId};

use crate::container::ServiceContainer;

// Re-use progress event types from chat module.
pub use crate::chat::{ToolCallRecord, TurnEvent, TurnEventSender};

// Re-export public types.
pub use self::subagent::ServiceAgentRunner;

// ---------------------------------------------------------------------------
// Execution config & result types
// ---------------------------------------------------------------------------

/// Configuration for a single agent execution.
///
/// Built from an `AgentDefinition` (TOML) plus caller-supplied parameters.
/// This replaces the old `TurnInput` for the internal execution loop.
#[derive(Debug, Clone)]
pub struct AgentExecutionConfig {
    /// Human-readable agent name (for diagnostics/logging).
    pub agent_name: String,
    /// Agent's system prompt (from TOML definition or context pipeline).
    pub system_prompt: String,
    /// Maximum LLM iterations (tool-call loop limit).
    pub max_iterations: usize,
    /// Tool definitions in `OpenAI` function-calling JSON format.
    /// Empty = no tool calling.
    pub tool_definitions: Vec<serde_json::Value>,
    /// Tool calling mode (Native or `PromptBased`).
    pub tool_calling_mode: ToolCallingMode,
    /// Conversation messages (system prompt prepended by caller if needed).
    pub messages: Vec<Message>,
    /// Provider routing preference.
    pub provider_id: Option<String>,
    /// Preferred model identifiers (tried in order via `RouteRequest`).
    pub preferred_models: Vec<String>,
    /// Provider routing tags.
    pub provider_tags: Vec<String>,
    /// Temperature override (None = use provider default).
    pub temperature: Option<f64>,
    /// Max tokens to generate.
    pub max_tokens: Option<u32>,
    /// Thinking/reasoning configuration (`None` = use model defaults).
    pub thinking: Option<y_core::provider::ThinkingConfig>,
    /// Session ID for diagnostics tracing.
    pub session_id: Option<SessionId>,
    /// Session UUID for diagnostics tracing.
    pub session_uuid: Uuid,
    /// Knowledge collection names (empty = skip KB retrieval).
    pub knowledge_collections: Vec<String>,
    /// Whether to use the context pipeline for system prompt assembly.
    /// `true` for the root agent (chat), `false` for sub-agents.
    pub use_context_pipeline: bool,
    /// User query text (for context pipeline + knowledge retrieval).
    pub user_query: String,
    /// Pre-created trace ID from the diagnostics delegator.
    ///
    /// When `Some`, `execute()` reuses this trace for per-iteration
    /// observations instead of creating its own trace. The caller is
    /// responsible for calling `on_trace_start` / `on_trace_end`.
    pub external_trace_id: Option<Uuid>,
    /// Trust tier of the executing agent.
    ///
    /// When `Some(TrustTier::BuiltIn)`, tools listed in `agent_allowed_tools`
    /// are auto-allowed without consulting the global permission policy.
    /// `None` for the root chat agent (uses global policy as-is).
    pub trust_tier: Option<TrustTier>,
    /// Tools declared in the agent definition's `allowed_tools` list.
    ///
    /// Used together with `trust_tier` to auto-allow built-in agent tools.
    /// Empty for the root chat agent.
    pub agent_allowed_tools: Vec<String>,
    /// Whether to prune historical tool call pairs from `working_history`.
    ///
    /// When `true`, old assistant+tool message pairs (all except the most
    /// recent batch) are removed between iterations.
    pub prune_tool_history: bool,
}

/// Result of agent execution.
#[derive(Debug, Clone)]
pub struct AgentExecutionResult {
    /// Final assistant text content.
    pub content: String,
    /// Model that served the final request.
    pub model: String,
    /// Provider ID that served the final request.
    pub provider_id: Option<String>,
    /// Cumulative input tokens across all LLM iterations.
    pub input_tokens: u64,
    /// Cumulative output tokens across all LLM iterations.
    pub output_tokens: u64,
    /// Input tokens from the **last** LLM iteration -- represents the actual
    /// prompt size sent to the model and thus the current context occupancy.
    pub last_input_tokens: u64,
    /// Context window size of the serving provider.
    pub context_window: usize,
    /// Total cost in USD.
    pub cost_usd: f64,
    /// Tool calls executed during this agent run.
    pub tool_calls_executed: Vec<ToolCallRecord>,
    /// Number of LLM iterations (>1 when tool loop occurs).
    pub iterations: usize,
    /// Messages generated during this agent run (assistant + tool messages).
    pub new_messages: Vec<Message>,
    /// Reasoning/thinking content from the final LLM response (if the model
    /// supports chain-of-thought). `None` when the model did not produce
    /// reasoning output.
    pub reasoning_content: Option<String>,
    /// Wall-clock duration of reasoning/thinking in milliseconds.
    /// Measured from the first `StreamReasoningDelta` to the first
    /// `StreamDelta` (content) or end-of-stream, whichever comes first.
    /// `None` when no reasoning was produced or when using non-streaming.
    pub reasoning_duration_ms: Option<u64>,
}

/// Error returned by [`AgentService::execute`].
#[derive(Debug)]
pub enum AgentExecutionError {
    /// LLM request failed.
    LlmError {
        /// Human-readable error message.
        message: String,
        /// Messages accumulated before the failure (assistant + tool messages
        /// from earlier successful iterations). Empty when the error occurs on
        /// the first LLM call.
        partial_messages: Vec<Message>,
    },
    /// Context assembly failed.
    ContextError(String),
    /// Tool-call iteration limit exceeded.
    ToolLoopLimitExceeded {
        /// Maximum allowed iterations.
        max_iterations: usize,
    },
    /// The execution was explicitly cancelled by the caller.
    Cancelled,
}

impl std::fmt::Display for AgentExecutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentExecutionError::LlmError { message, .. } => write!(f, "LLM error: {message}"),
            AgentExecutionError::ContextError(msg) => write!(f, "Context error: {msg}"),
            AgentExecutionError::ToolLoopLimitExceeded { max_iterations } => {
                write!(f, "Tool call loop limit ({max_iterations}) exceeded")
            }
            AgentExecutionError::Cancelled => write!(f, "Cancelled"),
        }
    }
}

impl std::error::Error for AgentExecutionError {}

// ---------------------------------------------------------------------------
// Internal mutable state for the tool-call loop
// ---------------------------------------------------------------------------

/// Carries mutable state across iterations of the agent execution loop.
///
/// Extracted to reduce parameter count on helper methods.
pub(crate) struct ToolExecContext {
    pub(crate) iteration: usize,
    pub(crate) last_gen_id: Option<Uuid>,
    pub(crate) tool_calls_executed: Vec<ToolCallRecord>,
    pub(crate) new_messages: Vec<Message>,
    pub(crate) cumulative_input_tokens: u64,
    pub(crate) cumulative_output_tokens: u64,
    pub(crate) cumulative_cost: f64,
    pub(crate) last_input_tokens: u64,
    pub(crate) trace_id: Option<Uuid>,
    pub(crate) session_id: SessionId,
    pub(crate) working_history: Vec<Message>,
    pub(crate) accumulated_content: String,
    /// Tool definitions dynamically activated via `ToolSearch` during this turn.
    /// Merged with `config.tool_definitions` when building each `ChatRequest`.
    pub(crate) dynamic_tool_defs: Vec<serde_json::Value>,
    /// Pending user-interaction answer channels for `AskUser` tool calls.
    pub(crate) pending_interactions: crate::chat::PendingInteractions,
    /// Pending permission-approval channels for HITL permission requests.
    pub(crate) pending_permissions: crate::chat::PendingPermissions,
}

/// Per-iteration LLM response data bundle.
///
/// Avoids passing 7+ scalar arguments to helpers.
pub(crate) struct LlmIterationData {
    pub(crate) resp_input_tokens: u64,
    pub(crate) resp_output_tokens: u64,
    pub(crate) cost: f64,
    pub(crate) llm_elapsed_ms: u64,
    pub(crate) prompt_preview: String,
    pub(crate) response_text_raw: String,
}

/// Parameters for building the final agent execution result.
///
/// Extracted from a tuple to improve readability at the call site.
pub(crate) struct FinalResultParams {
    pub(crate) final_model: String,
    pub(crate) final_provider_id: Option<String>,
    pub(crate) owns_trace: bool,
    pub(crate) context_window: usize,
    pub(crate) reasoning_duration_ms: Option<u64>,
}

// ---------------------------------------------------------------------------
// AgentService
// ---------------------------------------------------------------------------

/// Unified agent execution service.
///
/// All agents -- interactive chat (root), sub-agents, and system agents --
/// run through [`AgentService::execute`]. The difference between agents is
/// configuration, not code path.
pub struct AgentService;

impl AgentService {
    /// Execute an agent with full capabilities.
    ///
    /// The execution loop:
    /// 1. (Optional) Assemble context pipeline for system prompt
    /// 2. Build messages with system prompt
    /// 3. LLM call via `ProviderPool`
    /// 4. If tool calls: execute tools, append results, loop (up to `max_iterations`)
    /// 5. Return final text + diagnostics
    pub async fn execute(
        container: &ServiceContainer,
        config: &AgentExecutionConfig,
        progress: Option<TurnEventSender>,
        cancel: Option<tokio_util::sync::CancellationToken>,
    ) -> Result<AgentExecutionResult, AgentExecutionError> {
        // 1. Context assembly + diagnostics trace (extracted to keep execute() under 200 lines).
        let (assembled, trace_id, owns_trace) =
            executor::init_context_and_trace(container, config).await;

        // Set up DIAGNOSTICS_CTX so gateways can record observations
        // automatically. If no trace_id, we still run without a context.
        let diag_ctx = trace_id.map(|tid| {
            y_diagnostics::DiagnosticsContext::new(
                tid,
                Some(config.session_uuid),
                config.agent_name.clone(),
            )
        });

        // Delegate to the inner execute logic, optionally scoped with
        // the diagnostics context task-local.
        if let Some(ctx) = diag_ctx {
            y_diagnostics::DIAGNOSTICS_CTX
                .scope(
                    ctx,
                    executor::execute_inner(
                        container, config, progress, cancel, assembled, trace_id, owns_trace,
                    ),
                )
                .await
        } else {
            executor::execute_inner(
                container, config, progress, cancel, assembled, trace_id, owns_trace,
            )
            .await
        }
    }

    /// Build LLM messages by prepending system prompt from assembled context.
    ///
    /// Delegates to [`crate::message_builder::build_chat_messages`].
    pub fn build_chat_messages(
        assembled: &y_context::AssembledContext,
        history: &[Message],
    ) -> Vec<Message> {
        crate::message_builder::build_chat_messages(assembled, history)
    }

    /// Filter tool definitions by an agent's allowed/denied tool lists.
    ///
    /// Returns the raw [`ToolDefinition`](y_core::tool::ToolDefinition)s so
    /// callers can both build JSON tool schemas and generate a tools summary
    /// for prompt injection.
    ///
    /// - `"*"` in `allowed` means all tools in the registry.
    /// - Empty `allowed` means no tools (returns empty vec).
    /// - `denied` overrides `allowed`.
    pub(crate) async fn filter_tool_definitions(
        container: &ServiceContainer,
        allowed: &[String],
        denied: &[String],
    ) -> Vec<y_core::tool::ToolDefinition> {
        if allowed.is_empty() {
            return vec![];
        }

        let defs = container.tool_registry.get_all_definitions().await;
        let allow_all = allowed.iter().any(|a| a == "*");

        defs.into_iter()
            .filter(|def| {
                let name = def.name.as_str();
                let is_allowed = allow_all || allowed.iter().any(|a| a == name);
                let is_denied = denied.iter().any(|d| d == name);
                is_allowed && !is_denied
            })
            .collect()
    }

    /// Build tool definitions filtered by an agent's allowed/denied tool lists.
    ///
    /// Returns `OpenAI` function-calling JSON format. Delegates filtering to
    /// `filter_tool_definitions`.
    pub async fn build_filtered_tool_definitions(
        container: &ServiceContainer,
        allowed: &[String],
        denied: &[String],
    ) -> Vec<serde_json::Value> {
        Self::filter_tool_definitions(container, allowed, denied)
            .await
            .iter()
            .map(|def| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": def.name.as_str(),
                        "description": def.description,
                        "parameters": def.parameters,
                    }
                })
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Think tag stripping
// ---------------------------------------------------------------------------

/// Remove all `<think>...</think>` blocks and `<!-- iter -->` markers from
/// a string.
///
/// Handles multiple consecutive think blocks and unclosed tags (drops from
/// the opening tag to the end of the string). Also strips `<!-- iter -->`
/// iteration boundary markers inserted during tool-call loops.
pub(crate) fn strip_think_tags(content: &str) -> String {
    let mut result = content.to_string();
    while let Some(start) = result.find("<think>") {
        if let Some(end_offset) = result[start..].find("</think>") {
            // Remove <think>...</think> including the tags.
            let end = start + end_offset + "</think>".len();
            result = format!("{}{}", &result[..start], result[end..].trim_start());
        } else {
            // Unclosed <think> -- drop from tag to end.
            result.truncate(start);
            break;
        }
    }
    // Also strip iteration boundary markers.
    result = result.replace("<!-- iter -->", "");
    result.trim().to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use y_context::{ContextCategory, ContextItem};
    use y_core::permission_types::PermissionMode;
    use y_core::types::Role;
    use y_guardrails::{PermissionAction, PermissionDecision};

    #[test]
    fn test_build_chat_messages_prepends_system() {
        let mut assembled = y_context::AssembledContext::default();
        assembled.add(ContextItem {
            category: ContextCategory::SystemPrompt,
            content: "You are y-agent, a helpful AI assistant.".to_string(),
            token_estimate: 10,
            priority: 100,
        });

        let history = vec![Message {
            message_id: y_core::types::generate_message_id(),
            role: Role::User,
            content: "Hello".to_string(),
            tool_call_id: None,
            tool_calls: vec![],
            timestamp: y_core::types::now(),
            metadata: serde_json::Value::Null,
        }];

        let messages = AgentService::build_chat_messages(&assembled, &history);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, Role::System);
        assert!(messages[0].content.contains("y-agent"));
        assert_eq!(messages[1].role, Role::User);
    }

    #[test]
    fn test_build_chat_messages_no_system_when_empty() {
        let assembled = y_context::AssembledContext::default();
        let history = vec![Message {
            message_id: y_core::types::generate_message_id(),
            role: Role::User,
            content: "Hello".to_string(),
            tool_call_id: None,
            tool_calls: vec![],
            timestamp: y_core::types::now(),
            metadata: serde_json::Value::Null,
        }];
        let messages = AgentService::build_chat_messages(&assembled, &history);
        assert_eq!(messages.len(), 1);
    }

    #[test]
    fn test_agent_execution_error_display() {
        assert!(AgentExecutionError::LlmError {
            message: "timeout".into(),
            partial_messages: vec![],
        }
        .to_string()
        .contains("timeout"));
        assert!(
            AgentExecutionError::ToolLoopLimitExceeded { max_iterations: 10 }
                .to_string()
                .contains("10")
        );
        assert!(AgentExecutionError::Cancelled
            .to_string()
            .contains("Cancelled"));
    }

    // -- build_subagent_system_prompt tests --

    fn make_test_tool_def(name: &str) -> y_core::tool::ToolDefinition {
        y_core::tool::ToolDefinition {
            name: y_core::types::ToolName::from_string(name),
            description: format!("{name} description. Extra detail."),
            help: None,
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "arg1": {"type": "string", "description": "First argument"}
                },
                "required": ["arg1"]
            }),
            result_schema: None,
            category: y_core::tool::ToolCategory::Shell,
            tool_type: y_core::tool::ToolType::BuiltIn,
            capabilities: Default::default(),
            is_dangerous: false,
        }
    }

    #[test]
    fn test_subagent_prompt_unchanged_without_tools() {
        let base = "You are a test agent.";
        let result =
            subagent::build_subagent_system_prompt(base, &[], ToolCallingMode::PromptBased);
        assert_eq!(result, base);
    }

    #[test]
    fn test_subagent_prompt_includes_protocol_and_summary() {
        let base = "You are a test agent.";
        let defs = vec![make_test_tool_def("ShellExec")];
        let result =
            subagent::build_subagent_system_prompt(base, &defs, ToolCallingMode::PromptBased);

        assert!(result.starts_with(base));
        assert!(result.contains("Tool Usage Protocol"));
        assert!(result.contains("## Available Tools"));
        assert!(result.contains("| ShellExec |"));
    }

    #[test]
    fn test_subagent_prompt_native_mode_returns_base_and_rules() {
        let base = "You are a test agent.";
        let defs = vec![make_test_tool_def("ShellExec")];
        let result = subagent::build_subagent_system_prompt(base, &defs, ToolCallingMode::Native);

        // Native mode: tools are sent via API field, prompt includes rules but no XML/summary.
        assert!(result.starts_with(base));
        assert!(result.contains("Tool Usage Protocol"));
        assert!(!result.contains("Available Tools"));
        assert!(!result.contains("<tool_call>"));
    }

    #[test]
    fn test_subagent_prompt_preserves_base() {
        let base = "Custom system prompt with specific instructions.";
        let defs = vec![make_test_tool_def("FileRead")];
        let result =
            subagent::build_subagent_system_prompt(base, &defs, ToolCallingMode::PromptBased);

        assert!(result.starts_with(base));
        assert!(result.contains("FileRead"));
    }

    #[test]
    fn test_session_allow_all_converts_ask_to_allow() {
        let decision = PermissionDecision {
            action: PermissionAction::Ask,
            reason: "global default policy".to_string(),
        };

        let resolved = tool_dispatch::resolve_permission_decision_for_session(
            decision,
            Some(PermissionMode::BypassPermissions),
        );

        assert_eq!(resolved.action, PermissionAction::Allow);
        assert!(resolved.reason.contains("session"));
    }

    #[test]
    fn test_session_allow_all_does_not_override_deny() {
        let decision = PermissionDecision {
            action: PermissionAction::Deny,
            reason: "per-tool override for `ShellExec`".to_string(),
        };

        let resolved = tool_dispatch::resolve_permission_decision_for_session(
            decision.clone(),
            Some(PermissionMode::BypassPermissions),
        );

        assert_eq!(resolved.action, PermissionAction::Deny);
        assert_eq!(resolved.reason, decision.reason);
    }

    // -----------------------------------------------------------------------
    // strip_think_tags tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_strip_think_tags_basic() {
        let input = "<think>reasoning here</think>Final answer";
        assert_eq!(super::strip_think_tags(input), "Final answer");
    }

    #[test]
    fn test_strip_think_tags_multiple() {
        let input = "<think>first</think>Part A <think>second</think>Part B";
        assert_eq!(super::strip_think_tags(input), "Part A Part B");
    }

    #[test]
    fn test_strip_think_tags_unclosed() {
        let input = "Some text <think>never closed";
        assert_eq!(super::strip_think_tags(input), "Some text");
    }

    #[test]
    fn test_strip_think_tags_no_tags() {
        let input = "No thinking here";
        assert_eq!(super::strip_think_tags(input), "No thinking here");
    }

    #[test]
    fn test_strip_think_tags_empty_think() {
        let input = "<think></think>Content";
        assert_eq!(super::strip_think_tags(input), "Content");
    }

    #[test]
    fn test_strip_iter_markers() {
        let input = "<!-- iter -->first iteration\n<!-- iter -->second iteration\nFinal answer";
        assert_eq!(
            super::strip_think_tags(input),
            "first iteration\nsecond iteration\nFinal answer"
        );
    }

    #[test]
    fn test_strip_iter_markers_with_think_tags() {
        // Mixed legacy think tags and new iter markers.
        let input = "<think>reasoning</think><!-- iter -->iteration text\nFinal answer";
        assert_eq!(
            super::strip_think_tags(input),
            "iteration text\nFinal answer"
        );
    }

    // -----------------------------------------------------------------------
    // prune_old_tool_results tests
    // -----------------------------------------------------------------------

    fn make_msg(role: Role, content: &str) -> Message {
        Message {
            message_id: y_core::types::generate_message_id(),
            role,
            content: content.to_string(),
            tool_call_id: None,
            tool_calls: vec![],
            timestamp: y_core::types::now(),
            metadata: serde_json::Value::Null,
        }
    }

    fn make_assistant_with_tool_calls(content: &str) -> Message {
        Message {
            message_id: y_core::types::generate_message_id(),
            role: Role::Assistant,
            content: content.to_string(),
            tool_call_id: None,
            tool_calls: vec![y_core::types::ToolCallRequest {
                id: "tc_1".to_string(),
                name: "FileRead".to_string(),
                arguments: serde_json::json!({}),
            }],
            timestamp: y_core::types::now(),
            metadata: serde_json::Value::Null,
        }
    }

    fn make_tool_result(content: &str) -> Message {
        Message {
            message_id: y_core::types::generate_message_id(),
            role: Role::Tool,
            content: content.to_string(),
            tool_call_id: Some("tc_1".to_string()),
            tool_calls: vec![],
            timestamp: y_core::types::now(),
            metadata: serde_json::Value::Null,
        }
    }

    #[test]
    fn test_prune_old_tool_results_merges_and_removes() {
        let mut history = vec![
            make_msg(Role::System, "system prompt"),
            make_msg(Role::User, "user question"),
            // Old pair -- should be merged + removed
            make_assistant_with_tool_calls("<think>reasoning</think>Summary of chunk 1"),
            make_tool_result("raw chunk 1 contents"),
            // Current pair -- kept, with old summary prepended
            make_assistant_with_tool_calls("<think>more reasoning</think>Summary of chunk 2"),
            make_tool_result("raw chunk 2 contents"),
        ];

        let removed = pruning::prune_old_tool_results(&mut history);
        assert_eq!(removed, 2); // old assistant + old tool removed
        assert_eq!(history.len(), 4); // system + user + merged assistant + current tool
        assert_eq!(history[0].role, Role::System);
        assert_eq!(history[1].role, Role::User);
        assert_eq!(history[2].role, Role::Assistant);
        assert_eq!(history[3].role, Role::Tool);

        // The merged assistant should contain old summary prepended to current.
        let merged = &history[2].content;
        assert!(
            merged.starts_with("Summary of chunk 1"),
            "old summary should be prepended"
        );
        assert!(
            merged.contains("<think>more reasoning</think>Summary of chunk 2"),
            "current content (including think tags) should be preserved"
        );
        // Old thinking tags should be stripped from the merged portion.
        assert!(
            !merged.contains("<think>reasoning</think>"),
            "old thinking should be stripped"
        );
    }

    #[test]
    fn test_prune_old_tool_results_three_iterations() {
        // Simulates three iterations of progressive summarization.
        let mut history = vec![
            make_msg(Role::System, "system prompt"),
            make_msg(Role::User, "summarize document"),
            // Iteration 0
            make_assistant_with_tool_calls("chunk 1 summary"),
            make_tool_result("raw chunk 1"),
            // Iteration 1
            make_assistant_with_tool_calls("chunk 2 summary"),
            make_tool_result("raw chunk 2"),
            // Iteration 2 (latest)
            make_assistant_with_tool_calls("chunk 3 summary"),
            make_tool_result("raw chunk 3"),
        ];

        let removed = pruning::prune_old_tool_results(&mut history);
        assert_eq!(removed, 4); // 2 old assistants + 2 old tools
        assert_eq!(history.len(), 4); // system + user + merged + latest tool

        let merged = &history[2].content;
        // All old summaries should be present in order.
        assert!(merged.contains("chunk 1 summary"));
        assert!(merged.contains("chunk 2 summary"));
        assert!(merged.contains("chunk 3 summary"));
        // Only the latest tool result should remain.
        assert_eq!(history[3].content, "raw chunk 3");
    }

    #[test]
    fn test_prune_old_tool_results_preserves_user_messages() {
        let mut history = vec![
            make_msg(Role::System, "system prompt"),
            make_msg(Role::User, "question 1"),
            make_assistant_with_tool_calls("old summary"),
            make_tool_result("old result"),
            make_msg(Role::User, "question 2"),
            make_assistant_with_tool_calls("new summary"),
            make_tool_result("new result"),
        ];

        let removed = pruning::prune_old_tool_results(&mut history);
        assert_eq!(removed, 2); // old assistant + old tool
        assert_eq!(history.len(), 5); // system + user1 + user2 + merged + tool
        assert!(history.iter().filter(|m| m.role == Role::User).count() == 2);
        // Merged assistant should have "old summary" prepended.
        let asst = history.iter().find(|m| m.role == Role::Assistant).unwrap();
        assert!(asst.content.contains("old summary"));
        assert!(asst.content.contains("new summary"));
    }

    #[test]
    fn test_prune_old_tool_results_no_assistant() {
        let mut history = vec![
            make_msg(Role::System, "prompt"),
            make_msg(Role::User, "hello"),
        ];
        let removed = pruning::prune_old_tool_results(&mut history);
        assert_eq!(removed, 0);
        assert_eq!(history.len(), 2);
    }

    #[test]
    fn test_prune_old_tool_results_single_pair() {
        // Only one assistant+tool pair -- nothing to prune.
        let mut history = vec![
            make_msg(Role::System, "prompt"),
            make_msg(Role::User, "hello"),
            make_assistant_with_tool_calls("call tool"),
            make_tool_result("result"),
        ];
        let removed = pruning::prune_old_tool_results(&mut history);
        assert_eq!(removed, 0);
        assert_eq!(history.len(), 4);
    }

    // -----------------------------------------------------------------------
    // strip_historical_thinking tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_strip_historical_thinking_removes_think_tags() {
        let mut history = vec![
            make_msg(Role::System, "prompt"),
            make_msg(Role::User, "hello"),
            // Historical assistant -- should have <think> stripped
            {
                let mut m = make_msg(Role::Assistant, "<think>reasoning</think>Answer 1");
                m.metadata = serde_json::json!({"reasoning_content": "deep thought"});
                m
            },
            // Current (latest) assistant -- should be preserved
            {
                let mut m = make_msg(Role::Assistant, "<think>current reasoning</think>Answer 2");
                m.metadata = serde_json::json!({"reasoning_content": "current thought"});
                m
            },
        ];

        pruning::strip_historical_thinking(&mut history);

        // Historical assistant: think tags and reasoning_content removed
        assert_eq!(history[2].content, "Answer 1");
        assert!(history[2].metadata.get("reasoning_content").is_none());

        // Current assistant: preserved intact
        assert!(history[3].content.contains("<think>"));
        assert!(history[3].metadata.get("reasoning_content").is_some());
    }

    #[test]
    fn test_strip_historical_thinking_skips_non_assistant() {
        let mut history = vec![
            make_msg(Role::User, "<think>user text</think>question"),
            make_msg(Role::Assistant, "answer"),
        ];
        pruning::strip_historical_thinking(&mut history);
        // User message content should not be modified
        assert!(history[0].content.contains("<think>"));
    }
}
