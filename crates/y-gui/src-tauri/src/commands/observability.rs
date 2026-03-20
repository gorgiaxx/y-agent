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

/// Return a snapshot with historical metrics filtered by time range.
///
/// Provider metrics (requests, tokens, cost, errors) come from the persistent
/// store, aggregated over the specified time window. Live data (concurrency,
/// freeze status, agent pool) is always real-time.
#[tauri::command]
pub async fn observability_history(
    state: State<'_, AppState>,
    since: Option<String>,
    until: Option<String>,
) -> Result<serde_json::Value, String> {
    let since_dt = since
        .as_deref()
        .map(|s| {
            chrono::DateTime::parse_from_rfc3339(s)
                .map(|dt| dt.with_timezone(&chrono::Utc))
                .map_err(|e| format!("invalid 'since' timestamp: {e}"))
        })
        .transpose()?;
    let until_dt = until
        .as_deref()
        .map(|s| {
            chrono::DateTime::parse_from_rfc3339(s)
                .map(|dt| dt.with_timezone(&chrono::Utc))
                .map_err(|e| format!("invalid 'until' timestamp: {e}"))
        })
        .transpose()?;

    let snap =
        ObservabilityService::snapshot_with_history(&state.container, since_dt, until_dt).await;
    serde_json::to_value(&snap).map_err(|e| e.to_string())
}
