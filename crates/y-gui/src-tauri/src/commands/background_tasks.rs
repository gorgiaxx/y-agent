//! Background task command handlers.

use tauri::State;

use y_service::{
    BackgroundTaskInfo, BackgroundTaskPollRequest, BackgroundTaskService, BackgroundTaskSnapshot,
    BackgroundTaskWriteRequest,
};

use crate::state::AppState;

/// List runtime-managed background tasks.
#[tauri::command]
pub async fn background_task_list(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<Vec<BackgroundTaskInfo>, String> {
    BackgroundTaskService::list(&state.container, session_id)
        .await
        .map_err(|e| e.to_string())
}

/// Poll incremental output for a background task.
#[tauri::command]
pub async fn background_task_poll(
    state: State<'_, AppState>,
    session_id: String,
    process_id: String,
    yield_time_ms: Option<u64>,
    max_output_bytes: Option<usize>,
) -> Result<BackgroundTaskSnapshot, String> {
    BackgroundTaskService::poll(
        &state.container,
        BackgroundTaskPollRequest {
            session_id,
            process_id,
            yield_time_ms,
            max_output_bytes,
        },
    )
    .await
    .map_err(|e| e.to_string())
}

/// Write stdin to a background task and drain any immediate output.
#[tauri::command]
pub async fn background_task_write(
    state: State<'_, AppState>,
    session_id: String,
    process_id: String,
    input: String,
    yield_time_ms: Option<u64>,
    max_output_bytes: Option<usize>,
) -> Result<BackgroundTaskSnapshot, String> {
    BackgroundTaskService::write(
        &state.container,
        BackgroundTaskWriteRequest {
            session_id,
            process_id,
            input,
            yield_time_ms,
            max_output_bytes,
        },
    )
    .await
    .map_err(|e| e.to_string())
}

/// Terminate a background task and return the final output snapshot.
#[tauri::command]
pub async fn background_task_kill(
    state: State<'_, AppState>,
    session_id: String,
    process_id: String,
    yield_time_ms: Option<u64>,
    max_output_bytes: Option<usize>,
) -> Result<BackgroundTaskSnapshot, String> {
    BackgroundTaskService::kill(
        &state.container,
        BackgroundTaskPollRequest {
            session_id,
            process_id,
            yield_time_ms,
            max_output_bytes,
        },
    )
    .await
    .map_err(|e| e.to_string())
}
