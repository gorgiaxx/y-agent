//! Observability command handlers -- live system state snapshots.

use tauri::State;

use y_service::ObservabilityService;

use crate::state::AppState;

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Return a point-in-time snapshot of the entire system state.
///
/// Includes provider pool (freeze, concurrency, metrics), agent pool
/// (instance lifecycle, resource usage), and scheduler queue.
/// The frontend polls this periodically while the observability panel is open.
#[tauri::command]
pub async fn observability_snapshot(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let snap = ObservabilityService::snapshot(&state.container).await;
    serde_json::to_value(&snap).map_err(|e| e.to_string())
}
