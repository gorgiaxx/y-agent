//! Tool execution, permission gating, and HITL approval flow.

use std::sync::Arc;

use y_core::permission_types::PermissionMode;
use y_core::runtime::CommandRunner;
use y_core::tool::ToolInput;
use y_core::trust::TrustTier;
use y_core::types::{SessionId, ToolCallRequest, ToolName};

use crate::container::ServiceContainer;

use super::{AgentExecutionConfig, ToolCallRecord, ToolExecContext, TurnEvent, TurnEventSender};

/// Execute a single tool call, record it, and emit progress events.
///
/// Returns `(success, result_content)`.
pub(crate) async fn execute_and_record_tool(
    container: &ServiceContainer,
    config: &AgentExecutionConfig,
    tc: &ToolCallRequest,
    progress: Option<&TurnEventSender>,
    ctx: &mut ToolExecContext,
) -> (bool, String) {
    let tool_start = std::time::Instant::now();

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
        resolve_permission_decision_for_session(
            permission_model.evaluate(&tc.name, is_dangerous),
            session_mode,
        )
    };

    match decision.action {
        y_guardrails::PermissionAction::Deny => {
            // Denied by policy -- do NOT execute the tool.
            tracing::warn!(
                tool = %tc.name,
                reason = %decision.reason,
                "tool execution denied by permission policy"
            );
            let error_content = format!(
                "[SYSTEM] Tool '{}' is blocked by security policy ({}). \
                 Do NOT ask the user for permission or retry this tool. \
                 Use an alternative approach or skip this action.",
                tc.name, decision.reason
            );

            ctx.tool_calls_executed.push(ToolCallRecord {
                name: tc.name.clone(),
                arguments: serde_json::to_string(&tc.arguments).unwrap_or_default(),
                success: false,
                duration_ms: 0,
                result_content: error_content.clone(),
            });

            if let Some(tx) = progress {
                let _ = tx.send(TurnEvent::ToolResult {
                    name: tc.name.clone(),
                    success: false,
                    duration_ms: 0,
                    input_preview: serde_json::to_string(&tc.arguments).unwrap_or_default(),
                    result_preview: error_content.clone(),
                    agent_name: config.agent_name.clone(),
                });
            }

            return (false, error_content);
        }
        y_guardrails::PermissionAction::Ask => {
            // Pause and ask the user for approval via HITL.
            let request_id = uuid::Uuid::new_v4().to_string();

            // Extract content preview (command for ShellExec, path for
            // file tools, etc.) for the permission prompt.
            let content_preview = tc
                .arguments
                .get("command")
                .or_else(|| tc.arguments.get("path"))
                .or_else(|| tc.arguments.get("url"))
                .and_then(|v| v.as_str())
                .map(String::from);

            let action_desc = if let Some(ref preview) = content_preview {
                format!("{} wants to execute: {}", tc.name, preview)
            } else {
                format!("{} wants to execute", tc.name)
            };

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
                map.insert(request_id.clone(), resp_tx);
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

            // Block until the user responds (or the channel is dropped).
            match resp_rx.await {
                Ok(crate::chat::PermissionPromptResponse::Approve) => {
                    tracing::info!(
                        tool = %tc.name,
                        request_id = %request_id,
                        "user approved tool execution"
                    );
                    // Fall through to execute the tool.
                }
                Ok(crate::chat::PermissionPromptResponse::AllowAllForSession) => {
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
                Ok(crate::chat::PermissionPromptResponse::Deny) | Err(_) => {
                    let denied_msg = "[SYSTEM] User denied permission for this tool call.";
                    tracing::info!(
                        tool = %tc.name,
                        request_id = %request_id,
                        "user denied tool execution"
                    );
                    let error_content = format!(
                        "{denied_msg} \
                         Do NOT retry this tool. Use an alternative approach."
                    );

                    ctx.tool_calls_executed.push(ToolCallRecord {
                        name: tc.name.clone(),
                        arguments: serde_json::to_string(&tc.arguments).unwrap_or_default(),
                        success: false,
                        duration_ms: 0,
                        result_content: error_content.clone(),
                    });

                    if let Some(tx) = progress {
                        let _ = tx.send(TurnEvent::ToolResult {
                            name: tc.name.clone(),
                            success: false,
                            duration_ms: 0,
                            input_preview: serde_json::to_string(&tc.arguments).unwrap_or_default(),
                            result_preview: error_content.clone(),
                            agent_name: config.agent_name.clone(),
                        });
                    }

                    return (false, error_content);
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

    // ---------------------------------------------------------------
    // Actual tool execution
    // ---------------------------------------------------------------
    let (tool_success, result_content) =
        match execute_tool_call(container, tc, &ctx.session_id).await {
            Ok(output) => {
                let content = serde_json::to_string(&output.content).unwrap_or_default();
                (true, content)
            }
            Err(e) => (false, format!("{e}")),
        };

    let tool_elapsed_ms = u64::try_from(tool_start.elapsed().as_millis()).unwrap_or(0);

    ctx.tool_calls_executed.push(ToolCallRecord {
        name: tc.name.clone(),
        arguments: serde_json::to_string(&tc.arguments).unwrap_or_default(),
        success: tool_success,
        duration_ms: tool_elapsed_ms,
        result_content: result_content.clone(),
    });

    // Emit ToolResult progress event.
    if let Some(tx) = progress {
        let preview_len = result_content.floor_char_boundary(500);
        let _ = tx.send(TurnEvent::ToolResult {
            name: tc.name.clone(),
            success: tool_success,
            duration_ms: tool_elapsed_ms,
            input_preview: serde_json::to_string(&tc.arguments).unwrap_or_default(),
            result_preview: result_content[..preview_len].to_string(),
            agent_name: config.agent_name.clone(),
        });
    }

    // AskUser interception: if the tool is AskUser, block until the
    // presentation layer delivers an answer via `PendingInteractions`.
    if tool_success && tc.name == "AskUser" {
        if let Some(questions) = tc.arguments.get("questions") {
            let interaction_id = uuid::Uuid::new_v4().to_string();
            let (answer_tx, answer_rx) = tokio::sync::oneshot::channel::<serde_json::Value>();
            {
                let mut map = ctx.pending_interactions.lock().await;
                map.insert(interaction_id.clone(), answer_tx);
            }

            if let Some(tx) = progress {
                let _ = tx.send(TurnEvent::UserInteractionRequest {
                    interaction_id: interaction_id.clone(),
                    questions: questions.clone(),
                });
            }

            // Block this iteration until the user answers.
            if let Ok(answer) = answer_rx.await {
                let answer_content =
                    serde_json::to_string(&answer).unwrap_or_else(|_| answer.to_string());
                return (true, answer_content);
            }
        }
    }

    // Auto-register agent files created via FileWrite.
    if tool_success && tc.name == "FileWrite" {
        maybe_auto_register_agent(container, &tc.arguments).await;
    }

    (tool_success, result_content)
}

/// Resolve permission decision applying session-level overrides.
pub(crate) fn resolve_permission_decision_for_session(
    decision: y_guardrails::PermissionDecision,
    session_mode: Option<PermissionMode>,
) -> y_guardrails::PermissionDecision {
    match session_mode {
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

pub(crate) async fn session_permission_mode(
    container: &ServiceContainer,
    session_id: &SessionId,
) -> Option<PermissionMode> {
    let modes = container.session_permission_modes.read().await;
    modes.get(session_id).copied()
}

pub(crate) async fn set_session_permission_mode(
    container: &ServiceContainer,
    session_id: &SessionId,
    mode: PermissionMode,
) {
    let mut modes = container.session_permission_modes.write().await;
    modes.insert(session_id.clone(), mode);
}

/// Execute a tool call -- delegates to the tool registry.
///
/// Special handling for `ToolSearch` and `task`: these meta-tools are
/// intercepted and routed to their respective orchestrators which have
/// access to the full `ServiceContainer`.
async fn execute_tool_call(
    container: &ServiceContainer,
    tc: &ToolCallRequest,
    session_id: &SessionId,
) -> Result<y_core::tool::ToolOutput, y_core::tool::ToolError> {
    // Intercept ToolSearch calls -- unified search across tools, skills, and agents.
    if tc.name == "ToolSearch" {
        let sources = crate::tool_search_orchestrator::CapabilitySearchSources {
            skill_search: Some(&container.skill_search),
            agent_registry: Some(&*container.agent_registry),
        };
        let result = crate::tool_search_orchestrator::ToolSearchOrchestrator::handle_with_sources(
            &tc.arguments,
            &container.tool_registry,
            &container.tool_taxonomy,
            &container.tool_activation_set,
            &sources,
        )
        .await;

        return result;
    }

    // Intercept task calls -- delegate to a sub-agent via AgentDelegator.
    if tc.name == "Task" {
        let session_uuid =
            uuid::Uuid::parse_str(session_id.as_str()).unwrap_or_else(|_| uuid::Uuid::new_v4());
        return crate::task_delegation_orchestrator::TaskDelegationOrchestrator::handle(
            &tc.arguments,
            container.agent_delegator.as_ref(),
            Some(session_uuid),
        )
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
            "ScheduleCreate" => return WO::handle_schedule_create(args, container).await,
            "ScheduleList" => return WO::handle_schedule_list(args, container).await,
            "SchedulePause" => return WO::handle_schedule_pause(args, container).await,
            "ScheduleResume" => return WO::handle_schedule_resume(args, container).await,
            "ScheduleDelete" => return WO::handle_schedule_delete(args, container).await,
            _ => {} // fall through to normal tool dispatch
        }
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
    match std::fs::read_to_string(path) {
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
