//! Diagnostics command handlers -- query historical traces and observations.

use tauri::State;

use y_service::DiagnosticsService;

use crate::state::AppState;

// Re-export the service-layer type so the Tauri command signature is stable.
pub use y_service::HistoricalEntry;

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Fetch historical diagnostics for a session, ordered by time.
///
/// Returns a flat list of entries reconstructed from stored Traces and
/// Observations.  Limited to the N most recent traces (default 50) so the
/// panel does not grow unbounded for long-lived sessions.
#[tauri::command]
pub async fn diagnostics_get_by_session(
    state: State<'_, AppState>,
    session_id: String,
    limit: Option<usize>,
) -> Result<Vec<HistoricalEntry>, String> {
    DiagnosticsService::get_session_history_including_descendants(
        &state.container,
        &session_id,
        limit.unwrap_or(50),
    )
    .await
}

/// Fetch all subagent traces regardless of session, ordered by time.
///
/// Returns entries for traces whose name starts with `subagent:`, covering
/// both session-scoped subagents (title-generator, pruning-summarizer) and
/// session-independent ones (skill-ingestion, security-check).
#[tauri::command]
pub async fn diagnostics_get_subagent_history(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Vec<HistoricalEntry>, String> {
    let store = state.container.diagnostics.store();
    DiagnosticsService::get_subagent_history(store, limit.unwrap_or(50)).await
}

/// Delete stored diagnostics for a session and its descendant sessions.
#[tauri::command]
pub async fn diagnostics_clear_by_session(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<u64, String> {
    DiagnosticsService::clear_session_history_including_descendants(&state.container, &session_id)
        .await
}

/// Delete all stored diagnostics history.
#[tauri::command]
pub async fn diagnostics_clear_all(state: State<'_, AppState>) -> Result<u64, String> {
    DiagnosticsService::clear_all_history(state.container.diagnostics.store()).await
}
