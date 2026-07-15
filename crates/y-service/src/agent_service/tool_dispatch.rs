//! Tool execution, permission gating, and HITL approval flow.

use std::sync::Arc;

use tokio_util::sync::CancellationToken;
use y_core::permission_types::PermissionMode;
use y_core::runtime::CommandRunner;
use y_core::tool::ToolInput;
use y_core::trust::TrustTier;
use y_core::types::{SessionId, ToolCallRequest, ToolName};

use crate::container::ServiceContainer;

use super::{AgentExecutionConfig, ToolCallRecord, ToolExecContext, TurnEvent, TurnEventSender};
use crate::chat_types::OperationMode;
use crate::user_interaction_orchestrator::INTERACTION_TIMEOUT;

/// Execute a single tool call, record it, and emit progress events.
///
/// Returns `(success, result_content, metadata)`.
pub(crate) async fn execute_and_record_tool(
    container: &ServiceContainer,
    config: &AgentExecutionConfig,
    tc: &ToolCallRequest,
    progress: Option<&TurnEventSender>,
    ctx: &mut ToolExecContext,
) -> (bool, String, serde_json::Value) {
    let tool_start = std::time::Instant::now();

    if ctx
        .cancel_token
        .as_ref()
        .is_some_and(tokio_util::sync::CancellationToken::is_cancelled)
    {
        let error_content = system_tool_error_content("Cancelled by user.", false);
        emit_tool_result(
            progress,
            tc,
            config,
            false,
            0,
            error_content.clone(),
            None,
            None,
        );
        return (false, error_content, serde_json::Value::Null);
    }

    if ctx.tool_calls_executed.len() >= config.max_tool_calls {
        let error_content = system_tool_error_content(
            format!(
                "Tool call limit ({}) reached. Do NOT request more tools. \
             Finish with the information already available.",
                config.max_tool_calls
            ),
            false,
        );
        tracing::warn!(
            agent = %config.agent_name,
            tool = %tc.name,
            max_tool_calls = config.max_tool_calls,
            "tool execution blocked by max_tool_calls limit"
        );
        emit_tool_result(
            progress,
            tc,
            config,
            false,
            0,
            error_content.clone(),
            None,
            None,
        );
        return (false, error_content, serde_json::Value::Null);
    }

    // (Plan-mode tool blocking removed -- the new Plan tool orchestrator
    // handles all plan-mode logic via sub-agent delegation, no need to
    // block tools at execution time.)

    // ---------------------------------------------------------------
    // Permission gatekeeper: evaluate guardrail permission BEFORE
    // executing the tool. Reads `default_permission`, per-tool overrides,
    // and `dangerous_auto_ask` from the hot-reloadable GuardrailConfig.
    // ---------------------------------------------------------------
    let guardrail_config = container.guardrail_manager.config();
    let is_dangerous = {
        let tool_name_key = ToolName::from_string(&tc.name);
        container
            .tool_registry
            .get_definition(&tool_name_key)
            .await
            .is_some_and(|def| def.is_dangerous)
    };

    let permission_model = y_guardrails::PermissionModel::new(guardrail_config);
    let session_mode = session_permission_mode(container, &ctx.session_id).await;
    let operation_mode = session_operation_mode(container, &ctx.session_id).await;

    // Extract shell command content for exec_policy evaluation.
    let shell_command_content: Option<&str> = if tc.name == "ShellExec" {
        tc.arguments.get("command").and_then(|v| v.as_str())
    } else {
        None
    };

    // Built-in agents auto-allow their declared tools without consulting
    // global permission policy. This prevents background subagents from
    // being blocked when the user sets a global "ask" mode.
    let builtin_auto_allow = config.trust_tier == Some(TrustTier::BuiltIn)
        && config.agent_allowed_tools.iter().any(|t| t == &tc.name);

    let decision = if builtin_auto_allow {
        tracing::debug!(
            tool = %tc.name,
            agent = %config.agent_name,
            "auto-allowed: built-in agent declared tool"
        );
        y_guardrails::PermissionDecision {
            action: y_guardrails::PermissionAction::Allow,
            reason: format!("built-in agent '{}' declared tool", config.agent_name),
        }
    } else {
        // Stage 0: Exec policy (Starlark DSL for shell commands).
        // If a rule matches, its decision takes precedence.
        let ep_decision = evaluate_exec_policy(
            container.exec_policy_manager.as_ref(),
            shell_command_content,
        );

        if let Some(ep_decision) = ep_decision {
            resolve_permission_decision_for_session(ep_decision, session_mode, operation_mode)
        } else {
            // No exec policy match — fall through to generic permission model.
            resolve_permission_decision_for_session(
                permission_model.evaluate(&tc.name, is_dangerous),
                session_mode,
                operation_mode,
            )
        }
    };

    match decision.action {
        y_guardrails::PermissionAction::Deny => {
            // Denied by policy -- do NOT execute the tool.
            tracing::warn!(
                tool = %tc.name,
                reason = %decision.reason,
                "tool execution denied by permission policy"
            );
            let error_content = system_tool_error_content(
                format!(
                    "Tool '{}' is blocked by security policy ({}). \
                 Do NOT ask the user for permission or retry this tool. \
                 Use an alternative approach or skip this action.",
                    tc.name, decision.reason
                ),
                false,
            );

            record_tool_call(ctx, tc, false, 0, error_content.clone(), None, None);
            emit_tool_result(
                progress,
                tc,
                config,
                false,
                0,
                error_content.clone(),
                None,
                None,
            );

            return (false, error_content, serde_json::Value::Null);
        }
        y_guardrails::PermissionAction::Ask => {
            // Pause and ask the user for approval via HITL.
            let request_id = uuid::Uuid::new_v4().to_string();

            // Extract content preview (command for ShellExec, path for
            // file tools, etc.) for the permission prompt.
            let content_preview = permission_prompt_content_preview(&tc.arguments);
            let action_desc = permission_action_description(&tc.name, content_preview.as_deref());

            tracing::info!(
                tool = %tc.name,
                request_id = %request_id,
                reason = %decision.reason,
                "permission escalation: asking user for approval"
            );

            // Register a oneshot channel for the response.
            let (resp_tx, resp_rx) =
                tokio::sync::oneshot::channel::<crate::chat::PermissionPromptResponse>();
            {
                let mut map = ctx.pending_permissions.lock().await;
                map.insert(
                    request_id.clone(),
                    crate::chat_types::PendingPermission::new(ctx.session_id.clone(), resp_tx),
                );
            }

            // Emit the permission request event to the presentation layer.
            if let Some(tx) = progress {
                let _ = tx.send(TurnEvent::PermissionRequest {
                    request_id: request_id.clone(),
                    tool_name: tc.name.clone(),
                    action_description: action_desc,
                    reason: decision.reason.clone(),
                    content_preview,
                });
            }

            // Block until the user responds, the channel is dropped,
            // or the run is cancelled via the Stop button.
            let user_response = if let Some(ref tok) = ctx.cancel_token {
                tokio::select! {
                    resp = resp_rx => resp.ok(),
                    () = tok.cancelled() => None,
                }
            } else {
                resp_rx.await.ok()
            };
            match user_response {
                Some(crate::chat::PermissionPromptResponse::Approve) => {
                    tracing::info!(
                        tool = %tc.name,
                        request_id = %request_id,
                        "user approved tool execution"
                    );
                    // Fall through to execute the tool.
                }
                Some(crate::chat::PermissionPromptResponse::AllowAllForSession) => {
                    tracing::info!(
                        tool = %tc.name,
                        request_id = %request_id,
                        "user approved: allow all for session"
                    );
                    set_session_permission_mode(
                        container,
                        &ctx.session_id,
                        PermissionMode::BypassPermissions,
                    )
                    .await;
                    // Fall through to execute the tool.
                }
                Some(crate::chat::PermissionPromptResponse::ApproveAlways) => {
                    tracing::info!(
                        tool = %tc.name,
                        request_id = %request_id,
                        "user approved: always allow (persist exec_policy rule)"
                    );
                    persist_exec_policy_amendment(container, tc).await;
                    // Fall through to execute the tool.
                }
                Some(crate::chat::PermissionPromptResponse::Deny) | None => {
                    tracing::info!(
                        tool = %tc.name,
                        request_id = %request_id,
                        "user denied tool execution"
                    );
                    let error_content = system_tool_error_content(
                        "User denied permission for this tool call. \
                         Do NOT retry this tool. Use an alternative approach.",
                        false,
                    );

                    record_tool_call(ctx, tc, false, 0, error_content.clone(), None, None);
                    emit_tool_result(
                        progress,
                        tc,
                        config,
                        false,
                        0,
                        error_content.clone(),
                        None,
                        None,
                    );

                    return (false, error_content, serde_json::Value::Null);
                }
            }
        }
        y_guardrails::PermissionAction::Allow => {
            // Permission granted -- proceed to execute.
        }
        y_guardrails::PermissionAction::Notify => {
            // Execute, but log for auditing.
            tracing::info!(
                tool = %tc.name,
                reason = %decision.reason,
                "tool execution allowed with notification (notify mode)"
            );
        }
    }

    track_file_history(container, tc, &ctx.session_id).await;

    // ---------------------------------------------------------------
    // Actual tool execution
    // ---------------------------------------------------------------
    if let Some(tx) = progress {
        let _ = tx.send(TurnEvent::ToolStart {
            name: tc.name.clone(),
            input_preview: tool_arguments_preview(tc),
            agent_name: config.agent_name.clone(),
        });
    }

    let (tool_success, full_result, result_content, tool_metadata) = match execute_tool_call(
        container,
        config,
        tc,
        &ctx.session_id,
        ctx.working_directory.as_deref(),
        &ctx.additional_read_dirs,
        progress,
        ctx.cancel_token.as_ref(),
    )
    .await
    {
        Ok(output) => {
            let success = output.success;
            let content = normalize_tool_output_content(output.success, output.content);
            let full = serde_json::to_string(&content).unwrap_or_default();
            // For Browser/WebFetch: strip GUI-only fields (favicon_url,
            // action, search_engine, navigation) before sending to the
            // LLM. Only keep text + url/title for context.
            let stripped = strip_url_tool_result(&tc.name, &content);
            // Global safety net: ensure no tool result exceeds 10K chars in
            // the LLM path. Per-tool truncation handles most cases, but this
            // catches MCP tools, meta-tools, or any tool that slips through.
            let stripped = y_prompt::budget::truncate_tool_result(
                &stripped,
                y_prompt::budget::MAX_TOOL_RESULT_CHARS,
            );
            let metadata = (!output.metadata.is_null()).then_some(output.metadata);
            (success, full, stripped, metadata)
        }
        Err(e) => {
            let content = tool_error_content(&e);
            let msg = serde_json::to_string(&content)
                .unwrap_or_else(|_| serde_json::json!({ "error": e.to_string() }).to_string());
            (false, msg.clone(), msg, None)
        }
    };

    let tool_elapsed_ms = u64::try_from(tool_start.elapsed().as_millis()).unwrap_or(0);

    // Extract URL metadata from the full (unstripped) result before storing.
    let url_meta = extract_url_meta(&tc.name, &full_result);

    record_tool_call(
        ctx,
        tc,
        tool_success,
        tool_elapsed_ms,
        result_content.clone(),
        url_meta.clone(),
        tool_metadata.clone(),
    );

    // Build correlation metadata before tool_metadata is consumed by emit_tool_result.
    let final_meta = build_tool_correlation_metadata(tc, tool_metadata.as_ref());

    // Emit ToolResult progress event.
    emit_tool_result(
        progress,
        tc,
        config,
        tool_success,
        tool_elapsed_ms,
        result_content.clone(),
        url_meta,
        tool_metadata,
    );

    // AskUser interception: if the tool is AskUser, block until the
    // presentation layer delivers an answer via `PendingInteractions`.
    if tool_success && tc.name == "AskUser" {
        if let Some(answer) = intercept_ask_user(tc, progress, ctx, config, tool_start).await {
            return (true, answer, serde_json::Value::Null);
        }
    }

    // Auto-register agent files created via FileWrite.
    if tool_success && tc.name == "FileWrite" {
        maybe_auto_register_agent(container, &tc.arguments).await;
    }

    (tool_success, result_content, final_meta)
}

/// Build tool message metadata with a correlation ID for related tool calls.
///
/// When a tool call is part of a multi-call sequence (e.g. `ShellExec`
/// `start`/`poll`/`write`/`kill` for the same `process_id`, or Plan updates),
/// the `correlation_id` field lets the UI group these calls and update the
/// existing card instead of creating a new one for each iteration.
///
/// Currently correlates by:
/// - `ShellExec` `start`/`poll`/`write`/`kill`: `process_id` from result or args
/// - Other tools: no correlation (each call gets a unique card)
fn build_tool_correlation_metadata(
    tc: &ToolCallRequest,
    tool_metadata: Option<&serde_json::Value>,
) -> serde_json::Value {
    let mut meta = tool_metadata.cloned().unwrap_or(serde_json::Value::Null);

    // Extract correlation ID for ShellExec background process operations.
    if tc.name == "ShellExec" {
        let action = tc
            .arguments
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("run");

        if matches!(action, "start" | "poll" | "write" | "kill") {
            // Try to get process_id from arguments (poll/write/kill) or
            // from the tool result metadata (start returns a new process_id).
            let process_id = tc
                .arguments
                .get("process_id")
                .and_then(|v| v.as_str())
                .map(String::from)
                .or_else(|| {
                    meta.get("process_id")
                        .and_then(|v| v.as_str())
                        .map(String::from)
                });

            if let Some(pid) = process_id {
                if meta.is_null() {
                    meta = serde_json::json!({});
                }
                if let Some(obj) = meta.as_object_mut() {
                    obj.insert(
                        "correlation_id".to_string(),
                        serde_json::Value::String(format!("shellexec:{pid}")),
                    );
                    obj.insert(
                        "correlation_action".to_string(),
                        serde_json::Value::String(action.to_string()),
                    );
                }
            }
        }
    }

    meta
}

fn tool_arguments_preview(tc: &ToolCallRequest) -> String {
    serde_json::to_string(&tc.arguments).unwrap_or_default()
}

fn normalize_tool_output_content(success: bool, content: serde_json::Value) -> serde_json::Value {
    match content {
        serde_json::Value::Object(_) => content,
        value if success => serde_json::json!({ "result": value }),
        value => serde_json::json!({ "error": value }),
    }
}

fn system_tool_error_content(message: impl AsRef<str>, retryable: bool) -> String {
    serde_json::json!({
        "error": message.as_ref(),
        "retryable": retryable,
    })
    .to_string()
}

fn tool_error_content(error: &y_core::tool::ToolError) -> serde_json::Value {
    serde_json::json!({
        "error": error.to_string(),
        "retryable": error.is_retryable(),
    })
}

fn emit_tool_result(
    progress: Option<&TurnEventSender>,
    tc: &ToolCallRequest,
    config: &AgentExecutionConfig,
    success: bool,
    duration_ms: u64,
    result_preview: String,
    url_meta: Option<String>,
    metadata: Option<serde_json::Value>,
) {
    if let Some(tx) = progress {
        let _ = tx.send(TurnEvent::ToolResult {
            name: tc.name.clone(),
            success,
            duration_ms,
            input_preview: tool_arguments_preview(tc),
            result_preview,
            agent_name: config.agent_name.clone(),
            url_meta,
            metadata,
        });
    }
}

fn record_tool_call(
    ctx: &mut ToolExecContext,
    tc: &ToolCallRequest,
    success: bool,
    duration_ms: u64,
    result_content: String,
    url_meta: Option<String>,
    metadata: Option<serde_json::Value>,
) {
    ctx.tool_calls_executed.push(ToolCallRecord {
        name: tc.name.clone(),
        arguments: tool_arguments_preview(tc),
        success,
        duration_ms,
        result_content,
        url_meta,
        metadata,
    });
}

fn permission_prompt_content_preview(arguments: &serde_json::Value) -> Option<String> {
    arguments
        .get("command")
        .or_else(|| arguments.get("path"))
        .or_else(|| arguments.get("url"))
        .and_then(|v| v.as_str())
        .map(String::from)
}

fn permission_action_description(tool_name: &str, content_preview: Option<&str>) -> String {
    if let Some(preview) = content_preview {
        format!("{tool_name} wants to execute: {preview}")
    } else {
        format!("{tool_name} wants to execute")
    }
}

/// Block until the user answers an `AskUser` question, then update the
/// `ToolCallRecord` and emit an updated `ToolResult` event with the real answer.
///
/// Returns `Some(answer_content)` if the user answered or the interaction
/// timed out, `None` if the questions field is missing or no progress channel
/// exists to surface the prompt.
async fn intercept_ask_user(
    tc: &ToolCallRequest,
    progress: Option<&TurnEventSender>,
    ctx: &mut ToolExecContext,
    config: &AgentExecutionConfig,
    tool_start: std::time::Instant,
) -> Option<String> {
    let questions = tc.arguments.get("questions")?;
    let tx = progress?;

    let interaction_id = uuid::Uuid::new_v4().to_string();
    let (answer_tx, answer_rx) = tokio::sync::oneshot::channel::<serde_json::Value>();
    {
        let mut map = ctx.pending_interactions.lock().await;
        map.insert(
            interaction_id.clone(),
            crate::chat_types::PendingInteraction::new(ctx.session_id.clone(), answer_tx),
        );
    }

    let _ = tx.send(TurnEvent::UserInteractionRequest {
        interaction_id: interaction_id.clone(),
        questions: questions.clone(),
    });

    // Block this iteration until the user answers.
    let answer = if let Some(ref tok) = ctx.cancel_token {
        tokio::select! {
            answer = tokio::time::timeout(INTERACTION_TIMEOUT, answer_rx) => answer.ok().and_then(Result::ok),
            () = tok.cancelled() => None,
        }
    } else {
        tokio::time::timeout(INTERACTION_TIMEOUT, answer_rx)
            .await
            .ok()
            .and_then(Result::ok)
    };

    let answer_content = if let Some(answer) = answer {
        serde_json::to_string(&answer).unwrap_or_else(|_| answer.to_string())
    } else {
        ctx.pending_interactions
            .lock()
            .await
            .remove(&interaction_id);
        serde_json::json!({
            "status": "timeout",
            "message": "User interaction timed out. Continue without these answers."
        })
        .to_string()
    };
    let answer_content = y_prompt::budget::truncate_tool_result(
        &answer_content,
        y_prompt::budget::MAX_TOOL_RESULT_CHARS,
    );

    // Update the already-pushed ToolCallRecord with the real user answer so
    // diagnostics and session persistence reflect the actual result instead
    // of the echoed questions.
    let total_ms = u64::try_from(tool_start.elapsed().as_millis()).unwrap_or(0);
    if let Some(record) = ctx.tool_calls_executed.last_mut() {
        record.result_content.clone_from(&answer_content);
        record.duration_ms = total_ms;
    }

    // Emit an updated ToolResult event so the GUI can refresh the tool card
    // with the real answer.
    emit_tool_result(
        progress,
        tc,
        config,
        true,
        total_ms,
        answer_content.clone(),
        None,
        None,
    );

    Some(answer_content)
}

/// Resolve permission decision applying session-level overrides.
pub(crate) fn resolve_permission_decision_for_session(
    decision: y_guardrails::PermissionDecision,
    session_mode: Option<PermissionMode>,
    operation_mode: Option<OperationMode>,
) -> y_guardrails::PermissionDecision {
    match session_mode {
        _ if operation_mode == Some(OperationMode::FullAccess) => {
            y_guardrails::PermissionDecision {
                action: y_guardrails::PermissionAction::Allow,
                reason: "session operation mode (full_access)".to_string(),
            }
        }
        Some(PermissionMode::BypassPermissions)
            if decision.action != y_guardrails::PermissionAction::Deny =>
        {
            y_guardrails::PermissionDecision {
                action: y_guardrails::PermissionAction::Allow,
                reason: format!(
                    "session permission override ({})",
                    PermissionMode::BypassPermissions
                ),
            }
        }
        _ => decision,
    }
}

/// Tokenize a shell command for exec policy matching.
///
/// Returns `None` for commands with shell metacharacters (pipes, redirects,
/// subshells) that make simple tokenization unreliable. The exec policy only
/// applies to simple prefix commands.
fn tokenize_shell_command_for_exec_policy(command: &str) -> Option<Vec<String>> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Skip commands with shell metacharacters.
    if trimmed.contains('|')
        || trimmed.contains('>')
        || trimmed.contains('&')
        || trimmed.contains(';')
        || trimmed.contains('$')
        || trimmed.contains('`')
    {
        return None;
    }

    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    for ch in trimmed.chars() {
        match ch {
            '\'' if !in_double_quote => in_single_quote = !in_single_quote,
            '"' if !in_single_quote => in_double_quote = !in_double_quote,
            c if c.is_whitespace() && !in_single_quote && !in_double_quote => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }

    if in_single_quote || in_double_quote {
        return None;
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    if tokens.is_empty() {
        None
    } else {
        Some(tokens)
    }
}

/// Evaluate a shell command against the exec policy if configured.
fn evaluate_exec_policy(
    manager: Option<&Arc<y_guardrails::ExecPolicyManager>>,
    shell_command: Option<&str>,
) -> Option<y_guardrails::PermissionDecision> {
    let mgr = manager?;
    let cmd = shell_command?;
    let tokens = tokenize_shell_command_for_exec_policy(cmd)?;
    let eval = mgr.evaluate(&tokens)?;
    let action = match eval.decision {
        y_guardrails::ExecDecision::Allow => y_guardrails::PermissionAction::Allow,
        y_guardrails::ExecDecision::Ask => y_guardrails::PermissionAction::Ask,
        y_guardrails::ExecDecision::Deny => y_guardrails::PermissionAction::Deny,
    };
    let justification = eval
        .determining_justification()
        .unwrap_or("exec policy rule")
        .to_string();
    Some(y_guardrails::PermissionDecision {
        action,
        reason: format!("exec_policy: {justification}"),
    })
}

pub(crate) async fn session_permission_mode(
    container: &ServiceContainer,
    session_id: &SessionId,
) -> Option<PermissionMode> {
    let modes = container
        .session_state
        .session_permission_modes
        .read()
        .await;
    modes.get(session_id).copied()
}

pub(crate) async fn session_operation_mode(
    container: &ServiceContainer,
    session_id: &SessionId,
) -> Option<OperationMode> {
    let modes = container.session_state.session_operation_modes.read().await;
    modes.get(session_id).copied()
}

pub(crate) async fn set_session_permission_mode(
    container: &ServiceContainer,
    session_id: &SessionId,
    mode: PermissionMode,
) {
    let mut modes = container
        .session_state
        .session_permission_modes
        .write()
        .await;
    modes.insert(session_id.clone(), mode);
}

/// Persist an `exec_policy` amendment for a `ShellExec` tool call.
///
/// Called when the user responds with "Always Allow" — derives a prefix
/// rule from the command tokens and appends it to the policy file.
async fn persist_exec_policy_amendment(container: &ServiceContainer, tc: &ToolCallRequest) {
    let Some(mgr) = &container.exec_policy_manager else {
        return;
    };
    if tc.name != "ShellExec" {
        return;
    }
    let Some(cmd_str) = tc.arguments.get("command").and_then(|v| v.as_str()) else {
        return;
    };
    let Some(tokens) = tokenize_shell_command_for_exec_policy(cmd_str) else {
        return;
    };
    let proposed = y_guardrails::ExecPolicyManager::propose_amendment(&tokens);
    if let Err(e) = mgr.persist_amendment(proposed).await {
        tracing::warn!(
            tool = %tc.name,
            error = %e,
            "failed to persist exec_policy amendment"
        );
    }
}

/// Resolve the root session ID for file history tracking.
///
/// Sub-agent file edits must be tracked under the root session's
/// `FileHistoryManager` so that rewind operates correctly for
/// Plan/Loop multi-agent executions.
async fn resolve_root_session_for_history(
    container: &ServiceContainer,
    session_id: &SessionId,
) -> SessionId {
    match container.session_manager.get_session(session_id).await {
        Ok(node) => node.root_id,
        Err(_) => session_id.clone(),
    }
}

/// Capture file state before mutating tools so rewind can restore it.
async fn track_file_history(
    container: &ServiceContainer,
    tc: &ToolCallRequest,
    session_id: &SessionId,
) {
    let file_path = match tc.name.as_str() {
        "FileWrite" | "FileCreate" | "FileDelete" | "FileMove" => tc
            .arguments
            .get("path")
            .or_else(|| tc.arguments.get("source"))
            .and_then(|v| v.as_str())
            .map(String::from),
        _ => None,
    };
    if let Some(ref path) = file_path {
        let root_id = resolve_root_session_for_history(container, session_id).await;
        crate::rewind::RewindService::track_edit(&container.file_history_managers, &root_id, path)
            .await;
    }
}

/// Execute a tool call -- delegates to the tool registry.
///
/// Special handling for `ToolSearch` and `task`: these meta-tools are
/// intercepted and routed to their respective orchestrators which have
/// access to the full `ServiceContainer`.
async fn execute_tool_call(
    container: &ServiceContainer,
    config: &AgentExecutionConfig,
    tc: &ToolCallRequest,
    session_id: &SessionId,
    working_dir: Option<&str>,
    additional_read_dirs: &[String],
    progress: Option<&TurnEventSender>,
    cancel: Option<&CancellationToken>,
) -> Result<y_core::tool::ToolOutput, y_core::tool::ToolError> {
    // Intercept ToolSearch calls -- unified search across tools, skills, and agents.
    if tc.name == "ToolSearch" {
        let workflows = crate::workflow_service::WorkflowService::list(&container.workflow_store)
            .await
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "failed to load workflows for capability search");
                Vec::new()
            })
            .into_iter()
            .map(
                |workflow| crate::tool_search_orchestrator::WorkflowSearchItem {
                    id: workflow.id,
                    name: workflow.name,
                    description: workflow.description,
                    tags: serde_json::from_str(&workflow.tags).unwrap_or_default(),
                },
            )
            .collect::<Vec<_>>();
        let taxonomy = container.tool_taxonomy.read().await;
        let sources = crate::tool_search_orchestrator::CapabilitySearchSources {
            skill_search: Some(&container.skill_search),
            agent_registry: Some(&*container.agent_registry),
            workflows: Some(&workflows),
        };
        let result = crate::tool_search_orchestrator::ToolSearchOrchestrator::handle_with_sources(
            &tc.arguments,
            &container.tool_registry,
            &taxonomy,
            &container.tool_activation_set,
            &sources,
        )
        .await;

        return result;
    }

    if matches!(
        tc.name.as_str(),
        "AgentCreate"
            | "AgentUpdate"
            | "AgentDeactivate"
            | "AgentSearch"
            | "AgentEvaluate"
            | "AgentProposalList"
            | "AgentProposalRefine"
            | "AgentProposalDecide"
    ) {
        return super::dynamic_agent_tools::handle(container, config, tc, session_id).await;
    }

    if matches!(
        tc.name.as_str(),
        "SkillProposalList" | "SkillProposalRefine" | "SkillProposalDecide"
    ) {
        return super::skill_evolution_tools::handle(container, tc, session_id).await;
    }

    if matches!(
        tc.name.as_str(),
        "ToolCreate" | "ToolUpdate" | "ToolDelete" | "ToolGet" | "ToolList"
    ) {
        return super::dynamic_tool_tools::handle(container, config, tc).await;
    }

    // Intercept task calls -- delegate to a sub-agent via AgentDelegator.
    if tc.name == "Task" {
        // skill-creator is a side-effecting, structured-output agent. Route it
        // through SkillCreationService so the generated skill is registered in
        // the on-disk store (the same store the GUI panel and search index read
        // from) and a concise summary -- not the agent's raw JSON -- is returned
        // to the conversation.
        if tc.arguments.get("agent_name").and_then(|v| v.as_str()) == Some("skill-creator") {
            let skills_dir = container.skills_dir.clone().ok_or_else(|| {
                y_core::tool::ToolError::RuntimeError {
                    name: "skill-creator".into(),
                    message: "skills directory is not configured".into(),
                }
            })?;
            let output = run_skill_creation(
                Arc::clone(&container.agent_delegator),
                &skills_dir,
                &tc.arguments,
            )
            .await?;
            // Make the newly created skill discoverable via ToolSearch in this
            // session; the GUI panel reads the store from disk and needs no
            // refresh.
            if output.success {
                container.refresh_skill_search().await;
            }
            return Ok(output);
        }

        let session_uuid =
            uuid::Uuid::parse_str(session_id.as_str()).unwrap_or_else(|_| uuid::Uuid::new_v4());

        // Run the delegation under the parent turn's interaction context so the
        // sub-agent executes against this session: its tool permissions follow
        // the active session's mode (incl. HITL), and progress/cancel are wired
        // to the parent turn. Read by `ServiceAgentRunner` across the delegator
        // boundary. See `delegation_ctx`.
        let interaction_ctx = super::delegation_ctx::DelegationInteractionCtx {
            session_id: session_id.clone(),
            progress: progress.cloned(),
            cancel: cancel.cloned(),
        };
        return super::delegation_ctx::DELEGATION_INTERACTION_CTX
            .scope(
                interaction_ctx,
                crate::task_delegation_orchestrator::TaskDelegationOrchestrator::handle(
                    &tc.arguments,
                    container.agent_delegator.as_ref(),
                    &container.agent_registry,
                    Some(session_uuid),
                ),
            )
            .await;
    }

    // Intercept Plan tool -- route through PlanOrchestrator.
    // Box::pin breaks the recursive async cycle:
    // execute_tool_call -> PlanOrchestrator::handle -> AgentService::execute
    // -> execute_inner -> tool_handling -> execute_and_record_tool -> execute_tool_call
    if tc.name == "Plan" {
        let session_id_clone = session_id.clone();
        return Box::pin(crate::plan_orchestrator::PlanOrchestrator::handle(
            &tc.arguments,
            container,
            &session_id_clone,
            progress,
            cancel.cloned(),
        ))
        .await;
    }

    // Intercept Loop tool -- route through LoopOrchestrator.
    if tc.name == "Loop" {
        let session_id_clone = session_id.clone();
        return Box::pin(crate::loop_orchestrator::LoopOrchestrator::handle(
            &tc.arguments,
            container,
            &session_id_clone,
            progress,
            cancel.cloned(),
        ))
        .await;
    }

    // Intercept workflow/schedule meta-tools -- route through orchestrator.
    {
        use crate::workflow_orchestrator::WorkflowOrchestrator as WO;
        let args = &tc.arguments;
        match tc.name.as_str() {
            "WorkflowCreate" => return WO::handle_create(args, container).await,
            "WorkflowList" => return WO::handle_list(args, container).await,
            "WorkflowGet" => return WO::handle_get(args, container).await,
            "WorkflowUpdate" => return WO::handle_update(args, container).await,
            "WorkflowDelete" => return WO::handle_delete(args, container).await,
            "WorkflowValidate" => return WO::handle_validate(args, container),
            "WorkflowRun" => {
                let id = args
                    .get("id")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| y_core::tool::ToolError::ValidationError {
                        message: "'id' is required".to_string(),
                    })?;
                let parameters = args
                    .get("parameters")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({}));
                let execution =
                    crate::workflow_run_service::WorkflowRunService::run(container, id, parameters)
                        .await
                        .map_err(|error| y_core::tool::ToolError::RuntimeError {
                            name: "WorkflowRun".to_string(),
                            message: error.to_string(),
                        })?;
                return Ok(y_core::tool::ToolOutput {
                    success: true,
                    content: serde_json::to_value(&execution).unwrap_or_default(),
                    warnings: vec![],
                    metadata: serde_json::json!({ "action": "WorkflowRun" }),
                });
            }
            "ScheduleCreate" => return WO::handle_schedule_create(args, container).await,
            "ScheduleList" => return WO::handle_schedule_list(args, container).await,
            "SchedulePause" => return WO::handle_schedule_pause(args, container).await,
            "ScheduleResume" => return WO::handle_schedule_resume(args, container).await,
            "ScheduleDelete" => return WO::handle_schedule_delete(args, container).await,
            _ => {} // fall through to normal tool dispatch
        }
    }

    let tool_name = ToolName::from_string(&tc.name);
    let definition = container.tool_registry.get_definition(&tool_name).await;

    let tool = container
        .tool_registry
        .get_tool(&tool_name)
        .await
        .ok_or_else(|| y_core::tool::ToolError::NotFound {
            name: tc.name.clone(),
        })?;

    let input = ToolInput {
        call_id: tc.id.clone(),
        name: tool_name.clone(),
        arguments: tc.arguments.clone(),
        session_id: session_id.clone(),
        working_dir: working_dir.map(ToOwned::to_owned),
        additional_read_dirs: additional_read_dirs.to_vec(),
        command_runner: Some(Arc::clone(&container.runtime_manager) as Arc<dyn CommandRunner>),
    };

    let result = if let Some(tok) = cancel {
        tokio::select! {
            result = tool.execute(input) => result,
            () = tok.cancelled() => Err(y_core::tool::ToolError::Cancelled),
        }
    } else {
        tool.execute(input).await
    };
    if definition.is_some_and(|definition| definition.tool_type == y_core::tool::ToolType::Dynamic)
    {
        container
            .dynamic_tool_service
            .record_execution(&tool_name, &config.agent_name)
            .await;
    }
    result
}

/// Route a `Task(skill-creator)` call through the skill-creation service so the
/// generated skill is registered on disk, returning a concise summary to the
/// conversation instead of the agent's raw structured output.
async fn run_skill_creation(
    delegator: Arc<dyn y_core::agent::AgentDelegator>,
    skills_dir: &std::path::Path,
    arguments: &serde_json::Value,
) -> Result<y_core::tool::ToolOutput, y_core::tool::ToolError> {
    let request = arguments
        .get("prompt")
        .and_then(|v| v.as_str())
        .ok_or_else(|| y_core::tool::ToolError::ValidationError {
            message: "'prompt' is required".into(),
        })?;

    let outcome = crate::skill_creation::create_skill_from_request(
        delegator, skills_dir, request, None, None,
    )
    .await
    .map_err(|message| y_core::tool::ToolError::RuntimeError {
        name: "skill-creator".into(),
        message,
    })?;

    let created = outcome.decision == "created";
    let mut content = serde_json::Map::new();
    content.insert(
        "decision".into(),
        serde_json::Value::String(outcome.decision),
    );
    if let Some(skill_id) = outcome.skill_id {
        content.insert("skill_id".into(), serde_json::Value::String(skill_id));
    }
    if let Some(error) = outcome.error {
        content.insert("error".into(), serde_json::Value::String(error));
    }

    Ok(y_core::tool::ToolOutput {
        success: created,
        content: serde_json::Value::Object(content),
        warnings: vec![],
        metadata: serde_json::json!({ "action": "skill_create" }),
    })
}

/// Check if a successful `FileWrite` just created an agent TOML and, if
/// so, auto-register it so it takes effect immediately.
///
/// Detection heuristic: the `path` argument ends with `.toml` and contains
/// an `agents/` directory segment. Errors are logged but never propagated
/// (auto-registration is best-effort).
async fn maybe_auto_register_agent(container: &ServiceContainer, arguments: &serde_json::Value) {
    let path_str = arguments.get("path").and_then(|v| v.as_str()).unwrap_or("");

    if path_str.is_empty() {
        return;
    }

    let path = std::path::Path::new(path_str);

    // Only consider .toml files in an agents/ directory.
    let is_toml = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("toml"));
    let in_agents_dir = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .is_some_and(|name| name == "agents");

    if !is_toml || !in_agents_dir {
        return;
    }

    // Read the file from disk and attempt registration.
    match tokio::fs::read_to_string(path).await {
        Ok(content) => match container.register_agent_from_toml(&content).await {
            Ok(id) => {
                tracing::info!(
                    agent_id = %id,
                    path = %path_str,
                    "Auto-registered new agent definition from FileWrite"
                );
            }
            Err(e) => {
                tracing::warn!(
                    path = %path_str,
                    error = %e,
                    "Failed to auto-register agent from written file"
                );
            }
        },
        Err(e) => {
            tracing::warn!(
                path = %path_str,
                error = %e,
                "Failed to read agent file for auto-registration"
            );
        }
    }
}

/// Extract compact URL metadata from a Browser/WebFetch tool result.
///
/// Parses the full (unstripped) result content and returns a compact JSON
/// string containing only `url`, `title`, and `favicon_url`. Returns `None`
/// for non-URL tools or when parsing fails.
fn extract_url_meta(tool_name: &str, result_content: &str) -> Option<String> {
    if tool_name != "Browser" && tool_name != "WebFetch" {
        return None;
    }
    let parsed: serde_json::Value = serde_json::from_str(result_content).ok()?;
    let url = parsed.get("url").and_then(|v| v.as_str())?;
    if url.is_empty() {
        return None;
    }
    let title = parsed
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let favicon = parsed
        .get("favicon_url")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    Some(
        serde_json::json!({
            "url": url,
            "title": title,
            "favicon_url": favicon,
        })
        .to_string(),
    )
}

/// Strip Browser/WebFetch results to only LLM-relevant fields.
///
/// GUI-only fields (`favicon_url`, `navigation`) and echo fields (`action`,
/// `search_engine`, `query`) are removed. The LLM receives `text` (the page
/// content) plus `url`, `title` when present.
///
/// Non-URL tools pass through unchanged.
fn strip_url_tool_result(tool_name: &str, content: &serde_json::Value) -> String {
    if tool_name != "Browser" && tool_name != "WebFetch" {
        return serde_json::to_string(content).unwrap_or_default();
    }

    let Some(obj) = content.as_object() else {
        return serde_json::to_string(content).unwrap_or_default();
    };

    // Keep only LLM-relevant fields.
    let mut stripped = serde_json::Map::new();
    for key in ["url", "title", "text"] {
        if let Some(v) = obj.get(key) {
            stripped.insert(key.to_string(), v.clone());
        }
    }

    // If stripping removed everything meaningful, fall back to full content.
    if stripped.is_empty() {
        return serde_json::to_string(content).unwrap_or_default();
    }

    serde_json::to_string(&serde_json::Value::Object(stripped)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[derive(Debug)]
    struct AgentRefinerDelegator;

    #[async_trait::async_trait]
    impl y_core::agent::AgentDelegator for AgentRefinerDelegator {
        async fn delegate(
            &self,
            agent_name: &str,
            input: serde_json::Value,
            context_strategy: y_core::agent::ContextStrategyHint,
            _session_id: Option<Uuid>,
        ) -> Result<y_core::agent::DelegationOutput, y_core::agent::DelegationError> {
            assert_eq!(agent_name, "agent-refiner");
            assert_eq!(context_strategy, y_core::agent::ContextStrategyHint::None);
            assert_eq!(input["constraints"]["active_mutation_allowed"], false);
            Ok(y_core::agent::DelegationOutput {
                text: serde_json::json!({
                    "description": "Finds repository evidence and cites source files",
                    "mode": "explore",
                    "allowed_tools": ["FileRead"],
                    "system_prompt": "Inspect repository evidence and cite source files before concluding.",
                    "rationale": "Require grounded answers and reduce the inherited tool surface"
                })
                .to_string(),
                tokens_used: 50,
                input_tokens: 35,
                output_tokens: 15,
                model_used: "mock-refiner".to_string(),
                duration_ms: 5,
            })
        }
    }

    #[derive(Debug)]
    struct SkillRefinerDelegator;

    #[async_trait::async_trait]
    impl y_core::agent::AgentDelegator for SkillRefinerDelegator {
        async fn delegate(
            &self,
            agent_name: &str,
            input: serde_json::Value,
            context_strategy: y_core::agent::ContextStrategyHint,
            _session_id: Option<Uuid>,
        ) -> Result<y_core::agent::DelegationOutput, y_core::agent::DelegationError> {
            assert_eq!(agent_name, "skill-refiner");
            assert_eq!(context_strategy, y_core::agent::ContextStrategyHint::None);
            assert_eq!(input["constraints"]["active_mutation_allowed"], false);
            Ok(y_core::agent::DelegationOutput {
                text: serde_json::json!({
                    "root_content": "Review ownership, temporary lifetimes, and borrow extension before proposing edits.",
                    "rationale": "Address the repeated user-corrected lifetime failure."
                })
                .to_string(),
                tokens_used: 50,
                input_tokens: 35,
                output_tokens: 15,
                model_used: "mock-refiner".to_string(),
                duration_ms: 5,
            })
        }
    }

    fn test_execution_config(session_id: SessionId, tool_names: &[&str]) -> AgentExecutionConfig {
        AgentExecutionConfig {
            agent_name: "test-agent".to_string(),
            system_prompt: String::new(),
            max_iterations: 12,
            max_tool_calls: 20,
            tool_definitions: tool_names
                .iter()
                .map(|name| {
                    serde_json::json!({
                        "type": "function",
                        "function": { "name": name }
                    })
                })
                .collect(),
            tool_calling_mode: y_core::provider::ToolCallingMode::Native,
            tool_dialect: y_core::provider::ToolDialect::default(),
            messages: Vec::new(),
            provider_id: None,
            preferred_models: Vec::new(),
            provider_tags: Vec::new(),
            fallback_provider_tags: Vec::new(),
            request_mode: y_core::provider::RequestMode::TextChat,
            working_directory: None,
            additional_read_dirs: Vec::new(),
            temperature: None,
            max_tokens: Some(2_048),
            thinking: None,
            session_id: Some(session_id),
            session_uuid: Uuid::new_v4(),
            knowledge_collections: Vec::new(),
            use_context_pipeline: false,
            user_query: String::new(),
            external_trace_id: None,
            trust_tier: None,
            agent_allowed_tools: Vec::new(),
            prune_tool_history: false,
            response_format: None,
            image_generation_options: None,
            inherited_constraints: None,
            trace_metadata: serde_json::Value::Null,
        }
    }

    #[tokio::test]
    async fn dynamic_agent_tools_persist_activate_and_inherit_creator_limits() {
        let temp = tempfile::TempDir::new().unwrap();
        let mut service_config = crate::ServiceConfig::default();
        service_config.storage = y_storage::StorageConfig {
            db_path: ":memory:".to_string(),
            pool_size: 1,
            wal_enabled: false,
            transcript_dir: temp.path().join("transcripts"),
            ..y_storage::StorageConfig::default()
        };
        let mut container = ServiceContainer::from_config(&service_config)
            .await
            .unwrap();
        let session_id = SessionId::new();
        let root_config = test_execution_config(
            session_id.clone(),
            &["AgentCreate", "AgentSearch", "AgentDeactivate", "FileRead"],
        );

        let create = ToolCallRequest {
            id: "create-parent".to_string(),
            name: "AgentCreate".to_string(),
            arguments: serde_json::json!({
                "name": "runtime-scout",
                "description": "Finds repository evidence",
                "allowed_tools": ["AgentCreate", "FileRead"]
            }),
        };
        let created = execute_tool_call(
            &container,
            &root_config,
            &create,
            &session_id,
            None,
            &[],
            None,
            None,
        )
        .await
        .unwrap();
        assert!(created.success);
        let parent_id = created.content["agent"]["id"].as_str().unwrap();
        assert_eq!(
            created.content["agent"]["effective_permissions"]["max_iterations"],
            12
        );
        assert_eq!(
            created.content["agent"]["effective_permissions"]["max_tokens"],
            2_048
        );
        assert!(container
            .agent_registry
            .lock()
            .await
            .get(parent_id)
            .is_some());

        let mut dynamic_config = test_execution_config(session_id.clone(), &[]);
        dynamic_config.agent_name = parent_id.to_string();
        dynamic_config.trust_tier = Some(TrustTier::Dynamic);
        dynamic_config.agent_allowed_tools =
            vec!["AgentCreate".to_string(), "FileRead".to_string()];
        let create_child = ToolCallRequest {
            id: "create-child".to_string(),
            name: "AgentCreate".to_string(),
            arguments: serde_json::json!({
                "name": "runtime-child-scout",
                "description": "Reads files selected by its parent",
                "allowed_tools": ["FileRead"]
            }),
        };
        let child = execute_tool_call(
            &container,
            &dynamic_config,
            &create_child,
            &session_id,
            None,
            &[],
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(child.content["agent"]["delegation_depth"], 1);

        execute_tool_call(
            &container,
            &root_config,
            &ToolCallRequest {
                id: "update-parent".to_string(),
                name: "AgentUpdate".to_string(),
                arguments: serde_json::json!({
                    "id": parent_id,
                    "description": "A deliberately regressed runtime definition"
                }),
            },
            &session_id,
            None,
            &[],
            None,
            None,
        )
        .await
        .unwrap();

        use y_diagnostics::TraceStore;
        let trace_store = container.diagnostics.store();
        for version in [1_u64, 2] {
            for sample in 0..5 {
                let mut trace = y_diagnostics::Trace::new(root_config.session_uuid, parent_id);
                trace.metadata = serde_json::json!({
                    "dynamic_agent": { "id": parent_id, "version": version }
                });
                if version == 1 || sample == 0 {
                    trace.complete();
                } else {
                    trace.fail();
                }
                trace_store.insert_trace(trace).await.unwrap();
            }
        }

        let evaluate = execute_tool_call(
            &container,
            &root_config,
            &ToolCallRequest {
                id: "evaluate-agents".to_string(),
                name: "AgentEvaluate".to_string(),
                arguments: serde_json::json!({}),
            },
            &session_id,
            None,
            &[],
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(evaluate.content["regression_count"], 1);
        assert_eq!(evaluate.content["proposal_count"], 1);
        let proposal_id = evaluate.content["proposals"][0]["id"].as_str().unwrap();

        let listed = execute_tool_call(
            &container,
            &root_config,
            &ToolCallRequest {
                id: "list-agent-proposals".to_string(),
                name: "AgentProposalList".to_string(),
                arguments: serde_json::json!({"agent_id": parent_id}),
            },
            &session_id,
            None,
            &[],
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(listed.content["count"], 1);

        container.agent_delegator = Arc::new(AgentRefinerDelegator);
        let refined = execute_tool_call(
            &container,
            &root_config,
            &ToolCallRequest {
                id: "refine-agent-proposal".to_string(),
                name: "AgentProposalRefine".to_string(),
                arguments: serde_json::json!({
                    "proposal_id": proposal_id,
                    "instructions": "Prefer the minimum sufficient tool set"
                }),
            },
            &session_id,
            None,
            &[],
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            refined.content["proposal"]["change"]["type"],
            "candidate_update"
        );
        assert_eq!(refined.content["active_agent_mutation_performed"], false);
        assert_eq!(
            container
                .dynamic_agent_service
                .get(parent_id)
                .unwrap()
                .version,
            2
        );

        let decided = execute_tool_call(
            &container,
            &root_config,
            &ToolCallRequest {
                id: "approve-agent-proposal".to_string(),
                name: "AgentProposalDecide".to_string(),
                arguments: serde_json::json!({
                    "proposal_id": proposal_id,
                    "decision": "approve",
                    "reason": "Repeated execution evidence shows a regression"
                }),
            },
            &session_id,
            None,
            &[],
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(decided.content["status"], "applied");
        assert_eq!(decided.content["proposal"]["applied_version"], 3);
        assert_eq!(
            container
                .dynamic_agent_service
                .get(parent_id)
                .unwrap()
                .definition
                .allowed_tools,
            vec!["FileRead"]
        );

        let deactivate = ToolCallRequest {
            id: "deactivate-parent".to_string(),
            name: "AgentDeactivate".to_string(),
            arguments: serde_json::json!({
                "id": parent_id,
                "reason": "specialized child is active"
            }),
        };
        execute_tool_call(
            &container,
            &root_config,
            &deactivate,
            &session_id,
            None,
            &[],
            None,
            None,
        )
        .await
        .unwrap();
        assert!(container
            .agent_registry
            .lock()
            .await
            .get(parent_id)
            .is_none());
    }

    #[tokio::test]
    async fn skill_proposal_tools_refine_without_mutation_then_promote_on_approval() {
        use y_core::skill::{SkillManifest, SkillVersion};
        use y_core::types::{now, SkillId};
        use y_skills::experience::{
            EvidenceEntry, EvidenceProvenance, ExperienceOutcome, TokenUsage, ToolCallRecord,
        };
        use y_skills::FilesystemSkillStore;

        let temp = tempfile::TempDir::new().unwrap();
        let skills_dir = temp.path().join("skills");
        let timestamp = now();
        FilesystemSkillStore::new(&skills_dir)
            .unwrap()
            .save_skill(&SkillManifest {
                id: SkillId::from_string("skill-review-rust"),
                name: "review-rust".to_string(),
                description: "Reviews Rust ownership".to_string(),
                version: SkillVersion("v1".to_string()),
                tags: vec!["rust".to_string()],
                trigger_patterns: vec![],
                knowledge_bases: vec![],
                root_content: "Review ownership carefully.".to_string(),
                sub_documents: vec![],
                token_estimate: 10,
                created_at: timestamp,
                updated_at: timestamp,
                classification: None,
                constraints: None,
                security: None,
                references: None,
                author: None,
                source_format: None,
                source_hash: None,
                state: None,
                root_path: None,
            })
            .unwrap();
        let mut service_config = crate::ServiceConfig::default();
        service_config.skills_dir = Some(skills_dir.clone());
        service_config.storage = y_storage::StorageConfig {
            db_path: ":memory:".to_string(),
            pool_size: 1,
            wal_enabled: false,
            transcript_dir: temp.path().join("transcripts"),
            ..y_storage::StorageConfig::default()
        };
        let mut container = ServiceContainer::from_config(&service_config)
            .await
            .unwrap();
        for _ in 0..3 {
            container
                .skill_evolution_service
                .record_turn(crate::skill_evolution_service::TurnExperienceInput {
                    skills: vec!["review-rust".to_string()],
                    task_description: "Review the ownership module".to_string(),
                    outcome: ExperienceOutcome::Failure,
                    trajectory_summary: "Compilation failed after the review edit".to_string(),
                    key_decisions: vec!["Changed the borrow strategy".to_string()],
                    evidence: vec![EvidenceEntry {
                        content: "Do not extend the temporary borrow".to_string(),
                        provenance: EvidenceProvenance::UserCorrection,
                    }],
                    tool_calls: vec![ToolCallRecord {
                        name: "ShellExec".to_string(),
                        success: false,
                        duration_ms: 25,
                    }],
                    error_messages: vec!["borrowed value does not live long enough".to_string()],
                    duration_ms: 100,
                    token_usage: TokenUsage::new(100, 50),
                })
                .await
                .unwrap();
        }
        let session_id = SessionId::new();
        let config = test_execution_config(
            session_id.clone(),
            &[
                "SkillProposalList",
                "SkillProposalRefine",
                "SkillProposalDecide",
            ],
        );

        let listed = execute_tool_call(
            &container,
            &config,
            &ToolCallRequest {
                id: "list-skill-proposals".to_string(),
                name: "SkillProposalList".to_string(),
                arguments: serde_json::json!({"skill_name": "review-rust"}),
            },
            &session_id,
            None,
            &[],
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(listed.content["count"], 1);
        let proposal_id = listed.content["proposals"][0]["id"]
            .as_str()
            .unwrap()
            .to_string();

        container.agent_delegator = Arc::new(SkillRefinerDelegator);
        let refined = execute_tool_call(
            &container,
            &config,
            &ToolCallRequest {
                id: "refine-skill-proposal".to_string(),
                name: "SkillProposalRefine".to_string(),
                arguments: serde_json::json!({"proposal_id": proposal_id}),
            },
            &session_id,
            None,
            &[],
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(refined.content["active_skill_mutation_performed"], false);
        assert_eq!(
            FilesystemSkillStore::new(&skills_dir)
                .unwrap()
                .load_skill("review-rust")
                .unwrap()
                .root_content,
            "Review ownership carefully."
        );

        let decided = execute_tool_call(
            &container,
            &config,
            &ToolCallRequest {
                id: "approve-skill-proposal".to_string(),
                name: "SkillProposalDecide".to_string(),
                arguments: serde_json::json!({
                    "proposal_id": proposal_id,
                    "decision": "approve",
                    "reason": "Repeated user-corrected failures"
                }),
            },
            &session_id,
            None,
            &[],
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(decided.content["active_skill_mutation_performed"], true);
        assert_eq!(decided.content["proposal"]["status"], "promoted");
        assert!(FilesystemSkillStore::new(&skills_dir)
            .unwrap()
            .load_skill("review-rust")
            .unwrap()
            .root_content
            .contains("temporary lifetimes"));
    }

    #[tokio::test]
    async fn dynamic_tool_lifecycle_dispatch_is_config_gated_and_registry_synchronized() {
        let temp = tempfile::TempDir::new().unwrap();
        let mut disabled_config = crate::ServiceConfig::default();
        disabled_config.storage = y_storage::StorageConfig {
            db_path: ":memory:".to_string(),
            pool_size: 1,
            wal_enabled: false,
            transcript_dir: temp.path().join("disabled-transcripts"),
            ..y_storage::StorageConfig::default()
        };
        let disabled = ServiceContainer::from_config(&disabled_config)
            .await
            .unwrap();
        assert!(!disabled
            .tool_registry
            .get_all_definitions()
            .await
            .iter()
            .any(|definition| definition.name.as_str() == "ToolCreate"));

        let session_id = SessionId::new();
        let execution_config = test_execution_config(
            session_id.clone(),
            &[
                "ToolCreate",
                "ToolUpdate",
                "ToolDelete",
                "ToolGet",
                "ToolList",
            ],
        );
        let create_call = ToolCallRequest {
            id: "create-dynamic-tool".to_string(),
            name: "ToolCreate".to_string(),
            arguments: serde_json::json!({
                "name": "RuntimeEcho",
                "description": "Echo structured input",
                "parameters": {"type": "object"},
                "interpreter": "bash",
                "source": "read input; printf '%s' \"$input\""
            }),
        };
        assert!(execute_tool_call(
            &disabled,
            &execution_config,
            &create_call,
            &session_id,
            None,
            &[],
            None,
            None,
        )
        .await
        .is_err());

        let enabled_temp = tempfile::TempDir::new().unwrap();
        let mut enabled_config = crate::ServiceConfig::default();
        enabled_config.tools.allow_dynamic_tools = true;
        enabled_config.storage = y_storage::StorageConfig {
            db_path: ":memory:".to_string(),
            pool_size: 1,
            wal_enabled: false,
            transcript_dir: enabled_temp.path().join("transcripts"),
            ..y_storage::StorageConfig::default()
        };
        let enabled = ServiceContainer::from_config(&enabled_config)
            .await
            .unwrap();
        assert!(enabled
            .tool_registry
            .get_all_definitions()
            .await
            .iter()
            .any(|definition| definition.name.as_str() == "ToolCreate"));

        let created = execute_tool_call(
            &enabled,
            &execution_config,
            &create_call,
            &session_id,
            None,
            &[],
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(created.content["tool"]["version"], 1);
        assert!(enabled
            .tool_registry
            .get_tool(&ToolName::from_string("RuntimeEcho"))
            .await
            .is_some());

        let updated = execute_tool_call(
            &enabled,
            &execution_config,
            &ToolCallRequest {
                id: "update-dynamic-tool".to_string(),
                name: "ToolUpdate".to_string(),
                arguments: serde_json::json!({
                    "name": "RuntimeEcho",
                    "source": "printf 'v2'"
                }),
            },
            &session_id,
            None,
            &[],
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(updated.content["tool"]["version"], 2);

        let listed = execute_tool_call(
            &enabled,
            &execution_config,
            &ToolCallRequest {
                id: "list-dynamic-tools".to_string(),
                name: "ToolList".to_string(),
                arguments: serde_json::json!({}),
            },
            &session_id,
            None,
            &[],
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(listed.content["count"], 1);

        execute_tool_call(
            &enabled,
            &execution_config,
            &ToolCallRequest {
                id: "delete-dynamic-tool".to_string(),
                name: "ToolDelete".to_string(),
                arguments: serde_json::json!({
                    "name": "RuntimeEcho",
                    "reason": "Test lifecycle cleanup"
                }),
            },
            &session_id,
            None,
            &[],
            None,
            None,
        )
        .await
        .unwrap();
        assert!(enabled
            .tool_registry
            .get_tool(&ToolName::from_string("RuntimeEcho"))
            .await
            .is_none());
    }

    #[test]
    fn test_strip_url_tool_result_removes_navigation_and_favicon() {
        let content = serde_json::json!({
            "action": "navigate",
            "url": "https://example.com",
            "title": "Example",
            "favicon_url": "data:image/png;base64,abc",
            "navigation": {
                "frame_id": "frame-1"
            }
        });

        let stripped = strip_url_tool_result("Browser", &content);
        let stripped_json: serde_json::Value = serde_json::from_str(&stripped).unwrap();

        assert_eq!(stripped_json["url"], content["url"]);
        assert_eq!(stripped_json["title"], content["title"]);
        assert!(stripped_json.get("favicon_url").is_none());
        assert!(stripped_json.get("navigation").is_none());
    }

    #[test]
    fn test_strip_url_tool_result_keeps_search_text() {
        let content = serde_json::json!({
            "action": "search",
            "query": "IGS Speed Driver 街机游戏 全系列",
            "search_engine": "google",
            "url": "https://www.google.com/search?q=IGS+Speed+Driver",
            "title": "IGS Speed Driver - Google 搜索",
            "text": "搜索结果页文本"
        });

        let stripped = strip_url_tool_result("WebFetch", &content);
        let stripped_json: serde_json::Value = serde_json::from_str(&stripped).unwrap();

        assert_eq!(stripped_json["url"], content["url"]);
        assert_eq!(stripped_json["title"], content["title"]);
        assert_eq!(stripped_json["text"], content["text"]);
        assert!(stripped_json.get("results").is_none());
    }

    #[test]
    fn test_permission_prompt_preview_prefers_command_path_then_url() {
        assert_eq!(
            permission_prompt_content_preview(&serde_json::json!({
                "command": "cargo test",
                "path": "src/lib.rs",
                "url": "https://example.com"
            })),
            Some("cargo test".to_string())
        );
        assert_eq!(
            permission_prompt_content_preview(&serde_json::json!({
                "path": "src/lib.rs",
                "url": "https://example.com"
            })),
            Some("src/lib.rs".to_string())
        );
        assert_eq!(
            permission_prompt_content_preview(&serde_json::json!({
                "url": "https://example.com"
            })),
            Some("https://example.com".to_string())
        );
    }

    #[tokio::test(start_paused = true)]
    async fn test_intercept_ask_user_times_out_and_cleans_pending_entry() {
        let tc = ToolCallRequest {
            id: "call-1".to_string(),
            name: "AskUser".to_string(),
            arguments: serde_json::json!({
                "questions": [
                    {
                        "question": "Choose a direction?",
                        "options": ["Fast", "Careful"]
                    }
                ]
            }),
        };
        let pending_interactions =
            std::sync::Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let pending_permissions =
            std::sync::Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let session_id = SessionId("session-timeout".to_string());
        let mut ctx = ToolExecContext {
            iteration: 0,
            last_gen_id: None,
            tool_calls_executed: vec![ToolCallRecord {
                name: "AskUser".to_string(),
                arguments: "{}".to_string(),
                success: true,
                duration_ms: 0,
                result_content: String::new(),
                url_meta: None,
                metadata: None,
            }],
            new_messages: Vec::new(),
            cumulative_input_tokens: 0,
            cumulative_output_tokens: 0,
            cumulative_cost: 0.0,
            last_input_tokens: 0,
            last_cache_read_tokens: 0,
            last_cache_write_tokens: 0,
            trace_id: None,
            session_id: session_id.clone(),
            working_directory: None,
            additional_read_dirs: Vec::new(),
            working_history: Vec::new(),
            accumulated_content: String::new(),
            iteration_texts: Vec::new(),
            iteration_reasonings: Vec::new(),
            iteration_reasoning_durations_ms: Vec::new(),
            iteration_tool_counts: Vec::new(),
            dynamic_tool_defs: Vec::new(),
            pending_interactions: pending_interactions.clone(),
            pending_permissions,
            cancel_token: None,
            injected_steers: Vec::new(),
        };
        let mut config = test_execution_config(session_id, &[]);
        config.max_iterations = 1;
        config.max_tool_calls = 1;
        let (tx, _rx) = crate::chat::TurnEventSender::channel();

        let answer = tokio::time::timeout(
            std::time::Duration::from_secs(181),
            intercept_ask_user(&tc, Some(&tx), &mut ctx, &config, std::time::Instant::now()),
        )
        .await
        .expect("AskUser should resolve through its internal timeout");

        assert!(answer.unwrap().contains("timed out"));
        assert!(pending_interactions.lock().await.is_empty());
    }

    #[test]
    fn test_tool_error_content_is_json_object_for_llm_retry() {
        let error = y_core::tool::ToolError::NotFound {
            name: "NotARealTool".to_string(),
        };

        let content = tool_error_content(&error);

        assert!(content.is_object());
        assert_eq!(
            content.get("error").and_then(serde_json::Value::as_str),
            Some("tool not found: NotARealTool")
        );
        assert_eq!(
            content
                .get("retryable")
                .and_then(serde_json::Value::as_bool),
            Some(false)
        );
    }

    #[test]
    fn test_normalize_failed_tool_string_wraps_error_object() {
        let content = normalize_tool_output_content(
            false,
            serde_json::Value::String("permission denied".to_string()),
        );

        assert_eq!(content, serde_json::json!({ "error": "permission denied" }));
    }

    #[test]
    fn test_system_tool_error_content_is_json_object() {
        let content = system_tool_error_content("Tool call limit reached.", false);
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();

        assert_eq!(
            parsed.get("error").and_then(serde_json::Value::as_str),
            Some("Tool call limit reached.")
        );
        assert_eq!(
            parsed.get("retryable").and_then(serde_json::Value::as_bool),
            Some(false)
        );
    }

    // -- run_skill_creation -------------------------------------------------

    #[derive(Debug)]
    struct SkillCreatorDelegator {
        response: String,
        root_md: Option<String>,
    }

    #[async_trait::async_trait]
    impl y_core::agent::AgentDelegator for SkillCreatorDelegator {
        async fn delegate(
            &self,
            _agent_name: &str,
            input: serde_json::Value,
            _context_strategy: y_core::agent::ContextStrategyHint,
            _session_id: Option<Uuid>,
        ) -> Result<y_core::agent::DelegationOutput, y_core::agent::DelegationError> {
            // Mirror the real agent: write root.md into the provided output dir.
            if let Some(root_md) = &self.root_md {
                let input_str = input.as_str().unwrap_or_default();
                for line in input_str.lines() {
                    if let Some(rest) = line.strip_prefix("- **Output directory**: `") {
                        if let Some(dir) = rest.strip_suffix('`') {
                            std::fs::write(std::path::Path::new(dir).join("root.md"), root_md)
                                .unwrap();
                        }
                    }
                }
            }
            Ok(y_core::agent::DelegationOutput {
                text: self.response.clone(),
                tokens_used: 10,
                input_tokens: 8,
                output_tokens: 2,
                model_used: "mock".into(),
                duration_ms: 1,
            })
        }
    }

    fn created_agent_response() -> String {
        // Note the leading narration: the helper must not surface it.
        let json = serde_json::json!({
            "decision": "created",
            "manifest": {
                "name": "summarize-academic",
                "version": "1.0.0",
                "description": "Summarize academic papers",
                "author": "skill-creator-agent",
                "classification": {
                    "type": "llm_reasoning",
                    "domain": ["academic"],
                    "tags": ["summarize"],
                    "atomic": true
                },
                "constraints": {},
                "root": { "path": "root.md", "token_count": 50 },
                "references": { "tools": [], "skills": [], "knowledge_bases": [] }
            },
            "sub_documents": [],
            "extracted_tools": []
        })
        .to_string();
        format!("Now I have a clear understanding. Let me create the skill.\n\n{json}")
    }

    #[tokio::test]
    async fn test_run_skill_creation_returns_clean_summary() {
        let tmp = tempfile::TempDir::new().unwrap();
        let delegator = Arc::new(SkillCreatorDelegator {
            response: created_agent_response(),
            root_md: Some("# Summarize Academic\n\nSummarize papers.".into()),
        });
        let args = serde_json::json!({
            "agent_name": "skill-creator",
            "prompt": "Summarize academic papers",
        });

        let output = run_skill_creation(delegator, tmp.path(), &args)
            .await
            .unwrap();

        assert!(output.success);
        assert_eq!(output.content["decision"], "created");
        assert_eq!(output.content["skill_id"], "summarize-academic");
        // The conversation must not see the agent's raw structured output or
        // its narration -- only the concise summary fields.
        assert!(output.content.get("manifest").is_none());
        assert!(output.content.get("output").is_none());
        let serialized = serde_json::to_string(&output.content).unwrap();
        assert!(!serialized.contains("Now I have"));
        assert!(!serialized.contains("optimization_notes"));
        assert_eq!(output.metadata["action"], "skill_create");
    }

    #[tokio::test]
    async fn test_run_skill_creation_rejected_is_unsuccessful() {
        let tmp = tempfile::TempDir::new().unwrap();
        let response = serde_json::json!({
            "decision": "rejected",
            "rejection_reason": "This is a CLI wrapper, not an LLM reasoning task",
            "redirect_suggestion": "Tool System"
        })
        .to_string();
        let delegator = Arc::new(SkillCreatorDelegator {
            response,
            root_md: None,
        });
        let args = serde_json::json!({
            "agent_name": "skill-creator",
            "prompt": "Wrap the curl command",
        });

        let output = run_skill_creation(delegator, tmp.path(), &args)
            .await
            .unwrap();

        assert!(!output.success);
        assert_eq!(output.content["decision"], "rejected");
        assert!(output.content.get("skill_id").is_none());
        assert!(output.content["error"]
            .as_str()
            .unwrap()
            .contains("CLI wrapper"));
    }

    #[tokio::test]
    async fn test_run_skill_creation_missing_prompt() {
        let tmp = tempfile::TempDir::new().unwrap();
        let delegator = Arc::new(SkillCreatorDelegator {
            response: String::new(),
            root_md: None,
        });
        let args = serde_json::json!({ "agent_name": "skill-creator" });

        let result = run_skill_creation(delegator, tmp.path(), &args).await;

        assert!(matches!(
            result.unwrap_err(),
            y_core::tool::ToolError::ValidationError { .. }
        ));
    }
}
