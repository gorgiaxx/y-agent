//! Unified Agent Service — single execution path for all agents.
//!
//! Every agent (interactive chat, sub-agents, system agents) runs through
//! the same [`AgentService::execute`] loop. The agent's capabilities (tools,
//! knowledge, iteration limits) are controlled by its [`AgentExecutionConfig`],
//! not by separate code paths.
//!
//! When `max_iterations=1` and `allowed_tools` is empty, the loop naturally
//! degrades to a single LLM call (equivalent to the old `SingleTurnRunner`).

use std::sync::Arc;

use futures::StreamExt;
use tokio_util::sync::CancellationToken;
use tracing::warn;
use uuid::Uuid;

use y_context::{AssembledContext, ContextCategory, ContextRequest};
use y_core::agent::{AgentRunConfig, AgentRunOutput, AgentRunner, DelegationError};
use y_core::provider::{ChatRequest, ProviderPool, RouteRequest, ToolCallingMode};
use y_core::runtime::CommandRunner;
use y_core::tool::ToolInput;
use y_core::types::{Message, ProviderId, Role, SessionId, ToolCallRequest, ToolName};
use y_tools::{format_tool_result, parse_tool_calls, strip_tool_call_blocks};

use crate::container::ServiceContainer;
use crate::cost::CostService;

// Re-use progress event types from chat module.
pub use crate::chat::{ToolCallRecord, TurnEvent, TurnEventSender};

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
    /// Tool definitions in OpenAI function-calling JSON format.
    /// Empty = no tool calling.
    pub tool_definitions: Vec<serde_json::Value>,
    /// Tool calling mode (Native or PromptBased).
    pub tool_calling_mode: ToolCallingMode,
    /// Conversation messages (system prompt prepended by caller if needed).
    pub messages: Vec<Message>,
    /// Provider routing preference.
    pub provider_id: Option<String>,
    /// Preferred model identifiers (tried in order via RouteRequest).
    pub preferred_models: Vec<String>,
    /// Provider routing tags.
    pub provider_tags: Vec<String>,
    /// Temperature override (None = use provider default).
    pub temperature: Option<f64>,
    /// Max tokens to generate.
    pub max_tokens: Option<u32>,
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
}

/// Error returned by [`AgentService::execute`].
#[derive(Debug)]
pub enum AgentExecutionError {
    /// LLM request failed.
    LlmError(String),
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
            AgentExecutionError::LlmError(msg) => write!(f, "LLM error: {msg}"),
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
// AgentService
// ---------------------------------------------------------------------------

/// Unified agent execution service.
///
/// All agents — interactive chat (root), sub-agents, and system agents —
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
        cancel: Option<CancellationToken>,
    ) -> Result<AgentExecutionResult, AgentExecutionError> {
        // 1. Context assembly (optional — root agent uses pipeline, sub-agents don't).
        let assembled = if config.use_context_pipeline {
            let context_request = ContextRequest {
                user_query: config.user_query.clone(),
                session_id: config.session_id.clone(),
                knowledge_collections: config.knowledge_collections.clone(),
                ..Default::default()
            };
            container
                .context_pipeline
                .assemble_with_request(Some(context_request))
                .await
                .unwrap_or_else(|e| {
                    warn!(error = %e, "context pipeline assembly failed; using empty context");
                    AssembledContext::default()
                })
        } else {
            AssembledContext::default()
        };

        // 2. Start diagnostics trace.
        let trace_id = container
            .diagnostics
            .on_trace_start(config.session_uuid, &config.agent_name, &config.user_query)
            .await
            .ok();

        // 3. Build initial working history.
        //    For context-pipeline agents (root), prepend system prompt from assembled context.
        //    For sub-agents, the system prompt is already in config.messages.
        let mut working_history = if config.use_context_pipeline {
            Self::build_chat_messages(&assembled, &config.messages)
        } else {
            config.messages.clone()
        };

        // Mutable state for the tool-call loop.
        let mut iteration = 0usize;
        let mut last_gen_id: Option<Uuid> = None;
        let mut tool_calls_executed: Vec<ToolCallRecord> = Vec::new();
        let mut new_messages: Vec<Message> = Vec::new();
        let mut cumulative_input_tokens: u64 = 0;
        let mut cumulative_output_tokens: u64 = 0;
        let mut cumulative_cost: f64 = 0.0;
        #[allow(unused_assignments)]
        let mut final_model = String::new();
        #[allow(unused_assignments)]
        let mut final_provider_id: Option<String> = None;
        let mut accumulated_content = String::new();

        let max_iterations = config.max_iterations;
        let session_id_ref = config.session_id.as_ref();

        loop {
            // Check for cancellation at the top of every iteration.
            if let Some(ref tok) = cancel {
                if tok.is_cancelled() {
                    return Err(AgentExecutionError::Cancelled);
                }
            }

            iteration += 1;
            if iteration > max_iterations {
                if let Some(ref tx) = progress {
                    let _ = tx.send(TurnEvent::LoopLimitHit {
                        iterations: iteration - 1,
                        max_iterations,
                    });
                }
                if let Some(tid) = trace_id {
                    let _ = container
                        .diagnostics
                        .on_trace_end(tid, false, Some("tool loop limit exceeded"))
                        .await;
                }
                return Err(AgentExecutionError::ToolLoopLimitExceeded { max_iterations });
            }

            // Build ChatRequest.
            let request = ChatRequest {
                messages: working_history.clone(),
                model: None,
                max_tokens: config.max_tokens,
                temperature: config.temperature,
                top_p: None,
                tools: config.tool_definitions.clone(),
                tool_calling_mode: config.tool_calling_mode,
                stop: vec![],
                extra: serde_json::Value::Null,
            };

            let route = RouteRequest {
                preferred_provider_id: config.provider_id.as_ref().map(ProviderId::from_string),
                preferred_model: config.preferred_models.first().cloned(),
                required_tags: config.provider_tags.clone(),
                ..RouteRequest::default()
            };

            // Fallback prompt preview.
            let prompt_preview_fallback =
                serde_json::to_string(&request.messages).unwrap_or_default();

            let llm_start = std::time::Instant::now();
            let pool = container.provider_pool().await;

            // Streaming vs non-streaming.
            let llm_result = if progress.is_some() {
                Self::call_llm_streaming(
                    &*pool,
                    &request,
                    &route,
                    progress.as_ref(),
                    cancel.as_ref(),
                )
                .await
            } else {
                let llm_future = pool.chat_completion(&request, &route);
                if let Some(ref tok) = cancel {
                    tokio::select! {
                        res = llm_future => res,
                        () = tok.cancelled() => {
                            return Err(AgentExecutionError::Cancelled);
                        }
                    }
                } else {
                    llm_future.await
                }
            };

            match llm_result {
                Ok(response) => {
                    let llm_elapsed_ms = llm_start.elapsed().as_millis() as u64;
                    let resp_input_tokens = u64::from(response.usage.input_tokens);
                    let resp_output_tokens = u64::from(response.usage.output_tokens);
                    let cost = CostService::compute_cost(resp_input_tokens, resp_output_tokens);

                    cumulative_input_tokens += resp_input_tokens;
                    cumulative_output_tokens += resp_output_tokens;
                    cumulative_cost += cost;
                    final_model = response.model.clone();
                    final_provider_id = response.provider_id.as_ref().map(|id| id.to_string());

                    // Prompt preview.
                    let prompt_preview = response.raw_request.as_ref().map_or_else(
                        || prompt_preview_fallback.clone(),
                        |v| serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string()),
                    );

                    // Response text for diagnostics.
                    let response_text_raw = response.raw_response.as_ref().map_or_else(
                        || {
                            serde_json::json!({
                                "content": response.content.clone().unwrap_or_default(),
                                "model": response.model,
                                "usage": {
                                    "input_tokens": resp_input_tokens,
                                    "output_tokens": resp_output_tokens,
                                }
                            })
                            .to_string()
                        },
                        |v| v.to_string(),
                    );

                    // Diagnostics: record generation observation.
                    if let Some(tid) = trace_id {
                        let diag_input = response.raw_request.clone().unwrap_or_else(|| {
                            serde_json::from_str(&prompt_preview_fallback)
                                .unwrap_or(serde_json::Value::Null)
                        });
                        let diag_output = response.raw_response.clone().unwrap_or_else(|| {
                            serde_json::json!({
                                "content": response.content.clone().unwrap_or_default(),
                                "model": response.model,
                                "usage": {
                                    "input_tokens": resp_input_tokens,
                                    "output_tokens": resp_output_tokens,
                                }
                            })
                        });

                        let gen_id = container
                            .diagnostics
                            .on_generation(
                                tid,
                                None,
                                Some(config.session_uuid),
                                &response.model,
                                resp_input_tokens,
                                resp_output_tokens,
                                cost,
                                diag_input,
                                diag_output,
                                llm_elapsed_ms,
                            )
                            .await
                            .ok();
                        last_gen_id = gen_id;

                        tracing::debug!(
                            trace_id = %tid,
                            agent = %config.agent_name,
                            model = %response.model,
                            input_tokens = resp_input_tokens,
                            output_tokens = resp_output_tokens,
                            llm_ms = llm_elapsed_ms,
                            "Diagnostics: agent LLM call recorded"
                        );
                    }

                    // Gather tool call names for the progress event.
                    let native_tc_names: Vec<String> = response
                        .tool_calls
                        .iter()
                        .map(|tc| tc.name.clone())
                        .collect();

                    // Handle native tool calls.
                    if !response.tool_calls.is_empty() {
                        // Emit LlmResponse progress event.
                        if let Some(ref tx) = progress {
                            let _ = tx.send(TurnEvent::LlmResponse {
                                iteration,
                                model: response.model.clone(),
                                input_tokens: resp_input_tokens,
                                output_tokens: resp_output_tokens,
                                duration_ms: llm_elapsed_ms,
                                cost_usd: cost,
                                tool_calls_requested: native_tc_names,
                                prompt_preview: prompt_preview.clone(),
                                response_text: response_text_raw.clone(),
                            });
                        }

                        let mut meta = serde_json::json!({ "model": response.model });
                        if let Some(ref rc) = response.reasoning_content {
                            meta["reasoning_content"] = serde_json::Value::String(rc.clone());
                        }
                        let assistant_msg = Message {
                            message_id: y_core::types::generate_message_id(),
                            role: Role::Assistant,
                            content: response.content.clone().unwrap_or_default(),
                            tool_call_id: None,
                            tool_calls: response.tool_calls.clone(),
                            timestamp: y_core::types::now(),
                            metadata: meta,
                        };
                        accumulated_content.push_str(&assistant_msg.content);
                        accumulated_content.push('\n');
                        working_history.push(assistant_msg.clone());
                        new_messages.push(assistant_msg);

                        for tc in &response.tool_calls {
                            let tool_start = std::time::Instant::now();
                            let tool_result = Self::execute_tool_call(
                                container,
                                tc,
                                session_id_ref.unwrap_or(&SessionId("agent".into())),
                            )
                            .await;
                            let tool_elapsed_ms = tool_start.elapsed().as_millis() as u64;

                            let (tool_success, result_content) = match &tool_result {
                                Ok(output) => {
                                    let content = serde_json::to_string(&output.content)
                                        .unwrap_or_else(|_| "{}".to_string());
                                    (output.success, content)
                                }
                                Err(e) => {
                                    let content =
                                        serde_json::json!({ "error": e.to_string() }).to_string();
                                    (false, content)
                                }
                            };

                            tool_calls_executed.push(ToolCallRecord {
                                name: tc.name.clone(),
                                success: tool_success,
                                duration_ms: tool_elapsed_ms,
                                result_content: result_content.clone(),
                            });

                            // Emit ToolResult progress event.
                            if let Some(ref tx) = progress {
                                let _ = tx.send(TurnEvent::ToolResult {
                                    name: tc.name.clone(),
                                    success: tool_success,
                                    duration_ms: tool_elapsed_ms,
                                    input_preview: serde_json::to_string(&tc.arguments)
                                        .unwrap_or_default(),
                                    result_preview: result_content.clone(),
                                });
                            }

                            // Diagnostics: record tool call observation.
                            if let Some(tid) = trace_id {
                                let tool_output_json: serde_json::Value = serde_json::from_str(
                                    &result_content,
                                )
                                .unwrap_or(serde_json::Value::String(result_content.clone()));
                                let _ = container
                                    .diagnostics
                                    .on_tool_call(
                                        tid,
                                        last_gen_id,
                                        Some(config.session_uuid),
                                        &tc.name,
                                        tc.arguments.clone(),
                                        tool_output_json,
                                        tool_elapsed_ms,
                                        tool_success,
                                    )
                                    .await;
                            }

                            let tool_msg = Message {
                                message_id: y_core::types::generate_message_id(),
                                role: Role::Tool,
                                content: result_content,
                                tool_call_id: Some(tc.id.clone()),
                                tool_calls: vec![],
                                timestamp: y_core::types::now(),
                                metadata: serde_json::Value::Null,
                            };
                            working_history.push(tool_msg.clone());
                            new_messages.push(tool_msg);
                        }

                        continue;
                    }

                    // PromptBased mode: parse <tool_call> tags from text.
                    if config.tool_calling_mode == ToolCallingMode::PromptBased {
                        if let Some(ref text) = response.content {
                            let parse_result = parse_tool_calls(text);
                            if !parse_result.tool_calls.is_empty() {
                                // Emit LlmResponse progress event.
                                if let Some(ref tx) = progress {
                                    let prompt_tc_names: Vec<String> = parse_result
                                        .tool_calls
                                        .iter()
                                        .map(|ptc| ptc.name.clone())
                                        .collect();
                                    let _ = tx.send(TurnEvent::LlmResponse {
                                        iteration,
                                        model: response.model.clone(),
                                        input_tokens: resp_input_tokens,
                                        output_tokens: resp_output_tokens,
                                        duration_ms: llm_elapsed_ms,
                                        cost_usd: cost,
                                        tool_calls_requested: prompt_tc_names,
                                        prompt_preview: prompt_preview.clone(),
                                        response_text: response_text_raw.clone(),
                                    });
                                }
                                let mut meta = serde_json::json!({ "model": response.model });
                                if let Some(ref rc) = response.reasoning_content {
                                    meta["reasoning_content"] =
                                        serde_json::Value::String(rc.clone());
                                }
                                let assistant_msg = Message {
                                    message_id: y_core::types::generate_message_id(),
                                    role: Role::Assistant,
                                    content: text.clone(),
                                    tool_call_id: None,
                                    tool_calls: vec![],
                                    timestamp: y_core::types::now(),
                                    metadata: meta,
                                };
                                accumulated_content.push_str(text);
                                accumulated_content.push('\n');
                                working_history.push(assistant_msg.clone());
                                new_messages.push(assistant_msg);

                                // Execute each parsed tool call.
                                let mut result_blocks = Vec::new();
                                for ptc in &parse_result.tool_calls {
                                    let tc = ToolCallRequest {
                                        id: format!("prompt_{}", uuid::Uuid::new_v4()),
                                        name: ptc.name.clone(),
                                        arguments: ptc.arguments.clone(),
                                    };

                                    let tool_start = std::time::Instant::now();
                                    let tool_result = Self::execute_tool_call(
                                        container,
                                        &tc,
                                        session_id_ref.unwrap_or(&SessionId("agent".into())),
                                    )
                                    .await;
                                    let tool_elapsed_ms = tool_start.elapsed().as_millis() as u64;

                                    let (tool_success, result_content) = match &tool_result {
                                        Ok(output) => {
                                            let content = serde_json::to_string(&output.content)
                                                .unwrap_or_else(|_| "{}".to_string());
                                            (output.success, content)
                                        }
                                        Err(e) => {
                                            let content = serde_json::json!({
                                                "error": e.to_string()
                                            })
                                            .to_string();
                                            (false, content)
                                        }
                                    };

                                    tool_calls_executed.push(ToolCallRecord {
                                        name: tc.name.clone(),
                                        success: tool_success,
                                        duration_ms: tool_elapsed_ms,
                                        result_content: result_content.clone(),
                                    });

                                    if let Some(ref tx) = progress {
                                        let _ = tx.send(TurnEvent::ToolResult {
                                            name: tc.name.clone(),
                                            success: tool_success,
                                            duration_ms: tool_elapsed_ms,
                                            input_preview: serde_json::to_string(&tc.arguments)
                                                .unwrap_or_default(),
                                            result_preview: result_content.clone(),
                                        });
                                    }

                                    // Diagnostics.
                                    if let Some(tid) = trace_id {
                                        let tool_output_json: serde_json::Value =
                                            serde_json::from_str(&result_content).unwrap_or(
                                                serde_json::Value::String(result_content.clone()),
                                            );
                                        let _ = container
                                            .diagnostics
                                            .on_tool_call(
                                                tid,
                                                last_gen_id,
                                                Some(config.session_uuid),
                                                &tc.name,
                                                tc.arguments.clone(),
                                                tool_output_json,
                                                tool_elapsed_ms,
                                                tool_success,
                                            )
                                            .await;
                                    }

                                    let result_value: serde_json::Value = serde_json::from_str(
                                        &result_content,
                                    )
                                    .unwrap_or(serde_json::Value::String(result_content.clone()));
                                    result_blocks.push(format_tool_result(
                                        &tc.name,
                                        tool_success,
                                        &result_value,
                                    ));
                                }

                                // Append results as a user message.
                                let results_text = result_blocks.join("\n");
                                let user_msg = Message {
                                    message_id: y_core::types::generate_message_id(),
                                    role: Role::User,
                                    content: results_text,
                                    tool_call_id: None,
                                    tool_calls: vec![],
                                    timestamp: y_core::types::now(),
                                    metadata: serde_json::json!({
                                        "type": "tool_result"
                                    }),
                                };
                                working_history.push(user_msg.clone());
                                new_messages.push(user_msg);

                                continue;
                            }
                        }
                    }

                    // No tool calls — text response.
                    let raw_content = response
                        .content
                        .clone()
                        .unwrap_or_else(|| "(no content)".to_string());

                    // Emit LlmResponse progress event for final iteration.
                    if let Some(ref tx) = progress {
                        let _ = tx.send(TurnEvent::LlmResponse {
                            iteration,
                            model: response.model.clone(),
                            input_tokens: resp_input_tokens,
                            output_tokens: resp_output_tokens,
                            duration_ms: llm_elapsed_ms,
                            cost_usd: cost,
                            tool_calls_requested: vec![],
                            prompt_preview: prompt_preview.clone(),
                            response_text: response_text_raw.clone(),
                        });
                    }

                    // Sanitize: strip any remaining <tool_call> XML.
                    let content = if config.tool_calling_mode == ToolCallingMode::PromptBased {
                        let stripped = strip_tool_call_blocks(&raw_content);
                        if stripped.is_empty() {
                            raw_content
                        } else {
                            stripped
                        }
                    } else {
                        raw_content
                    };

                    if let Some(tid) = trace_id {
                        let _ = container
                            .diagnostics
                            .on_trace_end(tid, true, Some(&content))
                            .await;
                    }

                    // Build the final content.
                    let final_content = if accumulated_content.is_empty() {
                        content.clone()
                    } else {
                        format!("{}{}", accumulated_content, content)
                    };

                    // Compute context_window.
                    let metadata_list = container.provider_pool().await.list_metadata();
                    let ctx_window = if let Some(ref pid) = final_provider_id {
                        metadata_list
                            .iter()
                            .find(|m| m.id.to_string() == *pid)
                            .map_or(0, |m| m.context_window)
                    } else {
                        metadata_list.first().map_or(0, |m| m.context_window)
                    };

                    return Ok(AgentExecutionResult {
                        content: final_content,
                        model: final_model,
                        provider_id: final_provider_id,
                        input_tokens: cumulative_input_tokens,
                        output_tokens: cumulative_output_tokens,
                        context_window: ctx_window,
                        cost_usd: cumulative_cost,
                        tool_calls_executed,
                        iterations: iteration,
                        new_messages,
                    });
                }
                Err(e) => {
                    // Convert streaming cancellation into Cancelled.
                    if matches!(e, y_core::provider::ProviderError::Cancelled) {
                        return Err(AgentExecutionError::Cancelled);
                    }
                    if let Some(tid) = trace_id {
                        let _ = container
                            .diagnostics
                            .on_trace_end(tid, false, Some(&e.to_string()))
                            .await;
                    }
                    return Err(AgentExecutionError::LlmError(format!("{e}")));
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    /// Build LLM messages by prepending system prompt from assembled context.
    pub fn build_chat_messages(assembled: &AssembledContext, history: &[Message]) -> Vec<Message> {
        let system_parts: Vec<&str> = assembled
            .items
            .iter()
            .filter(|item| {
                matches!(
                    item.category,
                    ContextCategory::SystemPrompt
                        | ContextCategory::Skills
                        | ContextCategory::Knowledge
                        | ContextCategory::Tools
                )
            })
            .map(|item| item.content.as_str())
            .collect();

        let mut messages = Vec::with_capacity(history.len() + 1);

        if !system_parts.is_empty() {
            let system_content = system_parts.join("\n\n");
            messages.push(Message {
                message_id: y_core::types::generate_message_id(),
                role: Role::System,
                content: system_content,
                tool_call_id: None,
                tool_calls: vec![],
                timestamp: y_core::types::now(),
                metadata: serde_json::Value::Null,
            });
        }

        messages.extend_from_slice(history);
        messages
    }

    /// Build tool definitions filtered by an agent's allowed/denied tool lists.
    ///
    /// - `"*"` in `allowed` means all tools in the registry.
    /// - Empty `allowed` means no tools.
    /// - `denied` overrides `allowed`.
    pub async fn build_filtered_tool_definitions(
        container: &ServiceContainer,
        allowed: &[String],
        denied: &[String],
    ) -> Vec<serde_json::Value> {
        if allowed.is_empty() {
            return vec![];
        }

        let defs = container.tool_registry.get_all_definitions().await;
        let allow_all = allowed.iter().any(|a| a == "*");

        defs.iter()
            .filter(|def| {
                let name = def.name.as_str();
                let is_allowed = allow_all || allowed.iter().any(|a| a == name);
                let is_denied = denied.iter().any(|d| d == name);
                is_allowed && !is_denied
            })
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

    /// Execute a tool call — delegates to the tool registry.
    ///
    /// Special handling for `tool_search`: delegates to [`ToolSearchOrchestrator`].
    async fn execute_tool_call(
        container: &ServiceContainer,
        tc: &ToolCallRequest,
        session_id: &SessionId,
    ) -> Result<y_core::tool::ToolOutput, y_core::tool::ToolError> {
        // Intercept tool_search calls.
        if tc.name == "tool_search" {
            let result = crate::tool_search_orchestrator::ToolSearchOrchestrator::handle(
                &tc.arguments,
                &container.tool_registry,
                &container.tool_taxonomy,
                &container.tool_activation_set,
            )
            .await;

            // Sync activated tool schemas into dynamic_tool_schemas.
            if result.is_ok() {
                let activation_set = container.tool_activation_set.read().await;
                let schemas: Vec<String> = activation_set
                    .active_definitions()
                    .iter()
                    .map(|def| {
                        format!(
                            "### {}\n{}\nParameters: {}",
                            def.name.as_str(),
                            def.description,
                            serde_json::to_string_pretty(&def.parameters)
                                .unwrap_or_else(|_| "{}".to_string()),
                        )
                    })
                    .collect();
                let mut dynamic = container.dynamic_tool_schemas.write().await;
                *dynamic = schemas;
            }

            return result;
        }

        let tool_name = ToolName::from_string(&tc.name);

        let tool = container
            .tool_registry
            .get_tool(&tool_name)
            .await
            .ok_or_else(|| y_core::tool::ToolError::NotFound {
                name: tc.name.clone(),
            })?;

        let input = ToolInput {
            call_id: tc.id.clone(),
            name: tool_name,
            arguments: tc.arguments.clone(),
            session_id: session_id.clone(),
            command_runner: Some(Arc::clone(&container.runtime_manager) as Arc<dyn CommandRunner>),
        };

        tool.execute(input).await
    }

    // -----------------------------------------------------------------------
    // Streaming LLM call helper
    // -----------------------------------------------------------------------

    /// Call the LLM via streaming and emit `TurnEvent::StreamDelta` events.
    ///
    /// Returns a fully assembled `ChatResponse` equivalent to the non-streaming
    /// path. Supports mid-stream cancellation via `CancellationToken`.
    async fn call_llm_streaming(
        pool: &dyn ProviderPool,
        request: &ChatRequest,
        route: &RouteRequest,
        progress: Option<&TurnEventSender>,
        cancel: Option<&CancellationToken>,
    ) -> Result<y_core::provider::ChatResponse, y_core::provider::ProviderError> {
        use y_core::provider::{ChatResponse, FinishReason, ProviderError};
        use y_core::types::TokenUsage;

        let stream_response = pool.chat_completion_stream(request, route).await?;
        let raw_request = stream_response.raw_request;
        let provider_id = stream_response.provider_id;
        let model_name = stream_response.model;
        let mut stream = stream_response.stream;

        let mut content = String::new();
        let mut reasoning_content = String::new();
        let mut tool_calls = Vec::new();
        let mut usage = TokenUsage {
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: None,
            cache_write_tokens: None,
        };
        let mut finish_reason = FinishReason::Stop;

        loop {
            // Check cancellation between chunks.
            if let Some(tok) = cancel {
                if tok.is_cancelled() {
                    return Err(ProviderError::Cancelled);
                }
            }

            let chunk_result = if let Some(tok) = cancel {
                tokio::select! {
                    next = stream.next() => next,
                    () = tok.cancelled() => {
                        return Err(ProviderError::Cancelled);
                    }
                }
            } else {
                stream.next().await
            };

            match chunk_result {
                Some(Ok(chunk)) => {
                    // Emit text delta to presentation layers.
                    if let Some(ref delta) = chunk.delta_content {
                        if !delta.is_empty() {
                            content.push_str(delta);
                            if let Some(tx) = progress {
                                let _ = tx.send(TurnEvent::StreamDelta {
                                    content: delta.clone(),
                                });
                            }
                        }
                    }

                    // Emit reasoning/thinking delta.
                    if let Some(ref reasoning) = chunk.delta_reasoning_content {
                        if !reasoning.is_empty() {
                            reasoning_content.push_str(reasoning);
                            if let Some(tx) = progress {
                                let _ = tx.send(TurnEvent::StreamReasoningDelta {
                                    content: reasoning.clone(),
                                });
                            }
                        }
                    }

                    // Collect tool calls on finish.
                    if !chunk.delta_tool_calls.is_empty() {
                        tool_calls.extend(chunk.delta_tool_calls);
                    }

                    // Capture usage from the final chunk.
                    if let Some(u) = chunk.usage {
                        usage = u;
                    }

                    if let Some(fr) = chunk.finish_reason {
                        finish_reason = fr;
                    }
                }
                Some(Err(e)) => return Err(e),
                None => break,
            }
        }

        // Build synthetic raw response for diagnostics.
        let finish_reason_str = match finish_reason {
            FinishReason::Stop => "stop",
            FinishReason::Length => "length",
            FinishReason::ToolUse => "tool_calls",
            FinishReason::ContentFilter => "content_filter",
            FinishReason::Unknown => "stop",
        };
        let raw_response = serde_json::json!({
            "id": "",
            "object": "chat.completion",
            "model": model_name,
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": content,
                },
                "finish_reason": finish_reason_str,
            }],
            "usage": {
                "prompt_tokens": usage.input_tokens,
                "completion_tokens": usage.output_tokens,
            }
        });

        Ok(ChatResponse {
            id: String::new(),
            content: if content.is_empty() {
                None
            } else {
                Some(content)
            },
            reasoning_content: if reasoning_content.is_empty() {
                None
            } else {
                Some(reasoning_content)
            },
            model: model_name,
            tool_calls,
            finish_reason,
            usage,
            raw_request,
            raw_response: Some(raw_response),
            provider_id,
        })
    }
}

// ---------------------------------------------------------------------------
// ServiceAgentRunner — bridges AgentPool.delegate() → AgentService.execute()
// ---------------------------------------------------------------------------

/// `AgentRunner` implementation that uses `AgentService::execute()`.
///
/// Replaces `SingleTurnRunner` — sub-agents now get the same execution loop
/// as the root chat agent (with capabilities controlled by `AgentRunConfig`).
pub struct ServiceAgentRunner {
    container: Arc<ServiceContainer>,
}

impl ServiceAgentRunner {
    /// Create a new `ServiceAgentRunner` backed by the given `ServiceContainer`.
    pub fn new(container: Arc<ServiceContainer>) -> Self {
        Self { container }
    }
}

#[async_trait::async_trait]
impl AgentRunner for ServiceAgentRunner {
    async fn run(&self, config: AgentRunConfig) -> Result<AgentRunOutput, DelegationError> {
        let start = std::time::Instant::now();

        // Build messages: system_prompt + input as user message.
        let mut messages = Vec::with_capacity(2);
        messages.push(Message {
            message_id: y_core::types::generate_message_id(),
            role: Role::System,
            content: config.system_prompt.clone(),
            tool_call_id: None,
            tool_calls: vec![],
            timestamp: y_core::types::now(),
            metadata: serde_json::Value::Null,
        });

        let user_content = match &config.input {
            serde_json::Value::String(s) => s.clone(),
            other => serde_json::to_string_pretty(other).unwrap_or_else(|_| other.to_string()),
        };
        messages.push(Message {
            message_id: y_core::types::generate_message_id(),
            role: Role::User,
            content: user_content.clone(),
            tool_call_id: None,
            tool_calls: vec![],
            timestamp: y_core::types::now(),
            metadata: serde_json::Value::Null,
        });

        // Build tool definitions from allowed_tools/denied_tools.
        // When allowed_tools is non-empty, agents can make tool calls across
        // multiple iterations (e.g. skill-ingestion reading companion files).
        let tool_definitions = AgentService::build_filtered_tool_definitions(
            &self.container,
            &config.allowed_tools,
            &config.denied_tools,
        )
        .await;

        // Determine max_iterations: if tools are available, use the agent
        // definition's max_iterations; otherwise single-turn.
        let max_iterations = if tool_definitions.is_empty() {
            1
        } else {
            config.max_iterations
        };

        // Determine tool calling mode: use Native when tools are available.
        let tool_calling_mode = if tool_definitions.is_empty() {
            ToolCallingMode::default()
        } else {
            ToolCallingMode::Native
        };

        let exec_config = AgentExecutionConfig {
            agent_name: config.agent_name.clone(),
            system_prompt: config.system_prompt.clone(),
            max_iterations,
            tool_definitions,
            tool_calling_mode,
            messages,
            provider_id: None,
            preferred_models: config.preferred_models.clone(),
            provider_tags: config.provider_tags.clone(),
            temperature: config.temperature,
            max_tokens: config.max_tokens,
            session_id: None,
            session_uuid: Uuid::nil(),
            knowledge_collections: vec![],
            use_context_pipeline: false,
            user_query: user_content,
        };

        let result = AgentService::execute(&self.container, &exec_config, None, None)
            .await
            .map_err(|e| DelegationError::DelegationFailed {
                message: format!(
                    "AgentService execution failed for agent '{}': {e}",
                    config.agent_name
                ),
            })?;

        if result.content.is_empty() {
            return Err(DelegationError::DelegationFailed {
                message: format!("agent '{}' returned empty response", config.agent_name),
            });
        }

        let tokens_used = result.input_tokens as u32 + result.output_tokens as u32;

        Ok(AgentRunOutput {
            text: result.content,
            tokens_used,
            input_tokens: result.input_tokens,
            output_tokens: result.output_tokens,
            model_used: result.model,
            duration_ms: start.elapsed().as_millis() as u64,
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use y_context::{ContextCategory, ContextItem};

    #[test]
    fn test_build_chat_messages_prepends_system() {
        let mut assembled = AssembledContext::default();
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
        let assembled = AssembledContext::default();
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
        assert!(AgentExecutionError::LlmError("timeout".into())
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
}
