//! Chat turn orchestrator -- thin delegation to `y-service::ChatService`.
//!
//! Re-exports service types so existing `y-cli` modules can import from
//! `crate::orchestrator` without changes.

// Re-export all service types for backward compatibility within y-cli.
pub use y_service::chat::{
    ChatService, TurnError, TurnEventSender, TurnInput, TurnResult,
};

use crate::wire::AppServices;

/// Execute a single chat turn (no streaming progress).
pub async fn execute_turn(
    services: &AppServices,
    input: &TurnInput<'_>,
) -> Result<TurnResult, TurnError> {
    ChatService::execute_turn(services, input).await
}

/// Execute a single chat turn with a progress channel for streaming.
///
/// `TurnEvent::StreamDelta` events are emitted so callers can display
/// incremental text as it arrives from the LLM.
pub async fn execute_turn_streaming(
    services: &AppServices,
    input: &TurnInput<'_>,
    progress: TurnEventSender,
) -> Result<TurnResult, TurnError> {
    ChatService::execute_turn_with_progress(services, input, progress, None).await
}
