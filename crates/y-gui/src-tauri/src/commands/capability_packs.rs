//! Capability Pack presentation commands.

use std::path::Path;

use tauri::{AppHandle, Emitter, State};
use tokio_util::sync::CancellationToken;
use y_core::types::SessionId;
use y_service::capability_pack::{
    CapabilityPackActivationRevocationReceipt, CapabilityPackInspection,
    CapabilityPackInstallOptions, CapabilityPackInstallReceipt,
    CapabilityPackLiveActivationReceipt, CapabilityPackRemoveReceipt,
    CapabilityPackRollbackReceipt, CapabilityPackService, InstalledCapabilityPackSummary,
};
use y_service::{TurnEvent, WorkspaceService};

use crate::state::AppState;

#[tauri::command]
pub async fn capability_pack_list(
    state: State<'_, AppState>,
) -> Result<Vec<InstalledCapabilityPackSummary>, String> {
    CapabilityPackService::list_installed(&state.container)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn capability_pack_inspect(
    state: State<'_, AppState>,
    path: String,
) -> Result<CapabilityPackInspection, String> {
    CapabilityPackService::inspect_local(&state.container, Path::new(&path))
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn capability_pack_install(
    state: State<'_, AppState>,
    path: String,
    allow_replacements: bool,
) -> Result<CapabilityPackInstallReceipt, String> {
    CapabilityPackService::install_local(
        &state.container,
        Path::new(&path),
        CapabilityPackInstallOptions { allow_replacements },
    )
    .await
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn capability_pack_activate(
    app: AppHandle,
    state: State<'_, AppState>,
    pack_id: String,
    workspace_path: String,
    session_id: String,
    operation_id: String,
) -> Result<CapabilityPackLiveActivationReceipt, String> {
    let cancel_token = CancellationToken::new();
    {
        let mut runs = state
            .pending_runs
            .lock()
            .map_err(|_| "pending operation lock is poisoned".to_string())?;
        if runs.contains_key(&operation_id) {
            return Err(format!("operation is already running: {operation_id}"));
        }
        runs.insert(operation_id.clone(), cancel_token.clone());
    }

    let (progress, mut progress_rx) = y_service::TurnEventSender::channel();
    let event_app = app.clone();
    let event_operation_id = operation_id.clone();
    let event_session_id = session_id.clone();
    let forwarder = tokio::spawn(async move {
        while let Some((event, _child_session_id)) = progress_rx.recv().await {
            if let TurnEvent::PermissionRequest {
                request_id,
                tool_name,
                action_description,
                reason,
                content_preview,
            } = event
            {
                let _ = event_app.emit(
                    "chat:PermissionRequest",
                    super::chat::PermissionRequestPayload {
                        run_id: event_operation_id.clone(),
                        session_id: event_session_id.clone(),
                        request_id,
                        tool_name,
                        action_description,
                        reason,
                        content_preview,
                    },
                );
            }
        }
    });

    let workspace_service = WorkspaceService::new(&state.config_dir);
    let sid = SessionId(session_id);
    let result = async {
        CapabilityPackService::grant_activation(
            &state.container,
            &workspace_service,
            &pack_id,
            Path::new(&workspace_path),
            &sid,
            Some(&progress),
            Some(&cancel_token),
        )
        .await?;
        CapabilityPackService::activate_granted(
            &state.container,
            &workspace_service,
            &pack_id,
            Path::new(&workspace_path),
        )
        .await
    }
    .await;

    drop(progress);
    let _ = forwarder.await;
    if let Ok(mut runs) = state.pending_runs.lock() {
        runs.remove(&operation_id);
    }
    result.map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn capability_pack_revoke(
    state: State<'_, AppState>,
    pack_id: String,
    workspace_path: String,
) -> Result<CapabilityPackActivationRevocationReceipt, String> {
    CapabilityPackService::revoke_activation(
        &state.container,
        &WorkspaceService::new(&state.config_dir),
        &pack_id,
        Path::new(&workspace_path),
    )
    .await
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn capability_pack_activate_granted(
    state: State<'_, AppState>,
    pack_id: String,
    workspace_path: String,
) -> Result<CapabilityPackLiveActivationReceipt, String> {
    CapabilityPackService::activate_granted(
        &state.container,
        &WorkspaceService::new(&state.config_dir),
        &pack_id,
        Path::new(&workspace_path),
    )
    .await
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn capability_pack_rollback(
    state: State<'_, AppState>,
    pack_id: String,
) -> Result<CapabilityPackRollbackReceipt, String> {
    CapabilityPackService::rollback(&state.container, &pack_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn capability_pack_remove(
    state: State<'_, AppState>,
    pack_id: String,
) -> Result<CapabilityPackRemoveReceipt, String> {
    CapabilityPackService::remove(&state.container, &pack_id)
        .await
        .map_err(|error| error.to_string())
}
