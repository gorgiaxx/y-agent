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
    let store = state.container.diagnostics.store();
    DiagnosticsService::get_session_history(store, &session_id, limit.unwrap_or(50)).await
}
