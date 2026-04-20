//! `EventSink` trait -- presentation-layer abstraction for emitting chat events.
//!
//! The shared `chat_worker::spawn_llm_worker` function uses this trait to emit
//! events without knowing whether the presentation layer is SSE (y-web),
//! Tauri (y-gui), or something else.

use crate::chat::TurnEvent;

/// Abstraction over the event emission mechanism.
///
/// Each presentation layer (y-web SSE, y-gui Tauri, future CLI, etc.)
/// implements this trait to translate generic chat lifecycle events into
/// its own transport format.
pub trait EventSink: Send + Sync + 'static {
    /// Emitted when the LLM turn has started (`run_id` -> `session_id` mapping).
    fn emit_started(&self, run_id: &str, session_id: &str);

    /// Emitted for each real-time progress event during the turn.
    fn emit_progress(&self, run_id: &str, event: &TurnEvent);

    /// Emitted when the LLM requests user input (`AskUser` dialog).
    fn emit_ask_user(
        &self,
        run_id: &str,
        session_id: &str,
        interaction_id: &str,
        questions: &serde_json::Value,
    );

    /// Emitted when a tool requests permission approval from the user.
    fn emit_permission_request(
        &self,
        run_id: &str,
        session_id: &str,
        request_id: &str,
        tool_name: &str,
        action_description: &str,
        reason: &str,
        content_preview: Option<&str>,
    );

    /// Emitted when the turn completes successfully.
    fn emit_complete(&self, payload: &serde_json::Value);

    /// Emitted when the turn fails with an error.
    fn emit_error(&self, run_id: &str, session_id: &str, error: &str);

    /// Emitted when a session title is generated or updated.
    fn emit_title_updated(&self, session_id: &str, title: &str);
}
