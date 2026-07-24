use std::sync::Arc;

use chrono::Utc;
use tokio::sync::mpsc;
use tracing::warn;

use y_core::session::{CreateSessionOptions, SessionType};
use y_core::types::{Message, Role, SessionId};

use crate::orchestrator::{self, ChatService, TurnCancellationToken, TurnError, TurnInput};
use crate::tui::state::{
    AppState, ChatMessage, MessageRole, SessionListItem, StreamSegment, ToolCallInfo, TurnMode,
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
    /// A tool call was executed during the LLM turn.
    ToolCallExecuted {
        name: String,
        success: bool,
        duration_ms: u64,
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

    // Build conversation history as y_core Messages for the LLM.
    let history: Vec<Message> = state
        .messages
        .iter()
        .map(|m| Message {
            message_id: y_core::types::generate_message_id(),
            role: match m.role {
                MessageRole::User => Role::User,
                MessageRole::Assistant => Role::Assistant,
                MessageRole::System => Role::System,
                MessageRole::Tool => Role::Tool,
            },
            content: m.content.clone(),
            tool_call_id: None,
            tool_calls: vec![],
            timestamp: y_core::types::now(),
            metadata: serde_json::Value::Null,
        })
        .collect();

    // Persist user message to session.
    let session_id_opt = state.current_session_id.clone();
    let user_msg = Message {
        message_id: y_core::types::generate_message_id(),
        role: Role::User,
        content: trimmed.to_string(),
        tool_call_id: None,
        tool_calls: vec![],
        timestamp: y_core::types::now(),
        metadata: serde_json::Value::Null,
    };

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
        // Lazy session creation: if no current session, create one now.
        let session_id_str = if let Some(sid) = session_id_opt {
            sid
        } else {
            match services
                .session_manager
                .create_session(CreateSessionOptions {
                    parent_id: None,
                    session_type: SessionType::Main,
                    agent_id: None,
                    title: Some("New Chat".into()),
                })
                .await
            {
                Ok(node) => {
                    let sid = node.id.to_string();
                    let _ = tx
                        .send(ChatEvent::SessionCreated {
                            id: sid.clone(),
                            title: "New Chat".into(),
                            updated_at: node.updated_at,
                        })
                        .await;
                    sid
                }
                Err(e) => {
                    warn!(error = %e, "failed to create session lazily");
                    let _ = tx
                        .send(ChatEvent::Error(format!("Failed to create session: {e}")))
                        .await;
                    return;
                }
            }
        };

        // Persist user message to session transcript.
        let session_id = SessionId::from_string(session_id_str.clone());
        let _ = services
            .session_manager
            .append_message(&session_id, &user_msg)
            .await;

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

        // Parse session UUID for diagnostics.
        let session_uuid =
            uuid::Uuid::parse_str(&session_id_str).unwrap_or_else(|_| uuid::Uuid::new_v4());
        let working_directory = std::env::current_dir()
            .ok()
            .map(|path| path.to_string_lossy().to_string());

        // Delegate to the shared orchestrator.
        let turn_input = TurnInput {
            user_input: &trimmed_owned,
            session_id: session_id.clone(),
            session_uuid,
            history: &history,
            turn_number: user_msg_count,
            provider_id: selected_provider_id,
            request_mode: y_core::provider::RequestMode::TextChat,
            working_directory,
            knowledge_collections: vec![],
            skills: vec![],
            thinking: None,
            plan_mode: turn_mode.plan_mode().map(str::to_string),
            operation_mode: y_service::chat_types::OperationMode::Default,
            agent_name: "chat-turn".to_string(),
            toolcall_enabled: true,
            preferred_models: vec![],
            provider_tags: vec![],
            temperature: None,
            max_completion_tokens: None,
            max_iterations: None,
            max_tool_calls: None,
            trust_tier: None,
            agent_allowed_tools: vec![],
            prune_tool_history: false,
            mcp_mode: None,
            mcp_servers: vec![],
            image_generation_options: None,
            pre_turn_message_count: None,
        };

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
                    y_service::TurnEvent::ToolResult {
                        name,
                        success,
                        duration_ms,
                        ..
                    } => {
                        let _ = tx_stream
                            .send(ChatEvent::ToolCallExecuted {
                                name,
                                success,
                                duration_ms,
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
                    if last.content.is_empty() && last.segments.is_empty() {
                        last.content = content;
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
        ChatEvent::ToolCallExecuted {
            name,
            success,
            duration_ms,
        } => {
            // Store structured tool call info for card rendering.
            if let Some(last) = state.messages.last_mut() {
                if last.role == MessageRole::Assistant && last.is_streaming {
                    let tc = ToolCallInfo {
                        name,
                        success,
                        duration_ms,
                    };
                    last.tool_calls.push(tc.clone());
                    last.segments.push(StreamSegment::ToolCall(tc));
                }
            }
        }
        ChatEvent::StreamDelta { content } => {
            // Append incremental text to the streaming assistant message.
            if let Some(last) = state.messages.last_mut() {
                if last.role == MessageRole::Assistant && last.is_streaming {
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
            // Append incremental reasoning text to the streaming assistant message.
            if let Some(last) = state.messages.last_mut() {
                if last.role == MessageRole::Assistant && last.is_streaming {
                    last.reasoning_content.push_str(&content);
                }
            }
        }
        ChatEvent::Error(msg) => {
            // Replace the streaming message with error.
            if let Some(last) = state.messages.last_mut() {
                if last.role == MessageRole::Assistant && last.is_streaming {
                    last.content = format!("Error: {msg}");
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
                    last.is_streaming = false;
                    last.is_cancelled = true;
                    last.reasoning_complete = true;
                    if last.content.is_empty() {
                        last.content = "(cancelled)".to_string();
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

    // T-TUI-TOOL-01: ToolCallExecuted events stored as structured data.
    #[test]
    fn test_apply_tool_call_executed() {
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
            ChatEvent::ToolCallExecuted {
                name: "WebSearch".into(),
                success: true,
                duration_ms: 120,
            },
            &mut state,
        );

        let last = state.messages.last().unwrap();
        assert_eq!(last.tool_calls.len(), 1);
        assert_eq!(last.tool_calls[0].name, "WebSearch");
        assert!(last.tool_calls[0].success);
        assert_eq!(last.tool_calls[0].duration_ms, 120);
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
            ChatEvent::ToolCallExecuted {
                name: "WebSearch".into(),
                success: true,
                duration_ms: 120,
            },
            &mut state,
        );
        apply_chat_event(
            ChatEvent::ToolCallExecuted {
                name: "ShellExec".into(),
                success: false,
                duration_ms: 50,
            },
            &mut state,
        );

        let last = state.messages.last().unwrap();
        assert_eq!(last.tool_calls.len(), 2);
        assert_eq!(last.tool_calls[0].name, "WebSearch");
        assert_eq!(last.tool_calls[1].name, "ShellExec");
        assert!(!last.tool_calls[1].success);
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
