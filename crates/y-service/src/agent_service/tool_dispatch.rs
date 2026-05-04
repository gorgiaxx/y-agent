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

    if ctx
        .cancel_token
        .as_ref()
        .is_some_and(tokio_util::sync::CancellationToken::is_cancelled)
    {
        let error_content = "[SYSTEM] Cancelled by user.".to_string();
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
        return (false, error_content);
    }

    if ctx.tool_calls_executed.len() >= config.max_tool_calls {
        let error_content = format!(
            "[SYSTEM] Tool call limit ({}) reached. Do NOT request more tools. \
             Finish with the information already available.",
            config.max_tool_calls
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
        return (false, error_content);
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

            return (false, error_content);
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
                Some(crate::chat::PermissionPromptResponse::Deny) | None => {
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
        tc,
        &ctx.session_id,
        ctx.working_directory.as_deref(),
        progress,
        ctx.cancel_token.as_ref(),
    )
    .await
    {
        Ok(output) => {
            let full = serde_json::to_string(&output.content).unwrap_or_default();
            // For Browser/WebFetch: strip GUI-only fields (favicon_url,
            // action, search_engine, navigation) before sending to the
            // LLM. Only keep text + url/title for context.
            let stripped = strip_url_tool_result(&tc.name, &output.content);
            // Global safety net: ensure no tool result exceeds 10K chars in
            // the LLM path. Per-tool truncation handles most cases, but this
            // catches MCP tools, meta-tools, or any tool that slips through.
            let stripped = y_prompt::budget::truncate_tool_result(
                &stripped,
                y_prompt::budget::MAX_TOOL_RESULT_CHARS,
            );
            let metadata = (!output.metadata.is_null()).then_some(output.metadata);
            (true, full, stripped, metadata)
        }
        Err(e) => {
            let msg = format!("{e}");
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

    // Emit ToolResult progress event.
    // Use the full (unstripped) result for url_meta extraction and GUI
    // preview, but the stripped version is what the LLM sees.
    // Limit must be large enough to keep structured JSON (e.g. Grep results)
    // intact -- matches the persisted metadata limit in build_tool_results_metadata.
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
            return (true, answer);
        }
    }

    // Auto-register agent files created via FileWrite.
    if tool_success && tc.name == "FileWrite" {
        maybe_auto_register_agent(container, &tc.arguments).await;
    }

    (tool_success, result_content)
}

fn tool_arguments_preview(tc: &ToolCallRequest) -> String {
    serde_json::to_string(&tc.arguments).unwrap_or_default()
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
/// Returns `Some(answer_content)` if the user answered, `None` if the
/// questions field is missing or the channel was dropped.
async fn intercept_ask_user(
    tc: &ToolCallRequest,
    progress: Option<&TurnEventSender>,
    ctx: &mut ToolExecContext,
    config: &AgentExecutionConfig,
    tool_start: std::time::Instant,
) -> Option<String> {
    let questions = tc.arguments.get("questions")?;

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
    let answer = answer_rx.await.ok()?;
    let answer_content = serde_json::to_string(&answer).unwrap_or_else(|_| answer.to_string());
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
        crate::rewind::RewindService::track_edit(
            &container.file_history_managers,
            session_id,
            path,
        )
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
    tc: &ToolCallRequest,
    session_id: &SessionId,
    working_dir: Option<&str>,
    progress: Option<&TurnEventSender>,
    cancel: Option<&CancellationToken>,
) -> Result<y_core::tool::ToolOutput, y_core::tool::ToolError> {
    // Intercept ToolSearch calls -- unified search across tools, skills, and agents.
    if tc.name == "ToolSearch" {
        let taxonomy = container.tool_taxonomy.read().await;
        let sources = crate::tool_search_orchestrator::CapabilitySearchSources {
            skill_search: Some(&container.skill_search),
            agent_registry: Some(&*container.agent_registry),
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
        working_dir: working_dir.map(ToOwned::to_owned),
        command_runner: Some(Arc::clone(&container.runtime_manager) as Arc<dyn CommandRunner>),
    };

    if let Some(tok) = cancel {
        tokio::select! {
            result = tool.execute(input) => result,
            () = tok.cancelled() => Err(y_core::tool::ToolError::Cancelled),
        }
    } else {
        tool.execute(input).await
    }
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
}
