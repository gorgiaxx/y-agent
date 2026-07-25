use std::sync::Arc;

use chrono::Utc;
use tokio::sync::mpsc;
use tracing::warn;

use y_core::session::{CreateSessionOptions, SessionType};
use y_core::types::SessionId;
use y_service::{PrepareTurnRequest, PreparedTurn};

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
    FollowUpInjected { id: String, text: String },
    /// The active service run acknowledged cancellation.
    Cancelled,
}

/// Presentation routing for submitted composer text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputIntent {
    Ignore,
    Command(String),
    NewTurn(String),
    FollowUp(String),
}

/// Classify composer text without performing service work.
pub fn classify_input(input: &str, is_streaming: bool) -> InputIntent {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return InputIntent::Ignore;
    }
    if let Some(command) = trimmed.strip_prefix('/') {
        return InputIntent::Command(command.to_string());
    }
    if is_streaming {
        InputIntent::FollowUp(trimmed.to_string())
    } else {
        InputIntent::NewTurn(trimmed.to_string())
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

/// Submit a user message: adds to state, persists, starts async LLM call.
///
/// Returns a receiver for `ChatEvent`s. The caller should poll this
/// in the main event loop and apply events to state.
pub fn submit_message(
    input: &str,
    state: &mut AppState,
    services: &Arc<AppServices>,
) -> Option<ActiveChat> {
    submit_message_with_mode(input, state.turn_mode, state, services)
}

/// Submit a user message with an optional service-owned orchestration mode.
pub fn submit_message_with_mode(
    input: &str,
    turn_mode: TurnMode,
    state: &mut AppState,
    services: &Arc<AppServices>,
) -> Option<ActiveChat> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Add user message to chat history.
    state.messages.push(ChatMessage {
        role: MessageRole::User,
        content: trimmed.to_string(),
        timestamp: Utc::now(),
        is_streaming: false,
        is_cancelled: false,
        reasoning_content: String::new(),
        reasoning_complete: false,
        tool_calls: Vec::new(),
        segments: Vec::new(),
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

        // Fire title generation concurrently with the turn. The title only
        // consumes user messages (the just-appended one is already persisted),
        // so it does not need to wait for the assistant reply.
        if should_generate_title {
            let title_services = Arc::clone(&services);
            let title_tx = tx.clone();
            let title_session_id = session_id.clone();
            let title_session_id_str = session_id_str.clone();
            tokio::spawn(async move {
                let has_manual_title = title_services
                    .session_manager
                    .get_session(&title_session_id)
                    .await
                    .map(|s| s.manual_title.is_some())
                    .unwrap_or(false);
                if has_manual_title {
                    return;
                }
                match title_services
                    .session_manager
                    .read_transcript(&title_session_id)
                    .await
                {
                    Ok(transcript) => {
                        match title_services
                            .session_manager
                            .generate_title(
                                &*title_services.agent_delegator,
                                &title_session_id,
                                &transcript,
                            )
                            .await
                        {
                            Ok(title) => {
                                let _ = title_tx
                                    .send(ChatEvent::TitleUpdated {
                                        session_id: title_session_id_str,
                                        title,
                                    })
                                    .await;
                            }
                            Err(e) => warn!(error = %e, "title generation failed"),
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "failed to read transcript for title generation");
                    }
                }
            });
        }

        let turn_input = prepared.as_turn_input();

        // Set up a progress channel to receive streaming deltas.
        let (progress_tx, mut progress_rx) = y_service::TurnEventSender::channel();

        // Spawn a sub-task to forward StreamDelta events from the progress
        // channel to the TUI event channel.
        let tx_stream = tx.clone();
        let progress_forwarder = tokio::spawn(async move {
            while let Some((event, _session_id)) = progress_rx.recv().await {
                match event {
                    y_service::TurnEvent::StreamDelta { content, .. } => {
                        let _ = tx_stream.send(ChatEvent::StreamDelta { content }).await;
                    }
                    y_service::TurnEvent::StreamReasoningDelta { content, .. } => {
                        let _ = tx_stream
                            .send(ChatEvent::StreamReasoningDelta { content })
                            .await;
                    }
                    y_service::TurnEvent::ToolStart {
                        tool_call_id,
                        name,
                        input_preview,
                        agent_name,
                    } => {
                        let _ = tx_stream
                            .send(ChatEvent::ToolCallStarted {
                                tool_call_id,
                                name,
                                input_preview,
                                agent_name,
                            })
                            .await;
                    }
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
                    } => {
                        let _ = tx_stream
                            .send(ChatEvent::ToolCallCompleted {
                                tool_call_id,
                                name,
                                success,
                                duration_ms,
                                input_preview,
                                result_preview,
                                agent_name,
                                url_meta,
                                metadata,
                            })
                            .await;
                    }
                    y_service::TurnEvent::FollowUpInjected { follow_up_id, text } => {
                        let _ = tx_stream
                            .send(ChatEvent::FollowUpInjected {
                                id: follow_up_id,
                                text,
                            })
                            .await;
                    }
                    _ => {}
                }
            }
        });

        ChatService::begin_follow_up_run(&services, &session_id);
        let result = orchestrator::execute_turn_streaming(
            &services,
            &turn_input,
            progress_tx,
            Some(task_cancellation),
        )
        .await;
        let _ = progress_forwarder.await;
        ChatService::finish_follow_up_run(&services, &session_id).await;

        match result {
            Ok(result) => {
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
            Err(e) => {
                let _ = tx.send(ChatEvent::Error(format!("{e}"))).await;
            }
        }
    });

    Some(ActiveChat {
        events: rx,
        cancellation,
    })
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
) -> Result<(PreparedTurn, Option<y_core::session::SessionNode>), String> {
    let (session_id, created_session) = if let Some(session_id) = session_id {
        (SessionId::from_string(session_id), None)
    } else {
        let session = create_workspace_session(services).await?;
        (session.id.clone(), Some(session))
    };
    let prepared = ChatService::prepare_turn(
        services,
        PrepareTurnRequest {
            session_id: Some(session_id),
            user_input,
            provider_id,
            request_mode: Some(y_core::provider::RequestMode::TextChat),
            plan_mode: turn_mode.plan_mode().map(str::to_string),
            operation_mode: Some(y_service::chat_types::OperationMode::Default),
            ..PrepareTurnRequest::default()
        },
    )
    .await
    .map_err(|error| error.to_string())?;
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

            // Also emit a transient warning toast.
            state.push_toast(msg, crate::tui::state::ToastLevel::Warning);
        }
        ChatEvent::FollowUpInjected { text, .. } => {
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
            });
        }
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
    async fn prepare_tui_turn_uses_shared_service_preparation() {
        let temp = tempfile::tempdir().unwrap();
        let mut config = y_service::ServiceConfig::default();
        config.storage = y_service::config_types::StorageConfig::in_memory();
        config.storage.transcript_dir = temp.path().join("transcripts");
        let services = AppServices::from_config(&config).await.unwrap();

        let (prepared, created_session) = prepare_tui_turn(
            &services,
            None,
            "edit the file".into(),
            None,
            TurnMode::Fast,
        )
        .await
        .unwrap();

        assert!(created_session.is_some());
        assert_eq!(prepared.history.last().unwrap().content, "edit the file");
        let managers = services.file_history_managers.read().await;
        let manager = managers.get(&prepared.session_id).unwrap();
        assert_eq!(manager.snapshots().len(), 1);
    }

    // T-TUI-05-01: User message appended to history.
    #[test]
    fn test_apply_chat_response() {
        let mut state = AppState::default();
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
        let mut state = AppState::default();
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
        let mut state = AppState::default();
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
        });

        cancel_streaming(&mut state);
        assert!(state.is_streaming);
        assert!(state.is_cancelling);
        assert!(!state.messages.last().unwrap().is_cancelled);
    }

    #[test]
    fn test_apply_chat_cancelled_preserves_partial_response() {
        let mut state = AppState::default();
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
        let mut state = AppState::default();
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
        });

        apply_chat_event(
            ChatEvent::FollowUpInjected {
                id: "follow-up-1".into(),
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

    #[test]
    fn test_final_response_does_not_duplicate_prior_follow_up_iterations() {
        let mut state = AppState::default();
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
        let mut state = AppState::default();
        state.sessions.push(SessionListItem {
            id: "session-1".into(),
            title: String::new(),
            updated_at: Utc::now(),
            message_count: 3,
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
        let mut state = AppState::default();
        state.sessions.push(SessionListItem {
            id: "session-1".into(),
            title: "Original".into(),
            updated_at: Utc::now(),
            message_count: 3,
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
        let mut state = AppState::default();
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
        let mut state = AppState::default();
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
    fn test_same_name_tool_results_complete_by_tool_call_id() {
        let mut state = AppState::default();
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
        let mut state = AppState::default();
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
        let mut state = AppState::default();
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
        let mut state = AppState::default();
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
}
