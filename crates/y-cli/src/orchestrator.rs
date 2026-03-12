//! Chat turn orchestrator — thin delegation to `y-service::ChatService`.
//!
//! Re-exports service types so existing `y-cli` modules can import from
//! `crate::orchestrator` without changes.

// Re-export all service types for backward compatibility within y-cli.
pub use y_service::chat::{ChatService, TurnError, TurnInput, TurnResult};

use crate::wire::AppServices;

/// Execute a single chat turn — thin wrapper that delegates to `ChatService`.
pub async fn execute_turn(
    services: &AppServices,
    input: &TurnInput<'_>,
) -> Result<TurnResult, TurnError> {
    ChatService::execute_turn(services, input).await
}
