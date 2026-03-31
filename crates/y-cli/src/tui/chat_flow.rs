use std::sync::Arc;

use chrono::Utc;
use tokio::sync::mpsc;
use tracing::warn;

use y_core::session::{CreateSessionOptions, SessionType};
use y_core::types::{Message, Role, SessionId};

use crate::orchestrator::{self, TurnInput};
use crate::tui::state::{AppState, ChatMessage, MessageRole, SessionListItem};
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
    },
    /// A tool call was executed during the LLM turn.
    ToolCallExecuted {
        name: String,
        success: bool,
        duration_ms: u64,
    },
    /// Incremental text delta from the LLM stream.
    StreamDelta { content: String },
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
}

/// Submit a user message: adds to state, persists, starts async LLM call.
///
/// Returns a receiver for `ChatEvent`s. The caller should poll this
/// in the main event loop and apply events to state.
pub fn submit_message(
    input: &str,
    state: &mut AppState,
    services: &Arc<AppServices>,
) -> Option<mpsc::Receiver<ChatEvent>> {
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

    // Determine if title summarization should be triggered.
    let title_interval = services.session_manager.config().title_summarize_interval;
    let should_generate_title = title_interval > 0
        && user_msg_count > 0
        && (user_msg_count == 1 || user_msg_count.is_multiple_of(title_interval));

    // Mark state as streaming — add placeholder assistant message.
    state.is_streaming = true;
    state.messages.push(ChatMessage {
        role: MessageRole::Assistant,
        content: String::new(),
        timestamp: Utc::now(),
        is_streaming: true,
        is_cancelled: false,
    });

    // Spawn async task for LLM call.
    let (tx, rx) = mpsc::channel(16);

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

        // Parse session UUID for diagnostics.
        let session_uuid =
            uuid::Uuid::parse_str(&session_id_str).unwrap_or_else(|_| uuid::Uuid::new_v4());

        // Delegate to the shared orchestrator.
        let turn_input = TurnInput {
            user_input: &trimmed_owned,
            session_id: session_id.clone(),
            session_uuid,
            history: &history,
            turn_number: user_msg_count,
            provider_id: None,
            knowledge_collections: vec![],
        };

        // Set up a progress channel to receive streaming deltas.
        let (progress_tx, mut progress_rx) = tokio::sync::mpsc::unbounded_channel();

        // Spawn a sub-task to forward StreamDelta events from the progress
        // channel to the TUI event channel.
        let tx_stream = tx.clone();
        let progress_forwarder = tokio::spawn(async move {
            while let Some(event) = progress_rx.recv().await {
                if let y_service::TurnEvent::StreamDelta { content } = event {
                    let _ = tx_stream.send(ChatEvent::StreamDelta { content }).await;
                }
            }
        });

        match orchestrator::execute_turn_streaming(&services, &turn_input, progress_tx).await {
            Ok(result) => {
                // Wait for the progress forwarder to finish before emitting
                // the final Response so all deltas arrive first.
                let _ = progress_forwarder.await;
                // Emit tool call events for TUI display.
                for tc in &result.tool_calls_executed {
                    let _ = tx
                        .send(ChatEvent::ToolCallExecuted {
                            name: tc.name.clone(),
                            success: tc.success,
                            duration_ms: tc.duration_ms,
                        })
                        .await;
                }

                let _ = tx
                    .send(ChatEvent::Response {
                        content: result.content,
                        model: result.model,
                        input_tokens: result.input_tokens,
                        output_tokens: result.output_tokens,
                        last_input_tokens: result.last_input_tokens,
                        context_window: result.context_window,
                    })
                    .await;
            }
            Err(e) => {
                let _ = tx.send(ChatEvent::Error(format!("{e}"))).await;
            }
        }

        // Trigger title summarization if interval reached.
        if should_generate_title {
            let session_id = SessionId::from_string(session_id_str.clone());
            // Re-read the full history for title generation context.
            match services.session_manager.read_transcript(&session_id).await {
                Ok(transcript) => {
                    match services
                        .session_manager
                        .generate_title(&*services.agent_delegator, &session_id, &transcript)
                        .await
                    {
                        Ok(title) => {
                            let _ = tx
                                .send(ChatEvent::TitleUpdated {
                                    session_id: session_id_str,
                                    title,
                                })
                                .await;
                        }
                        Err(e) => {
                            warn!(error = %e, "title generation failed");
                        }
                    }
                }
                Err(e) => {
                    warn!(error = %e, "failed to read transcript for title generation");
                }
            }
        }
    });

    Some(rx)
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
        } => {
            // Update the last (streaming) assistant message.
            if let Some(last) = state.messages.last_mut() {
                if last.role == MessageRole::Assistant && last.is_streaming {
                    last.content = content;
                    last.is_streaming = false;
                }
            }
            state.is_streaming = false;

            // Update status bar data.
            state.status_model = model;
            state.status_tokens = format!("{input_tokens}↑ {output_tokens}↓");

            // Track cumulative tokens and context window for usage display.
            state.cumulative_input_tokens += input_tokens;
            state.cumulative_output_tokens += output_tokens;
            state.last_input_tokens = last_input_tokens;
            if context_window > 0 {
                state.context_window = context_window;
            }
        }
        ChatEvent::ToolCallExecuted {
            name,
            success,
            duration_ms,
        } => {
            // Append tool call info to the streaming assistant message content.
            if let Some(last) = state.messages.last_mut() {
                if last.role == MessageRole::Assistant && last.is_streaming {
                    let status = if success { "✓" } else { "✗" };
                    let info = format!("[tool: {name}] {status} ({duration_ms}ms)\n");
                    last.content.push_str(&info);
                }
            }
        }
        ChatEvent::StreamDelta { content } => {
            // Append incremental text to the streaming assistant message.
            if let Some(last) = state.messages.last_mut() {
                if last.role == MessageRole::Assistant && last.is_streaming {
                    last.content.push_str(&content);
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

            // Also emit a transient warning toast.
            state.push_toast(msg, crate::tui::state::ToastLevel::Warning);
        }
        ChatEvent::TitleUpdated { session_id, title } => {
            // Update matching session entry in the sidebar list.
            if let Some(session) = state.sessions.iter_mut().find(|s| s.id == session_id) {
                session.title = title;
            }
        }
        ChatEvent::SessionCreated {
            id,
            title,
            updated_at,
        } => {
            // Insert newly created session at the top of the sidebar list.
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
            state.sync_selected_session_index();
        }
    }
}

/// Cancel the currently streaming response (if any).
pub fn cancel_streaming(state: &mut AppState) {
    if state.is_streaming {
        state.is_streaming = false;
        if let Some(last) = state.messages.last_mut() {
            if last.role == MessageRole::Assistant && last.is_streaming {
                last.is_streaming = false;
                last.is_cancelled = true;
                if last.content.is_empty() {
                    last.content = "(cancelled)".to_string();
                }
            }
        }
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
        });

        apply_chat_event(
            ChatEvent::Response {
                content: "Hello!".into(),
                model: "gpt-4".into(),
                input_tokens: 10,
                output_tokens: 5,
                last_input_tokens: 10,
                context_window: 128_000,
            },
            &mut state,
        );

        assert!(!state.is_streaming);
        let last = state.messages.last().unwrap();
        assert_eq!(last.content, "Hello!");
        assert!(!last.is_streaming);
        assert_eq!(state.status_model, "gpt-4");
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
        });

        apply_chat_event(ChatEvent::Error("connection refused".into()), &mut state);

        assert!(!state.is_streaming);
        let last = state.messages.last().unwrap();
        assert!(last.content.contains("connection refused"));
        assert!(last.is_cancelled);
    }

    // T-TUI-05-03: Cancel streaming marks message.
    #[test]
    fn test_cancel_streaming() {
        let mut state = AppState::default();
        state.is_streaming = true;
        state.messages.push(ChatMessage {
            role: MessageRole::Assistant,
            content: "partial...".into(),
            timestamp: Utc::now(),
            is_streaming: true,
            is_cancelled: false,
        });

        cancel_streaming(&mut state);
        assert!(!state.is_streaming);
        assert!(state.messages.last().unwrap().is_cancelled);
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

    // T-TUI-TOOL-01: ToolCallExecuted events append to streaming message.
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
        assert!(last.content.contains("WebSearch"));
        assert!(last.content.contains("✓"));
        assert!(last.content.contains("120ms"));
    }
}
