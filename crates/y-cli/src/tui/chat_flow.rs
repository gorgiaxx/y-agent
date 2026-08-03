use std::sync::Arc;

use chrono::Utc;
use tokio::sync::mpsc;
use tracing::warn;

use y_core::session::{CreateSessionOptions, SessionType};
use y_core::types::SessionId;
use y_service::{PrepareTurnRequest, PreparedTurn, ResendTurnRequest};

use crate::orchestrator::{self, ChatService, TurnCancellationToken, TurnError};
use crate::tui::state::{
    AppState, ChatMessage, MessageRole, SessionListItem, StreamSegment, ToolCallDisplayMode,
    ToolCallInfo, ToolCallStatus, TurnMode,
};
use crate::wire::AppServices;

/// Events sent from the async LLM task back to the TUI event loop.
#[derive(Debug)]
pub enum ChatEvent {
    /// LLM response completed -- full content.
    Response {
        content: String,
        model: String,
        input_tokens: u64,
        output_tokens: u64,
        /// Input tokens from the last LLM iteration (actual context occupancy).
        last_input_tokens: u64,
        /// Context window size of the provider that served this request.
        context_window: usize,
        /// Cost in USD for this turn.
        cost_usd: f64,
    },
    /// A tool call started during the LLM turn.
    ToolCallStarted {
        tool_call_id: String,
        name: String,
        input_preview: String,
        agent_name: String,
    },
    /// A tool call completed during the LLM turn.
    ToolCallCompleted {
        tool_call_id: String,
        name: String,
        success: bool,
        duration_ms: u64,
        input_preview: String,
        result_preview: String,
        agent_name: String,
        url_meta: Option<String>,
        metadata: Option<serde_json::Value>,
    },
    /// Authoritative completed-turn tool list used to repair dropped progress events.
    ToolCallsSnapshot {
        calls: Vec<y_service::ToolCallRecord>,
    },
    /// Incremental text delta from the LLM stream.
    StreamDelta { content: String },
    /// Incremental reasoning/thinking delta from a thinking-mode LLM.
    StreamReasoningDelta { content: String },
    /// LLM request failed.
    Error(String),
    /// Session title was updated by the background summarizer.
    TitleUpdated { session_id: String, title: String },
    /// A new session was lazily created on first message.
    SessionCreated {
        id: String,
        title: String,
        updated_at: chrono::DateTime<Utc>,
    },
    /// A queued follow-up was injected into the active service run.
    FollowUpInjected { follow_up_id: String, text: String },
    /// A queued steer was injected into the active service run at an
    /// LLM-call boundary.
    SteerInjected { steer_id: String, text: String },
    /// A tool call is waiting for structured user input.
    AskUserRequested {
        interaction_id: String,
        questions: serde_json::Value,
    },
    /// A dangerous tool call escalated to the HITL permission gate and is
    /// waiting for an allow/deny answer.
    PermissionRequested {
        request_id: String,
        tool_name: String,
        action_description: String,
        reason: String,
        content_preview: Option<String>,
    },
    /// A drafted plan is waiting for manual review before execution.
    PlanReviewRequested {
        review_id: String,
        plan_title: String,
        plan_file: String,
        estimated_effort: String,
        overview: String,
        scope_in: Vec<String>,
        scope_out: Vec<String>,
    },
    /// The active service run acknowledged cancellation.
    Cancelled,
}

/// Presentation routing for submitted composer text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputIntent {
    Ignore,
    Command(String),
    ShellCommand(String),
    NewTurn(String),
    FollowUp(String),
}

/// Classify composer text without performing service work.
#[cfg(test)]
pub fn classify_input(input: &str, is_streaming: bool) -> InputIntent {
    classify_input_with_attachments(input, is_streaming, false)
}

/// Classify composer content while allowing an attachment-only new turn.
pub fn classify_input_with_attachments(
    input: &str,
    is_streaming: bool,
    has_attachments: bool,
) -> InputIntent {
    let trimmed = input.trim();
    if trimmed.is_empty() && !has_attachments {
        return InputIntent::Ignore;
    }
    if !has_attachments {
        if let Some(command) = trimmed.strip_prefix('/') {
            return InputIntent::Command(command.to_string());
        }
        if let Some(command) = trimmed.strip_prefix('!').map(str::trim) {
            if !command.is_empty() {
                return InputIntent::ShellCommand(command.to_string());
            }
        }
    }
    if is_streaming {
        InputIntent::FollowUp(trimmed.to_string())
    } else {
        InputIntent::NewTurn(trimmed.to_string())
    }
}

/// Classify raw composer text while persistent shell mode is active.
pub fn classify_shell_input(input: &str) -> InputIntent {
    let command = input.trim();
    if command.is_empty() {
        InputIntent::Ignore
    } else {
        InputIntent::ShellCommand(command.to_string())
    }
}

/// Receiver and cancellation handle for one active service turn.
pub struct ActiveChat {
    pub events: mpsc::Receiver<ChatEvent>,
    cancellation: TurnCancellationToken,
}

impl ActiveChat {
    /// Request cancellation of the service-owned turn.
    pub fn cancel(&self) {
        self.cancellation.cancel();
    }
}

/// Submit text plus typed attachments from a structured composer draft.
pub fn submit_message_with_attachments(
    input: &str,
    attachments: Vec<y_core::types::Attachment>,
    state: &mut AppState,
    services: &Arc<AppServices>,
) -> Option<ActiveChat> {
    submit_message_with_mode_and_attachments(input, state.turn_mode, attachments, state, services)
}

/// Execute an operator-entered `!command` through the service guardrail and
/// sandbox pipeline while reusing the normal TUI streaming lifecycle.
pub fn submit_shell_command(
    command: &str,
    confirmed: bool,
    state: &mut AppState,
    services: &Arc<AppServices>,
) -> Option<ActiveChat> {
    let command = command.trim();
    if command.is_empty() || state.is_streaming {
        return None;
    }

    state.messages.push(ChatMessage {
        role: MessageRole::User,
        content: format!("!{command}"),
        timestamp: Utc::now(),
        is_streaming: false,
        is_cancelled: false,
        reasoning_content: String::new(),
        reasoning_complete: false,
        tool_calls: Vec::new(),
        segments: Vec::new(),
        attachments: Vec::new(),
    });
    state.messages.push(ChatMessage {
        role: MessageRole::Assistant,
        content: String::new(),
        timestamp: Utc::now(),
        is_streaming: true,
        is_cancelled: false,
        reasoning_content: String::new(),
        reasoning_complete: false,
        tool_calls: Vec::new(),
        segments: Vec::new(),
        attachments: Vec::new(),
    });
    state.is_streaming = true;
    state.is_cancelling = false;
    state.scroll_offset = 0;

    let services = Arc::clone(services);
    let command = command.to_string();
    let current_session = state.current_session_id.clone();
    let (tx, rx) = mpsc::channel(8);
    let cancellation = TurnCancellationToken::new();
    let task_cancellation = cancellation.clone();
    tokio::spawn(async move {
        let (session_id, created_session) = if let Some(id) = current_session {
            (SessionId::from_string(id), None)
        } else {
            match create_workspace_session(&services).await {
                Ok(session) => (session.id.clone(), Some(session)),
                Err(error) => {
                    let _ = tx.send(ChatEvent::Error(error)).await;
                    return;
                }
            }
        };
        if let Some(session) = created_session {
            let _ = tx
                .send(ChatEvent::SessionCreated {
                    id: session.id.to_string(),
                    title: "New Chat".into(),
                    updated_at: session.updated_at,
                })
                .await;
        }

        let call_id = uuid::Uuid::new_v4().to_string();
        let _ = tx
            .send(ChatEvent::ToolCallStarted {
                tool_call_id: call_id.clone(),
                name: "ShellExec".into(),
                input_preview: command.clone(),
                agent_name: "operator".into(),
            })
            .await;
        let started = std::time::Instant::now();
        let working_dir = std::env::current_dir()
            .ok()
            .and_then(|path| path.to_str().map(str::to_owned));
        let result = y_service::OperatorShellService::execute(
            &services,
            y_service::OperatorShellRequest {
                session_id: &session_id,
                command: &command,
                working_dir: working_dir.as_deref(),
                additional_read_dirs: &[],
                confirmed,
                cancellation: Some(&task_cancellation),
            },
        )
        .await;
        let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        match result {
            Ok(output) => {
                let result_preview = serde_json::to_string_pretty(&output.content)
                    .unwrap_or_else(|_| output.content.to_string());
                let _ = tx
                    .send(ChatEvent::ToolCallCompleted {
                        tool_call_id: call_id,
                        name: "ShellExec".into(),
                        success: output.success,
                        duration_ms,
                        input_preview: command,
                        result_preview,
                        agent_name: "operator".into(),
                        url_meta: None,
                        metadata: Some(output.metadata),
                    })
                    .await;
                let _ = tx
                    .send(ChatEvent::Response {
                        content: String::new(),
                        model: "local-shell".into(),
                        input_tokens: 0,
                        output_tokens: 0,
                        last_input_tokens: 0,
                        context_window: 0,
                        cost_usd: 0.0,
                    })
                    .await;
            }
            Err(y_service::OperatorShellError::Execution(y_core::tool::ToolError::Cancelled)) => {
                let _ = tx.send(ChatEvent::Cancelled).await;
            }
            Err(error) => {
                let _ = tx
                    .send(ChatEvent::ToolCallCompleted {
                        tool_call_id: call_id,
                        name: "ShellExec".into(),
                        success: false,
                        duration_ms,
                        input_preview: command,
                        result_preview: error.to_string(),
                        agent_name: "operator".into(),
                        url_meta: None,
                        metadata: None,
                    })
                    .await;
                let _ = tx.send(ChatEvent::Error(error.to_string())).await;
            }
        }
    });

    Some(ActiveChat {
        events: rx,
        cancellation,
    })
}

/// Submit a user message with an optional service-owned orchestration mode.
pub fn submit_message_with_mode(
    input: &str,
    turn_mode: TurnMode,
    state: &mut AppState,
    services: &Arc<AppServices>,
) -> Option<ActiveChat> {
    submit_message_with_mode_and_attachments(input, turn_mode, Vec::new(), state, services)
}

/// Render the chat-history content for a submitted user turn, keeping an
/// attachment marker visible even when the turn also carries text.
fn user_display_content(trimmed: &str, attachments: &[y_core::types::Attachment]) -> String {
    let markers = attachment_markers(attachments);
    if trimmed.is_empty() {
        markers
    } else if markers.is_empty() {
        trimmed.to_string()
    } else {
        format!("{trimmed}\n{markers}")
    }
}

/// Placeholder markers like `[Image: clipboard.png]`, one per attachment.
pub(crate) fn attachment_markers(attachments: &[y_core::types::Attachment]) -> String {
    attachments
        .iter()
        .map(|attachment| {
            let kind = if attachment.mime_type.starts_with("image/") {
                "Image"
            } else {
                "File"
            };
            format!("[{kind}: {}]", attachment.filename)
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Re-attach attachment markers from persisted message metadata so replayed
/// transcripts show the same indicators as freshly submitted turns.
pub(crate) fn content_with_attachment_markers(
    content: &str,
    metadata: &serde_json::Value,
) -> String {
    let attachments: Vec<y_core::types::Attachment> = metadata
        .get("attachments")
        .and_then(|value| serde_json::from_value(value.clone()).ok())
        .unwrap_or_default();
    if attachments.is_empty() {
        return content.to_string();
    }
    user_display_content(content.trim(), &attachments)
}

/// Extract typed attachments from persisted message metadata.
///
/// Used during transcript loading to populate `ChatMessage::attachments`
/// so inline images can be rendered when the terminal supports it.
pub(crate) fn attachments_from_metadata(
    metadata: &serde_json::Value,
) -> Vec<y_core::types::Attachment> {
    metadata
        .get("attachments")
        .and_then(|value| serde_json::from_value(value.clone()).ok())
        .unwrap_or_default()
}

fn submit_message_with_mode_and_attachments(
    input: &str,
    turn_mode: TurnMode,
    attachments: Vec<y_core::types::Attachment>,
    state: &mut AppState,
    services: &Arc<AppServices>,
) -> Option<ActiveChat> {
    let trimmed = input.trim();
    if trimmed.is_empty() && attachments.is_empty() {
        return None;
    }

    let display_content = user_display_content(trimmed, &attachments);

    // Add user message to chat history.
    state.messages.push(ChatMessage {
        role: MessageRole::User,
        content: display_content,
        timestamp: Utc::now(),
        is_streaming: false,
        is_cancelled: false,
        reasoning_content: String::new(),
        reasoning_complete: false,
        tool_calls: Vec::new(),
        segments: Vec::new(),
        attachments: attachments.clone(),
    });

    // Track user message count for title summarization trigger.
    let user_msg_count = state.increment_user_message_count();

    // Reset scroll to bottom.
    state.scroll_offset = 0;

    // Clone the Arc for the spawned task.
    let services = Arc::clone(services);

    let session_id_opt = state.current_session_id.clone();
    let trimmed_owned = trimmed.to_string();
    let selected_provider_id = state.selected_provider_id.clone();
    let attachments_owned = attachments;

    // Determine if title generation should be triggered. The title only
    // consumes user messages, so it regenerates on every send (interval acts
    // as an on/off switch: 0 disables it).
    let title_interval = services.session_manager.config().title_summarize_interval;
    let should_generate_title = title_interval > 0 && user_msg_count > 0;

    // Mark state as streaming — add placeholder assistant message.
    state.is_streaming = true;
    state.is_cancelling = false;
    state.messages.push(ChatMessage {
        role: MessageRole::Assistant,
        content: String::new(),
        timestamp: Utc::now(),
        is_streaming: true,
        is_cancelled: false,
        reasoning_content: String::new(),
        reasoning_complete: false,
        tool_calls: Vec::new(),
        segments: Vec::new(),
        attachments: Vec::new(),
    });

    // Spawn async task for LLM call.
    let (tx, rx) = mpsc::channel(16);
    let cancellation = TurnCancellationToken::new();
    let task_cancellation = cancellation.clone();

    tokio::spawn(async move {
        let (prepared, created_session) = match prepare_tui_turn(
            &services,
            session_id_opt,
            trimmed_owned,
            selected_provider_id,
            turn_mode,
            attachments_owned,
        )
        .await
        {
            Ok(prepared) => prepared,
            Err(error) => {
                warn!(%error, "failed to prepare TUI turn");
                let _ = tx.send(ChatEvent::Error(error)).await;
                return;
            }
        };
        run_prepared_turn(
            services,
            prepared,
            created_session,
            should_generate_title,
            tx,
            task_cancellation,
        )
        .await;
    });

    Some(ActiveChat {
        events: rx,
        cancellation,
    })
}

/// Prepare the latest non-invalidated LLM checkpoint for an explicit retry.
pub async fn prepare_retry_last_request(
    services: &AppServices,
    session_id: &SessionId,
    config_dir: Option<&std::path::Path>,
) -> Result<Option<PreparedTurn>, String> {
    let checkpoints = services
        .chat_checkpoint_manager
        .list_checkpoints(session_id)
        .await
        .map_err(|error| format!("Could not list retry checkpoints: {error}"))?;
    let Some(checkpoint) = checkpoints.first() else {
        return Ok(None);
    };

    y_service::SystemService::thaw_frozen_providers(services).await;
    let mut prepared = ChatService::prepare_resend_turn(
        services,
        ResendTurnRequest {
            session_id: session_id.clone(),
            checkpoint_id: checkpoint.checkpoint_id.clone(),
            provider_id: None,
            request_mode: None,
            knowledge_collections: None,
            thinking: None,
            plan_mode: None,
            operation_mode: None,
        },
    )
    .await
    .map_err(|error| error.to_string())?;
    let effective_config_dir = config_dir
        .map(std::path::Path::to_path_buf)
        .or_else(crate::config::dirs_user_config)
        .ok_or_else(|| "Failed to resolve the y-agent configuration directory".to_string())?;
    let fallback_working_directory = std::env::current_dir().ok();
    ChatService::apply_prepared_turn_context(
        services,
        &effective_config_dir,
        fallback_working_directory.as_deref(),
        &mut prepared,
    )
    .await;
    Ok(Some(prepared))
}

/// Start a previously prepared resend without appending another user message.
pub fn submit_prepared_retry(
    prepared: PreparedTurn,
    state: &mut AppState,
    services: &Arc<AppServices>,
) -> ActiveChat {
    state.is_streaming = true;
    state.is_cancelling = false;
    state.scroll_offset = 0;
    state.messages.push(ChatMessage {
        role: MessageRole::Assistant,
        content: String::new(),
        timestamp: Utc::now(),
        is_streaming: true,
        is_cancelled: false,
        reasoning_content: String::new(),
        reasoning_complete: false,
        tool_calls: Vec::new(),
        segments: Vec::new(),
        attachments: Vec::new(),
    });

    let services = Arc::clone(services);
    let (tx, rx) = mpsc::channel(16);
    let cancellation = TurnCancellationToken::new();
    let task_cancellation = cancellation.clone();
    tokio::spawn(async move {
        run_prepared_turn(services, prepared, None, false, tx, task_cancellation).await;
    });

    ActiveChat {
        events: rx,
        cancellation,
    }
}

async fn run_prepared_turn(
    services: Arc<AppServices>,
    prepared: PreparedTurn,
    created_session: Option<y_core::session::SessionNode>,
    should_generate_title: bool,
    tx: mpsc::Sender<ChatEvent>,
    cancellation: TurnCancellationToken,
) {
    let session_id = prepared.session_id.clone();
    let session_id_str = session_id.to_string();
    if let Some(node) = created_session {
        let _ = tx
            .send(ChatEvent::SessionCreated {
                id: session_id_str.clone(),
                title: "New Chat".into(),
                updated_at: node.updated_at,
            })
            .await;
    }

    if should_generate_title {
        spawn_title_generation(
            Arc::clone(&services),
            tx.clone(),
            session_id.clone(),
            session_id_str,
        );
    }

    let turn_input = prepared.as_turn_input();
    let (progress_tx, progress_rx) = y_service::TurnEventSender::channel();
    let (progress_finish_tx, progress_finish_rx) = tokio::sync::oneshot::channel();
    let progress_forwarder = tokio::spawn(forward_progress_events(
        progress_rx,
        tx.clone(),
        progress_finish_rx,
    ));

    ChatService::begin_follow_up_run(&services, &session_id);
    let result = orchestrator::execute_turn_streaming(
        &services,
        &turn_input,
        progress_tx,
        Some(cancellation),
    )
    .await;
    let _ = progress_finish_tx.send(());
    let _ = progress_forwarder.await;
    ChatService::finish_follow_up_run(&services, &session_id).await;

    match result {
        Ok(result) => {
            let _ = tx
                .send(ChatEvent::ToolCallsSnapshot {
                    calls: result.tool_calls_executed.clone(),
                })
                .await;
            let _ = tx
                .send(ChatEvent::Response {
                    content: result.content,
                    model: result.model,
                    input_tokens: result.input_tokens,
                    output_tokens: result.output_tokens,
                    last_input_tokens: result.last_input_tokens,
                    context_window: result.context_window,
                    cost_usd: result.cost_usd,
                })
                .await;
        }
        Err(TurnError::Cancelled) => {
            let _ = tx.send(ChatEvent::Cancelled).await;
        }
        Err(error) => {
            let _ = tx.send(ChatEvent::Error(error.to_string())).await;
        }
    }
}

fn spawn_title_generation(
    services: Arc<AppServices>,
    tx: mpsc::Sender<ChatEvent>,
    session_id: SessionId,
    session_id_string: String,
) {
    tokio::spawn(async move {
        let has_manual_title = services
            .session_manager
            .get_session(&session_id)
            .await
            .map(|session| session.manual_title.is_some())
            .unwrap_or(false);
        if has_manual_title {
            return;
        }
        let transcript = match services.session_manager.read_transcript(&session_id).await {
            Ok(transcript) => transcript,
            Err(error) => {
                warn!(%error, "failed to read transcript for title generation");
                return;
            }
        };
        match services
            .session_manager
            .generate_title(&*services.agent_delegator, &session_id, &transcript)
            .await
        {
            Ok(title) => {
                let _ = tx
                    .send(ChatEvent::TitleUpdated {
                        session_id: session_id_string,
                        title,
                    })
                    .await;
            }
            Err(error) => warn!(%error, "title generation failed"),
        }
    });
}

async fn forward_progress_events(
    mut progress_rx: mpsc::UnboundedReceiver<(y_service::TurnEvent, Option<SessionId>)>,
    tx: mpsc::Sender<ChatEvent>,
    mut finish: tokio::sync::oneshot::Receiver<()>,
) {
    loop {
        tokio::select! {
            biased;
            _ = &mut finish => {
                while let Ok((event, _session_id)) = progress_rx.try_recv() {
                    if !forward_progress_event(event, &tx).await {
                        return;
                    }
                }
                return;
            }
            received = progress_rx.recv() => {
                let Some((event, _session_id)) = received else {
                    return;
                };
                if !forward_progress_event(event, &tx).await {
                    return;
                }
            }
        }
    }
}

async fn forward_progress_event(event: y_service::TurnEvent, tx: &mpsc::Sender<ChatEvent>) -> bool {
    let event = match event {
        y_service::TurnEvent::StreamDelta { content, .. } => {
            Some(ChatEvent::StreamDelta { content })
        }
        y_service::TurnEvent::StreamReasoningDelta { content, .. } => {
            Some(ChatEvent::StreamReasoningDelta { content })
        }
        y_service::TurnEvent::ToolStart {
            tool_call_id,
            name,
            input_preview,
            agent_name,
        } => Some(ChatEvent::ToolCallStarted {
            tool_call_id,
            name,
            input_preview,
            agent_name,
        }),
        y_service::TurnEvent::ToolResult {
            tool_call_id,
            name,
            success,
            duration_ms,
            input_preview,
            result_preview,
            agent_name,
            url_meta,
            metadata,
        } => Some(ChatEvent::ToolCallCompleted {
            tool_call_id,
            name,
            success,
            duration_ms,
            input_preview,
            result_preview,
            agent_name,
            url_meta,
            metadata,
        }),
        y_service::TurnEvent::FollowUpInjected { follow_up_id, text } => {
            Some(ChatEvent::FollowUpInjected { follow_up_id, text })
        }
        y_service::TurnEvent::SteerInjected { steer_id, text } => {
            Some(ChatEvent::SteerInjected { steer_id, text })
        }
        y_service::TurnEvent::UserInteractionRequest {
            interaction_id,
            questions,
            ..
        } => Some(ChatEvent::AskUserRequested {
            interaction_id,
            questions,
        }),
        y_service::TurnEvent::PermissionRequest {
            request_id,
            tool_name,
            action_description,
            reason,
            content_preview,
        } => Some(ChatEvent::PermissionRequested {
            request_id,
            tool_name,
            action_description,
            reason,
            content_preview,
        }),
        y_service::TurnEvent::PlanReviewRequest {
            review_id,
            plan_title,
            plan_file,
            estimated_effort,
            overview,
            scope_in,
            scope_out,
            ..
        } => Some(ChatEvent::PlanReviewRequested {
            review_id,
            plan_title,
            plan_file,
            estimated_effort,
            overview,
            scope_in,
            scope_out,
        }),
        _ => None,
    };
    if let Some(event) = event {
        tx.send(event).await.is_ok()
    } else {
        true
    }
}

async fn create_workspace_session(
    services: &AppServices,
) -> Result<y_core::session::SessionNode, String> {
    let workspace = std::env::current_dir()
        .map_err(|error| format!("Failed to resolve current workspace: {error}"))?;
    y_service::SessionService::create_session(
        &services.session_manager,
        CreateSessionOptions {
            parent_id: None,
            session_type: SessionType::Main,
            agent_id: None,
            title: Some("New Chat".into()),
        },
        &workspace,
    )
    .await
    .map_err(|error| format!("Failed to create session: {error}"))
}

async fn prepare_tui_turn(
    services: &AppServices,
    session_id: Option<String>,
    user_input: String,
    provider_id: Option<String>,
    turn_mode: TurnMode,
    attachments: Vec<y_core::types::Attachment>,
) -> Result<(PreparedTurn, Option<y_core::session::SessionNode>), String> {
    let (session_id, created_session) = if let Some(session_id) = session_id {
        (SessionId::from_string(session_id), None)
    } else {
        let session = create_workspace_session(services).await?;
        (session.id.clone(), Some(session))
    };
    let mut prepared = ChatService::prepare_turn(
        services,
        PrepareTurnRequest {
            session_id: Some(session_id),
            user_input,
            provider_id,
            request_mode: Some(y_core::provider::RequestMode::TextChat),
            plan_mode: turn_mode.plan_mode().map(str::to_string),
            operation_mode: Some(y_service::chat_types::OperationMode::Default),
            attachments,
            ..PrepareTurnRequest::default()
        },
    )
    .await
    .map_err(|error| error.to_string())?;
    let config_dir = crate::config::dirs_user_config()
        .ok_or_else(|| "Failed to resolve the y-agent configuration directory".to_string())?;
    let fallback_working_directory = std::env::current_dir().ok();
    ChatService::apply_prepared_turn_context(
        services,
        &config_dir,
        fallback_working_directory.as_deref(),
        &mut prepared,
    )
    .await;
    Ok((prepared, created_session))
}

/// Apply a `ChatEvent` to the state.
///
/// Called by the main event loop when the async LLM task sends results.
pub fn apply_chat_event(event: ChatEvent, state: &mut AppState) {
    match event {
        ChatEvent::Response {
            content,
            model,
            input_tokens,
            output_tokens,
            last_input_tokens,
            context_window,
            cost_usd,
        } => {
            // Update the last (streaming) assistant message.
            if let Some(last) = state.messages.last_mut() {
                if last.role == MessageRole::Assistant && last.is_streaming {
                    finish_active_reasoning(last);
                    if last.content.is_empty() {
                        last.content.clone_from(&content);
                        if !last.segments.is_empty() && !content.is_empty() {
                            last.segments.push(StreamSegment::Text(content));
                        }
                    }
                    last.is_streaming = false;
                    last.reasoning_complete = true;
                }
            }
            state.is_streaming = false;
            state.is_cancelling = false;
            // The service destroys the queue when the run finishes.
            state.follow_up_queue.clear();

            // Update status bar data.
            state.status_model = model;
            state.status_tokens = format!("{input_tokens}\u{2191} {output_tokens}\u{2193}");

            // Track cumulative tokens and context window for usage display.
            state.cumulative_input_tokens += input_tokens;
            state.cumulative_output_tokens += output_tokens;
            state.last_input_tokens = last_input_tokens;
            if context_window > 0 {
                state.context_window = context_window;
            }
            if cost_usd > 0.0 {
                state.last_cost = Some(state.last_cost.unwrap_or(0.0) + cost_usd);
            }
        }
        ChatEvent::ToolCallStarted {
            tool_call_id,
            name,
            input_preview,
            agent_name,
        } => {
            if let Some(last) = state.messages.last_mut() {
                if last.role == MessageRole::Assistant && last.is_streaming {
                    finish_active_reasoning(last);
                    let tool_call = ToolCallInfo {
                        tool_call_id,
                        name,
                        status: ToolCallStatus::Running,
                        duration_ms: None,
                        input_preview,
                        result_preview: String::new(),
                        agent_name,
                        url_meta: None,
                        metadata: None,
                        display_mode: ToolCallDisplayMode::Preview,
                    };
                    let tool_index = last.tool_calls.len();
                    last.tool_calls.push(tool_call);
                    last.segments.push(StreamSegment::ToolCall(tool_index));
                }
            }
        }
        ChatEvent::ToolCallCompleted {
            tool_call_id,
            name,
            success,
            duration_ms,
            input_preview,
            result_preview,
            agent_name,
            url_meta,
            metadata,
        } => {
            if let Some(last) = state.messages.last_mut() {
                if last.role == MessageRole::Assistant && last.is_streaming {
                    let completed = ToolCallInfo {
                        tool_call_id,
                        name,
                        status: if success {
                            ToolCallStatus::Succeeded
                        } else {
                            ToolCallStatus::Failed
                        },
                        duration_ms: Some(duration_ms),
                        input_preview,
                        result_preview,
                        agent_name,
                        url_meta,
                        metadata,
                        display_mode: ToolCallDisplayMode::Preview,
                    };
                    complete_or_append_tool_call(last, completed);
                }
            }
        }
        ChatEvent::ToolCallsSnapshot { calls } => {
            if let Some(last) = state.messages.last_mut() {
                if last.role == MessageRole::Assistant && last.is_streaming {
                    reconcile_tool_calls(last, calls);
                }
            }
        }
        ChatEvent::StreamDelta { content } => {
            // Append incremental text to the streaming assistant message.
            if let Some(last) = state.messages.last_mut() {
                if last.role == MessageRole::Assistant && last.is_streaming {
                    finish_active_reasoning(last);
                    last.content.push_str(&content);
                    // Maintain event-ordered segments for interleaved rendering.
                    if let Some(StreamSegment::Text(ref mut text)) = last.segments.last_mut() {
                        text.push_str(&content);
                    } else {
                        last.segments.push(StreamSegment::Text(content));
                    }
                }
            }
        }
        ChatEvent::StreamReasoningDelta { content } => {
            // Preserve reasoning at its event-ordered timeline position.
            if let Some(last) = state.messages.last_mut() {
                if last.role == MessageRole::Assistant && last.is_streaming {
                    last.reasoning_content.push_str(&content);
                    if let Some(StreamSegment::Reasoning {
                        content: reasoning,
                        is_complete: false,
                    }) = last.segments.last_mut()
                    {
                        reasoning.push_str(&content);
                    } else {
                        last.segments.push(StreamSegment::Reasoning {
                            content,
                            is_complete: false,
                        });
                    }
                }
            }
        }
        ChatEvent::Error(msg) => {
            // Replace the streaming message with error.
            if let Some(last) = state.messages.last_mut() {
                if last.role == MessageRole::Assistant && last.is_streaming {
                    finish_active_reasoning(last);
                    last.content = format!("Error: {msg}");
                    if !last.segments.is_empty() {
                        last.segments
                            .push(StreamSegment::Text(last.content.clone()));
                    }
                    last.is_streaming = false;
                    last.is_cancelled = true;
                }
            }
            state.is_streaming = false;
            state.is_cancelling = false;
            // The service destroys the queue when the run finishes.
            state.follow_up_queue.clear();

            // Also emit a transient warning toast.
            state.push_toast(msg, crate::tui::state::ToastLevel::Warning);
        }
        ChatEvent::FollowUpInjected { follow_up_id, text } => {
            // Drop the injected item from the queue projection.
            state.follow_up_queue.retain(|item| item.id != follow_up_id);
            if let Some(last) = state.messages.last_mut() {
                if last.role == MessageRole::Assistant && last.is_streaming {
                    finish_active_reasoning(last);
                    last.is_streaming = false;
                    last.reasoning_complete = true;
                }
            }
            state.messages.push(ChatMessage {
                role: MessageRole::User,
                content: text,
                timestamp: Utc::now(),
                is_streaming: false,
                is_cancelled: false,
                reasoning_content: String::new(),
                reasoning_complete: false,
                tool_calls: Vec::new(),
                segments: Vec::new(),
                attachments: Vec::new(),
            });
            state.messages.push(ChatMessage {
                role: MessageRole::Assistant,
                content: String::new(),
                timestamp: Utc::now(),
                is_streaming: true,
                is_cancelled: false,
                reasoning_content: String::new(),
                reasoning_complete: false,
                tool_calls: Vec::new(),
                segments: Vec::new(),
                attachments: Vec::new(),
            });
        }
        ChatEvent::SteerInjected { steer_id, text } => {
            // Drop the injected steer from the queue projection; the run
            // continues with the same streaming assistant message.
            state.follow_up_queue.retain(|item| item.id != steer_id);
            state.push_toast(
                format!("Steering the active run: {text}"),
                crate::tui::state::ToastLevel::Info,
            );
        }
        ChatEvent::AskUserRequested { .. }
        | ChatEvent::PermissionRequested { .. }
        | ChatEvent::PlanReviewRequested { .. } => {}
        ChatEvent::Cancelled => {
            if let Some(last) = state.messages.last_mut() {
                if last.role == MessageRole::Assistant && last.is_streaming {
                    finish_active_reasoning(last);
                    last.is_streaming = false;
                    last.is_cancelled = true;
                    last.reasoning_complete = true;
                    if last.content.is_empty() {
                        last.content = "(cancelled)".to_string();
                        if !last.segments.is_empty() {
                            last.segments
                                .push(StreamSegment::Text(last.content.clone()));
                        }
                    }
                }
            }
            state.is_streaming = false;
            state.is_cancelling = false;
            // The service destroys the queue when the run finishes.
            state.follow_up_queue.clear();
            state.push_toast(
                "Response cancelled.".into(),
                crate::tui::state::ToastLevel::Info,
            );
        }
        ChatEvent::TitleUpdated { session_id, title } => {
            // Update matching recent-session entry used by status and completion.
            if let Some(session) = state.sessions.iter_mut().find(|s| s.id == session_id) {
                session.title = title;
            }
        }
        ChatEvent::SessionCreated {
            id,
            title,
            updated_at,
        } => {
            // Insert newly created session at the top of the recent-session list.
            state.current_session_id = Some(id.clone());
            state.sessions.insert(
                0,
                SessionListItem {
                    id,
                    title,
                    updated_at,
                    message_count: 0,
                    state: y_core::session::SessionState::Active,
                    parent_id: None,
                    depth: 0,
                    pinned: false,
                    quick_slot: None,
                },
            );
        }
    }
}

fn complete_or_append_tool_call(message: &mut ChatMessage, completed: ToolCallInfo) {
    let existing_index = message
        .tool_calls
        .iter()
        .position(|tool| tool.tool_call_id == completed.tool_call_id);

    if let Some(index) = existing_index {
        let display_mode = message.tool_calls[index].display_mode;
        message.tool_calls[index] = ToolCallInfo {
            display_mode,
            ..completed
        };
    } else {
        finish_active_reasoning(message);
        let tool_index = message.tool_calls.len();
        message.tool_calls.push(completed);
        message.segments.push(StreamSegment::ToolCall(tool_index));
    }
}

fn reconcile_tool_calls(message: &mut ChatMessage, calls: Vec<y_service::ToolCallRecord>) {
    for call in calls {
        let status = if call.success {
            ToolCallStatus::Succeeded
        } else {
            ToolCallStatus::Failed
        };
        if let Some(existing) = message
            .tool_calls
            .iter_mut()
            .find(|tool| tool.tool_call_id == call.tool_call_id)
        {
            existing.status = status;
            existing.duration_ms = Some(call.duration_ms);
            if existing.input_preview.is_empty() {
                existing.input_preview.clone_from(&call.arguments);
            }
            existing.result_preview = call.result_content;
            existing.url_meta = call.url_meta;
            existing.metadata = call.metadata;
            continue;
        }
        complete_or_append_tool_call(
            message,
            ToolCallInfo {
                tool_call_id: call.tool_call_id,
                name: call.name,
                status,
                duration_ms: Some(call.duration_ms),
                input_preview: call.arguments,
                result_preview: call.result_content,
                agent_name: "chat-turn".into(),
                url_meta: call.url_meta,
                metadata: call.metadata,
                display_mode: ToolCallDisplayMode::Preview,
            },
        );
    }
}

fn finish_active_reasoning(message: &mut ChatMessage) {
    if let Some(StreamSegment::Reasoning { is_complete, .. }) = message.segments.last_mut() {
        *is_complete = true;
    }
}

/// Enqueue composer text into the service-owned follow-up queue.
pub fn enqueue_follow_up(
    input: &str,
    state: &AppState,
    services: &AppServices,
) -> Result<y_service::FollowUpMessage, String> {
    let text = input.trim();
    if text.is_empty() {
        return Err("follow-up text must not be empty".to_string());
    }
    let session_id = state
        .current_session_id
        .as_ref()
        .ok_or_else(|| "the active session is not ready yet".to_string())?;
    ChatService::add_follow_up(
        services,
        &SessionId::from_string(session_id.clone()),
        text.to_string(),
    )
    .map_err(|error| error.to_string())
}

/// Refresh the TUI projection of the service-side follow-up queue.
///
/// Best-effort: the service owns a queue only while a run is streaming, so
/// the projection is only re-read for an active streaming session; otherwise
/// the previous projection is kept.
pub fn refresh_follow_up_queue(state: &mut AppState, services: &AppServices) {
    let Some(ref session_id) = state.current_session_id else {
        return;
    };
    if !state.is_streaming {
        return;
    }
    state.follow_up_queue =
        ChatService::list_follow_ups(services, &SessionId::from_string(session_id.clone()));
}

/// Mark the active response as awaiting service-side cancellation.
pub fn cancel_streaming(state: &mut AppState) {
    if state.is_streaming && !state.is_cancelling {
        state.is_cancelling = true;
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::state::SessionListItem;

    #[tokio::test]
    async fn progress_forwarder_finishes_even_when_a_sender_clone_lingers() {
        let (progress, progress_rx) = y_service::TurnEventSender::channel();
        let lingering_sender = progress.clone();
        let (chat_tx, mut chat_rx) = mpsc::channel(4);
        let (finish_tx, finish_rx) = tokio::sync::oneshot::channel();
        let forwarder = tokio::spawn(forward_progress_events(progress_rx, chat_tx, finish_rx));
        progress
            .send(y_service::TurnEvent::StreamDelta {
                content: "partial".to_string(),
                agent_name: "root".to_string(),
            })
            .unwrap();

        finish_tx.send(()).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), forwarder)
            .await
            .expect("forwarder waited for the lingering sender")
            .unwrap();

        assert!(matches!(
            chat_rx.recv().await,
            Some(ChatEvent::StreamDelta { content }) if content == "partial"
        ));
        drop(lingering_sender);
    }

    #[tokio::test]
    async fn prepare_tui_turn_uses_shared_service_preparation() {
        let temp = tempfile::tempdir().unwrap();
        let mut config = y_service::ServiceConfig {
            storage: y_service::config_types::StorageConfig::in_memory(),
            ..Default::default()
        };
        config.storage.transcript_dir = temp.path().join("transcripts");
        let services = AppServices::from_config(&config).await.unwrap();

        let (prepared, created_session) = prepare_tui_turn(
            &services,
            None,
            "edit the file".into(),
            None,
            TurnMode::Fast,
            Vec::new(),
        )
        .await
        .unwrap();

        assert!(created_session.is_some());
        assert_eq!(prepared.history.last().unwrap().content, "edit the file");
        let managers = services.file_history_managers.read().await;
        let manager = managers.get(&prepared.session_id).unwrap();
        assert_eq!(manager.snapshots().len(), 1);
    }

    #[tokio::test]
    async fn prepare_retry_last_request_reuses_latest_user_turn() {
        let temp = tempfile::tempdir().unwrap();
        let mut config = y_service::ServiceConfig {
            storage: y_service::config_types::StorageConfig::in_memory(),
            ..Default::default()
        };
        config.storage.transcript_dir = temp.path().join("transcripts");
        let services = AppServices::from_config(&config).await.unwrap();
        let (prepared, _) = prepare_tui_turn(
            &services,
            None,
            "continue after the tool call".into(),
            None,
            TurnMode::Fast,
            Vec::new(),
        )
        .await
        .unwrap();
        services
            .chat_checkpoint_manager
            .create_checkpoint(&prepared.session_id, 1, 0, "scope-1".into())
            .await
            .unwrap();

        let resent = prepare_retry_last_request(&services, &prepared.session_id, Some(temp.path()))
            .await
            .unwrap()
            .expect("checkpoint should be retryable");

        assert_eq!(resent.user_input, "continue after the tool call");
        assert_eq!(resent.session_id, prepared.session_id);
    }

    #[tokio::test]
    async fn prepare_retry_last_request_returns_none_without_checkpoint() {
        let config = y_service::ServiceConfig {
            storage: y_service::config_types::StorageConfig::in_memory(),
            ..Default::default()
        };
        let services = AppServices::from_config(&config).await.unwrap();

        let resent = prepare_retry_last_request(&services, &SessionId::new(), None)
            .await
            .unwrap();

        assert!(resent.is_none());
    }

    // T-TUI-05-01: User message appended to history.
    #[test]
    fn test_apply_chat_response() {
        let mut state = AppState::new();
        state.is_streaming = true;
        state.messages.push(ChatMessage {
            role: MessageRole::Assistant,
            content: String::new(),
            timestamp: Utc::now(),
            is_streaming: true,
            is_cancelled: false,
            reasoning_content: String::new(),
            reasoning_complete: false,
            tool_calls: Vec::new(),
            segments: Vec::new(),
            attachments: Vec::new(),
        });

        apply_chat_event(
            ChatEvent::Response {
                content: "Hello!".into(),
                model: "gpt-4".into(),
                input_tokens: 10,
                output_tokens: 5,
                last_input_tokens: 10,
                context_window: 128_000,
                cost_usd: 0.001,
            },
            &mut state,
        );

        assert!(!state.is_streaming);
        let last = state.messages.last().unwrap();
        assert_eq!(last.content, "Hello!");
        assert!(!last.is_streaming);
        assert_eq!(state.status_model, "gpt-4");
        assert_eq!(state.last_cost, Some(0.001));
    }

    #[test]
    fn test_apply_chat_error() {
        let mut state = AppState::new();
        state.is_streaming = true;
        state.messages.push(ChatMessage {
            role: MessageRole::Assistant,
            content: String::new(),
            timestamp: Utc::now(),
            is_streaming: true,
            is_cancelled: false,
            reasoning_content: String::new(),
            reasoning_complete: false,
            tool_calls: Vec::new(),
            segments: Vec::new(),
            attachments: Vec::new(),
        });

        apply_chat_event(ChatEvent::Error("connection refused".into()), &mut state);

        assert!(!state.is_streaming);
        let last = state.messages.last().unwrap();
        assert!(last.content.contains("connection refused"));
        assert!(last.is_cancelled);
    }

    // T-TUI-05-03: Cancel streaming marks message.
    #[test]
    fn test_cancel_streaming_marks_request_pending() {
        let mut state = AppState::new();
        state.is_streaming = true;
        state.messages.push(ChatMessage {
            role: MessageRole::Assistant,
            content: "partial...".into(),
            timestamp: Utc::now(),
            is_streaming: true,
            is_cancelled: false,
            reasoning_content: String::new(),
            reasoning_complete: false,
            tool_calls: Vec::new(),
            segments: Vec::new(),
            attachments: Vec::new(),
        });

        cancel_streaming(&mut state);
        assert!(state.is_streaming);
        assert!(state.is_cancelling);
        assert!(!state.messages.last().unwrap().is_cancelled);
    }

    #[test]
    fn test_apply_chat_cancelled_preserves_partial_response() {
        let mut state = AppState::new();
        state.is_streaming = true;
        state.is_cancelling = true;
        state.messages.push(ChatMessage {
            role: MessageRole::Assistant,
            content: "partial response".into(),
            timestamp: Utc::now(),
            is_streaming: true,
            is_cancelled: false,
            reasoning_content: String::new(),
            reasoning_complete: false,
            tool_calls: Vec::new(),
            segments: vec![StreamSegment::Text("partial response".into())],
            attachments: Vec::new(),
        });

        apply_chat_event(ChatEvent::Cancelled, &mut state);

        assert!(!state.is_streaming);
        assert!(!state.is_cancelling);
        let last = state.messages.last().unwrap();
        assert_eq!(last.content, "partial response");
        assert!(last.is_cancelled);
    }

    #[test]
    fn test_apply_follow_up_injected_creates_real_history_boundary() {
        let mut state = AppState::new();
        state.is_streaming = true;
        state.messages.push(ChatMessage {
            role: MessageRole::Assistant,
            content: "first answer".into(),
            timestamp: Utc::now(),
            is_streaming: true,
            is_cancelled: false,
            reasoning_content: String::new(),
            reasoning_complete: false,
            tool_calls: Vec::new(),
            segments: vec![StreamSegment::Text("first answer".into())],
            attachments: Vec::new(),
        });

        apply_chat_event(
            ChatEvent::FollowUpInjected {
                follow_up_id: "fu-1".into(),
                text: "also add tests".into(),
            },
            &mut state,
        );

        assert_eq!(state.messages.len(), 3);
        assert_eq!(state.messages[0].role, MessageRole::Assistant);
        assert!(!state.messages[0].is_streaming);
        assert_eq!(state.messages[1].role, MessageRole::User);
        assert_eq!(state.messages[1].content, "also add tests");
        assert_eq!(state.messages[2].role, MessageRole::Assistant);
        assert!(state.messages[2].is_streaming);
    }

    fn queued_follow_up(id: &str, text: &str) -> y_service::FollowUpMessage {
        y_service::FollowUpMessage {
            id: id.into(),
            text: text.into(),
            created_at: 0,
            status: y_service::FollowUpStatus::Pending,
        }
    }

    fn streaming_assistant_message() -> ChatMessage {
        ChatMessage {
            role: MessageRole::Assistant,
            content: String::new(),
            timestamp: Utc::now(),
            is_streaming: true,
            is_cancelled: false,
            reasoning_content: String::new(),
            reasoning_complete: false,
            tool_calls: Vec::new(),
            segments: Vec::new(),
            attachments: Vec::new(),
        }
    }

    #[test]
    fn test_apply_follow_up_injected_removes_item_from_queue_projection() {
        let mut state = AppState::new();
        state.is_streaming = true;
        state.messages.push(streaming_assistant_message());
        state.follow_up_queue = vec![
            queued_follow_up("fu-1", "first"),
            queued_follow_up("fu-2", "second"),
        ];

        apply_chat_event(
            ChatEvent::FollowUpInjected {
                follow_up_id: "fu-1".into(),
                text: "first".into(),
            },
            &mut state,
        );

        let remaining: Vec<&str> = state
            .follow_up_queue
            .iter()
            .map(|item| item.id.as_str())
            .collect();
        assert_eq!(remaining, ["fu-2"]);
    }

    #[test]
    fn test_apply_steer_injected_removes_item_and_announces() {
        let mut state = AppState::new();
        state.is_streaming = true;
        state.messages.push(streaming_assistant_message());
        state.follow_up_queue = vec![
            queued_follow_up("fu-1", "first"),
            queued_follow_up("fu-2", "steer me"),
        ];

        apply_chat_event(
            ChatEvent::SteerInjected {
                steer_id: "fu-2".into(),
                text: "steer me".into(),
            },
            &mut state,
        );

        let remaining: Vec<&str> = state
            .follow_up_queue
            .iter()
            .map(|item| item.id.as_str())
            .collect();
        assert_eq!(remaining, ["fu-1"]);
        // The steer must not break the streaming boundary: no new messages.
        assert_eq!(state.messages.len(), 1);
        assert!(state.messages[0].is_streaming);
        let toast = state.toasts.back().expect("a steer toast is shown");
        assert!(toast.message.contains("steer me"));
        assert_eq!(toast.level, crate::tui::state::ToastLevel::Info);
    }

    #[test]
    fn test_terminal_events_clear_queue_projection() {
        for event in [
            ChatEvent::Response {
                content: "done".into(),
                model: "test".into(),
                input_tokens: 1,
                output_tokens: 1,
                last_input_tokens: 1,
                context_window: 1_000,
                cost_usd: 0.0,
            },
            ChatEvent::Error("boom".into()),
            ChatEvent::Cancelled,
        ] {
            let mut state = AppState::new();
            state.is_streaming = true;
            state.messages.push(streaming_assistant_message());
            state.follow_up_queue = vec![queued_follow_up("fu-1", "first")];

            apply_chat_event(event, &mut state);

            assert!(
                state.follow_up_queue.is_empty(),
                "terminal events must clear the queue projection"
            );
        }
    }

    #[tokio::test]
    async fn refresh_follow_up_queue_projects_service_queue_while_streaming() {
        let temp = tempfile::tempdir().unwrap();
        let mut config = y_service::ServiceConfig {
            storage: y_service::config_types::StorageConfig::in_memory(),
            ..Default::default()
        };
        config.storage.transcript_dir = temp.path().join("transcripts");
        let services = AppServices::from_config(&config).await.unwrap();

        let session_id = SessionId::new();
        ChatService::begin_follow_up_run(&services, &session_id);
        ChatService::add_follow_up(&services, &session_id, "queued one".to_string()).unwrap();
        ChatService::add_follow_up(&services, &session_id, "queued two".to_string()).unwrap();

        let mut state = AppState::new();
        state.current_session_id = Some(session_id.to_string());
        state.is_streaming = true;

        refresh_follow_up_queue(&mut state, &services);

        let texts: Vec<&str> = state
            .follow_up_queue
            .iter()
            .map(|item| item.text.as_str())
            .collect();
        assert_eq!(texts, ["queued one", "queued two"]);
    }

    #[tokio::test]
    async fn refresh_follow_up_queue_keeps_projection_when_not_streaming() {
        let config = y_service::ServiceConfig {
            storage: y_service::config_types::StorageConfig::in_memory(),
            ..Default::default()
        };
        let services = AppServices::from_config(&config).await.unwrap();

        let mut state = AppState::new();
        state.current_session_id = Some(SessionId::new().to_string());
        state.is_streaming = false;
        state.follow_up_queue = vec![queued_follow_up("fu-1", "stale")];

        refresh_follow_up_queue(&mut state, &services);

        assert_eq!(state.follow_up_queue.len(), 1);
        assert_eq!(state.follow_up_queue[0].text, "stale");
    }

    #[tokio::test]
    async fn refresh_follow_up_queue_without_session_is_noop() {
        let config = y_service::ServiceConfig {
            storage: y_service::config_types::StorageConfig::in_memory(),
            ..Default::default()
        };
        let services = AppServices::from_config(&config).await.unwrap();

        let mut state = AppState::new();
        state.is_streaming = true;

        refresh_follow_up_queue(&mut state, &services);

        assert!(state.follow_up_queue.is_empty());
    }

    #[test]
    fn test_final_response_does_not_duplicate_prior_follow_up_iterations() {
        let mut state = AppState::new();
        state.is_streaming = true;
        state.messages.push(ChatMessage {
            role: MessageRole::Assistant,
            content: "second answer".into(),
            timestamp: Utc::now(),
            is_streaming: true,
            is_cancelled: false,
            reasoning_content: String::new(),
            reasoning_complete: false,
            tool_calls: Vec::new(),
            segments: vec![StreamSegment::Text("second answer".into())],
            attachments: Vec::new(),
        });

        apply_chat_event(
            ChatEvent::Response {
                content: "first answer\nsecond answer".into(),
                model: "test-model".into(),
                input_tokens: 10,
                output_tokens: 5,
                last_input_tokens: 10,
                context_window: 1_000,
                cost_usd: 0.0,
            },
            &mut state,
        );

        assert_eq!(state.messages.last().unwrap().content, "second answer");
    }

    #[test]
    fn test_classify_input_queues_regular_text_while_streaming() {
        assert_eq!(
            classify_input("next task", true),
            InputIntent::FollowUp("next task".into())
        );
    }

    #[test]
    fn test_classify_input_keeps_commands_immediate_while_streaming() {
        assert_eq!(
            classify_input("/copy", true),
            InputIntent::Command("copy".into())
        );
    }

    #[test]
    fn test_classify_input_recognizes_shell_composer_command() {
        assert_eq!(
            classify_input("!cargo test", false),
            InputIntent::ShellCommand("cargo test".into())
        );
    }

    #[test]
    fn test_classify_shell_mode_treats_raw_text_as_a_command() {
        assert_eq!(
            classify_shell_input(" cargo test "),
            InputIntent::ShellCommand("cargo test".into())
        );
        assert_eq!(classify_shell_input("  "), InputIntent::Ignore);
    }

    #[test]
    fn test_active_chat_cancel_signals_service_token() {
        let cancellation = TurnCancellationToken::new();
        let (_, events) = mpsc::channel(1);
        let active_chat = ActiveChat {
            events,
            cancellation: cancellation.clone(),
        };

        active_chat.cancel();

        assert!(cancellation.is_cancelled());
    }

    // T-TUI-TITLE-01: TitleUpdated event updates session list.
    #[test]
    fn test_apply_title_updated() {
        let mut state = AppState::new();
        state.sessions.push(SessionListItem {
            id: "session-1".into(),
            title: String::new(),
            updated_at: Utc::now(),
            message_count: 3,
            state: y_core::session::SessionState::Active,
            parent_id: None,
            depth: 0,
            pinned: false,
            quick_slot: None,
        });

        apply_chat_event(
            ChatEvent::TitleUpdated {
                session_id: "session-1".into(),
                title: "New Title".into(),
            },
            &mut state,
        );

        assert_eq!(state.sessions[0].title, "New Title");
    }

    // T-TUI-TITLE-02: TitleUpdated for unknown session is no-op.
    #[test]
    fn test_apply_title_updated_unknown_session() {
        let mut state = AppState::new();
        state.sessions.push(SessionListItem {
            id: "session-1".into(),
            title: "Original".into(),
            updated_at: Utc::now(),
            message_count: 3,
            state: y_core::session::SessionState::Active,
            parent_id: None,
            depth: 0,
            pinned: false,
            quick_slot: None,
        });

        apply_chat_event(
            ChatEvent::TitleUpdated {
                session_id: "session-unknown".into(),
                title: "Should not appear".into(),
            },
            &mut state,
        );

        assert_eq!(state.sessions[0].title, "Original");
    }

    // T-TUI-TOOL-01: rich tool events update one event-ordered card in place.
    #[test]
    fn test_apply_tool_start_and_result_preserve_details() {
        let mut state = AppState::new();
        state.is_streaming = true;
        state.messages.push(ChatMessage {
            role: MessageRole::Assistant,
            content: String::new(),
            timestamp: Utc::now(),
            is_streaming: true,
            is_cancelled: false,
            reasoning_content: String::new(),
            reasoning_complete: false,
            tool_calls: Vec::new(),
            segments: Vec::new(),
            attachments: Vec::new(),
        });

        apply_chat_event(
            ChatEvent::ToolCallStarted {
                tool_call_id: "call-search-1".into(),
                name: "WebSearch".into(),
                input_preview: r#"{"query":"ratatui"}"#.into(),
                agent_name: "chat-turn".into(),
            },
            &mut state,
        );

        apply_chat_event(
            ChatEvent::ToolCallCompleted {
                tool_call_id: "call-search-1".into(),
                name: "WebSearch".into(),
                success: true,
                duration_ms: 120,
                input_preview: r#"{"query":"ratatui"}"#.into(),
                result_preview: "3 results".into(),
                agent_name: "chat-turn".into(),
                url_meta: Some("https://example.com".into()),
                metadata: Some(serde_json::json!({"result_count": 3})),
            },
            &mut state,
        );

        let last = state.messages.last().unwrap();
        assert_eq!(last.tool_calls.len(), 1);
        assert_eq!(last.tool_calls[0].name, "WebSearch");
        assert_eq!(last.tool_calls[0].status, ToolCallStatus::Succeeded);
        assert_eq!(last.tool_calls[0].duration_ms, Some(120));
        assert_eq!(last.tool_calls[0].result_preview, "3 results");
        assert_eq!(last.segments.len(), 1);
        let StreamSegment::ToolCall(tool_index) = &last.segments[0] else {
            panic!("expected tool call segment");
        };
        assert_eq!(*tool_index, 0);
        let tool = &last.tool_calls[*tool_index];
        assert_eq!(tool.status, ToolCallStatus::Succeeded);
        assert_eq!(tool.input_preview, r#"{"query":"ratatui"}"#);
    }

    // T-TUI-TOOL-02: Multiple tool calls accumulate.
    #[test]
    fn test_apply_multiple_tool_calls() {
        let mut state = AppState::new();
        state.is_streaming = true;
        state.messages.push(ChatMessage {
            role: MessageRole::Assistant,
            content: String::new(),
            timestamp: Utc::now(),
            is_streaming: true,
            is_cancelled: false,
            reasoning_content: String::new(),
            reasoning_complete: false,
            tool_calls: Vec::new(),
            segments: Vec::new(),
            attachments: Vec::new(),
        });

        apply_chat_event(
            ChatEvent::ToolCallCompleted {
                tool_call_id: "call-search-1".into(),
                name: "WebSearch".into(),
                success: true,
                duration_ms: 120,
                input_preview: String::new(),
                result_preview: "results".into(),
                agent_name: "chat-turn".into(),
                url_meta: None,
                metadata: None,
            },
            &mut state,
        );
        apply_chat_event(
            ChatEvent::ToolCallCompleted {
                tool_call_id: "call-shell-1".into(),
                name: "ShellExec".into(),
                success: false,
                duration_ms: 50,
                input_preview: r#"{"command":"false"}"#.into(),
                result_preview: "exit code 1".into(),
                agent_name: "chat-turn".into(),
                url_meta: None,
                metadata: None,
            },
            &mut state,
        );

        let last = state.messages.last().unwrap();
        assert_eq!(last.tool_calls.len(), 2);
        assert_eq!(last.tool_calls[0].name, "WebSearch");
        assert_eq!(last.tool_calls[1].name, "ShellExec");
        assert_eq!(last.tool_calls[1].status, ToolCallStatus::Failed);
    }

    #[test]
    fn test_final_tool_snapshot_recovers_event_missing_from_stream() {
        let mut state = AppState::new();
        state.is_streaming = true;
        state.messages.push(ChatMessage {
            role: MessageRole::Assistant,
            content: String::new(),
            timestamp: Utc::now(),
            is_streaming: true,
            is_cancelled: false,
            reasoning_content: String::new(),
            reasoning_complete: false,
            tool_calls: Vec::new(),
            segments: Vec::new(),
            attachments: Vec::new(),
        });
        apply_chat_event(
            ChatEvent::ToolCallCompleted {
                tool_call_id: "call-1".into(),
                name: "ShellExec".into(),
                success: true,
                duration_ms: 10,
                input_preview: "git init".into(),
                result_preview: "initialized".into(),
                agent_name: "chat-turn".into(),
                url_meta: None,
                metadata: None,
            },
            &mut state,
        );

        apply_chat_event(
            ChatEvent::ToolCallsSnapshot {
                calls: vec![
                    y_service::ToolCallRecord {
                        tool_call_id: "call-1".into(),
                        name: "ShellExec".into(),
                        arguments: r#"{"command":"git init"}"#.into(),
                        success: true,
                        duration_ms: 10,
                        result_content: "initialized".into(),
                        url_meta: None,
                        metadata: None,
                    },
                    y_service::ToolCallRecord {
                        tool_call_id: "call-2".into(),
                        name: "FileWrite".into(),
                        arguments: r#"{"path":".gitignore"}"#.into(),
                        success: true,
                        duration_ms: 4,
                        result_content: "written".into(),
                        url_meta: None,
                        metadata: None,
                    },
                ],
            },
            &mut state,
        );

        let tools = &state.messages.last().unwrap().tool_calls;
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].tool_call_id, "call-1");
        assert_eq!(tools[1].tool_call_id, "call-2");
        assert_eq!(tools[1].name, "FileWrite");
        assert_eq!(state.messages.last().unwrap().segments.len(), 2);
    }

    #[test]
    fn test_same_name_tool_results_complete_by_tool_call_id() {
        let mut state = AppState::new();
        state.is_streaming = true;
        state.messages.push(ChatMessage {
            role: MessageRole::Assistant,
            content: String::new(),
            timestamp: Utc::now(),
            is_streaming: true,
            is_cancelled: false,
            reasoning_content: String::new(),
            reasoning_complete: false,
            tool_calls: Vec::new(),
            segments: Vec::new(),
            attachments: Vec::new(),
        });

        for tool_call_id in ["call-a", "call-b"] {
            apply_chat_event(
                ChatEvent::ToolCallStarted {
                    tool_call_id: tool_call_id.into(),
                    name: "FileRead".into(),
                    input_preview: format!(r#"{{"path":"{tool_call_id}"}}"#),
                    agent_name: "chat-turn".into(),
                },
                &mut state,
            );
        }
        apply_chat_event(
            ChatEvent::ToolCallCompleted {
                tool_call_id: "call-a".into(),
                name: "FileRead".into(),
                success: true,
                duration_ms: 3,
                input_preview: r#"{"path":"call-a"}"#.into(),
                result_preview: "first result".into(),
                agent_name: "chat-turn".into(),
                url_meta: None,
                metadata: None,
            },
            &mut state,
        );

        let tools = &state.messages.last().unwrap().tool_calls;
        assert_eq!(tools[0].tool_call_id, "call-a");
        assert_eq!(tools[0].status, ToolCallStatus::Succeeded);
        assert_eq!(tools[1].tool_call_id, "call-b");
        assert_eq!(tools[1].status, ToolCallStatus::Running);
    }

    // T-TUI-REASON-01: StreamReasoningDelta accumulates reasoning content.
    #[test]
    fn test_apply_stream_reasoning_delta() {
        let mut state = AppState::new();
        state.is_streaming = true;
        state.messages.push(ChatMessage {
            role: MessageRole::Assistant,
            content: String::new(),
            timestamp: Utc::now(),
            is_streaming: true,
            is_cancelled: false,
            reasoning_content: String::new(),
            reasoning_complete: false,
            tool_calls: Vec::new(),
            segments: Vec::new(),
            attachments: Vec::new(),
        });

        apply_chat_event(
            ChatEvent::StreamReasoningDelta {
                content: "Let me think".into(),
            },
            &mut state,
        );
        apply_chat_event(
            ChatEvent::StreamReasoningDelta {
                content: " about this...".into(),
            },
            &mut state,
        );

        let last = state.messages.last().unwrap();
        assert_eq!(last.reasoning_content, "Let me think about this...");
        assert!(!last.reasoning_complete);
    }

    // T-TUI-REASON-03: reasoning remains in event order around tool calls.
    #[test]
    fn test_reasoning_segments_preserve_tool_timeline_order() {
        let mut state = AppState::new();
        state.is_streaming = true;
        state.messages.push(ChatMessage {
            role: MessageRole::Assistant,
            content: String::new(),
            timestamp: Utc::now(),
            is_streaming: true,
            is_cancelled: false,
            reasoning_content: String::new(),
            reasoning_complete: false,
            tool_calls: Vec::new(),
            segments: Vec::new(),
            attachments: Vec::new(),
        });

        apply_chat_event(
            ChatEvent::StreamReasoningDelta {
                content: "Inspect the file".into(),
            },
            &mut state,
        );
        apply_chat_event(
            ChatEvent::ToolCallStarted {
                tool_call_id: "call-read".into(),
                name: "FileRead".into(),
                input_preview: r#"{"path":"src/lib.rs"}"#.into(),
                agent_name: "chat-turn".into(),
            },
            &mut state,
        );
        apply_chat_event(
            ChatEvent::StreamReasoningDelta {
                content: "Now verify the result".into(),
            },
            &mut state,
        );
        apply_chat_event(
            ChatEvent::StreamDelta {
                content: "The result is valid.".into(),
            },
            &mut state,
        );

        let segments = &state.messages.last().unwrap().segments;
        assert_eq!(segments.len(), 4);
        assert!(matches!(
            &segments[0],
            StreamSegment::Reasoning { content, is_complete: true }
                if content == "Inspect the file"
        ));
        assert!(matches!(segments[1], StreamSegment::ToolCall(0)));
        assert!(matches!(
            &segments[2],
            StreamSegment::Reasoning { content, is_complete: true }
                if content == "Now verify the result"
        ));
        assert!(matches!(
            &segments[3],
            StreamSegment::Text(content) if content == "The result is valid."
        ));
    }

    // T-TUI-REASON-02: reasoning_complete set on Response.
    #[test]
    fn test_reasoning_complete_on_response() {
        let mut state = AppState::new();
        state.is_streaming = true;
        state.messages.push(ChatMessage {
            role: MessageRole::Assistant,
            content: String::new(),
            timestamp: Utc::now(),
            is_streaming: true,
            is_cancelled: false,
            reasoning_content: "some reasoning".into(),
            reasoning_complete: false,
            tool_calls: Vec::new(),
            segments: Vec::new(),
            attachments: Vec::new(),
        });

        apply_chat_event(
            ChatEvent::Response {
                content: "Answer".into(),
                model: "test".into(),
                input_tokens: 10,
                output_tokens: 5,
                last_input_tokens: 10,
                context_window: 128_000,
                cost_usd: 0.0,
            },
            &mut state,
        );

        let last = state.messages.last().unwrap();
        assert!(last.reasoning_complete);
        assert_eq!(last.reasoning_content, "some reasoning");
    }

    #[test]
    fn test_classify_input_accepts_image_only_turn() {
        assert_eq!(
            classify_input_with_attachments("", false, true),
            InputIntent::NewTurn(String::new())
        );
    }

    fn sample_image_attachment() -> y_core::types::Attachment {
        y_core::types::Attachment {
            id: "att-1".to_string(),
            filename: "clipboard.png".to_string(),
            mime_type: "image/png".to_string(),
            size: 128,
            sha256: None,
            width: Some(640),
            height: Some(480),
            source: y_core::types::AttachmentSource::InlineBase64 {
                base64_data: "aGVsbG8=".to_string(),
            },
        }
    }

    #[test]
    fn test_display_content_keeps_attachment_marker_with_text() {
        let attachments = vec![sample_image_attachment()];
        assert_eq!(
            user_display_content("what is this?", &attachments),
            "what is this?\n[Image: clipboard.png]"
        );
    }

    #[test]
    fn test_display_content_marker_only_for_image_only_turn() {
        let attachments = vec![sample_image_attachment()];
        assert_eq!(
            user_display_content("", &attachments),
            "[Image: clipboard.png]"
        );
    }

    #[test]
    fn test_display_content_plain_text_without_attachments() {
        assert_eq!(user_display_content("hello", &[]), "hello");
    }

    #[test]
    fn test_content_with_attachment_markers_restores_from_metadata() {
        let metadata = serde_json::json!({
            "attachments": [sample_image_attachment()]
        });
        assert_eq!(
            content_with_attachment_markers("look at this", &metadata),
            "look at this\n[Image: clipboard.png]"
        );
    }

    #[test]
    fn test_content_with_attachment_markers_ignores_messages_without_attachments() {
        let metadata = serde_json::json!({ "skills": ["demo"] });
        assert_eq!(
            content_with_attachment_markers("plain text", &metadata),
            "plain text"
        );
        assert_eq!(
            content_with_attachment_markers("plain text", &serde_json::Value::Null),
            "plain text"
        );
    }
}
