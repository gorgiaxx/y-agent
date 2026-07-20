//! Capability Pack REST lifecycle adapters.

use std::path::Path as FilePath;

use axum::extract::{Path, State};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::Deserialize;
use tokio_util::sync::CancellationToken;
use y_core::types::SessionId;
use y_service::capability_pack::{CapabilityPackInstallOptions, CapabilityPackService};
use y_service::{TurnEvent, TurnEventSender, WorkspaceService};

use crate::error::ApiError;
use crate::routes::events::{SseEnvelope, SseEvent};
use crate::state::AppState;

#[derive(Debug, Deserialize)]
struct LocalPackRequest {
    path: String,
}

#[derive(Debug, Deserialize)]
struct InstallPackRequest {
    path: String,
    #[serde(default)]
    allow_replacements: bool,
}

#[derive(Debug, Deserialize)]
struct ActivatePackRequest {
    workspace_path: String,
    session_id: String,
    operation_id: String,
}

#[derive(Debug, Deserialize)]
struct RevokePackRequest {
    workspace_path: String,
}

async fn list_packs(State(state): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    let packs = CapabilityPackService::list_installed(&state.container)
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    Ok(Json(serde_json::to_value(packs).unwrap_or_default()))
}

async fn inspect_pack(
    State(state): State<AppState>,
    Json(body): Json<LocalPackRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let inspection =
        CapabilityPackService::inspect_local(&state.container, FilePath::new(&body.path))
            .await
            .map_err(|error| ApiError::BadRequest(error.to_string()))?;
    Ok(Json(serde_json::to_value(inspection).unwrap_or_default()))
}

async fn install_pack(
    State(state): State<AppState>,
    Json(body): Json<InstallPackRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let receipt = CapabilityPackService::install_local(
        &state.container,
        FilePath::new(&body.path),
        CapabilityPackInstallOptions {
            allow_replacements: body.allow_replacements,
        },
    )
    .await
    .map_err(|error| ApiError::BadRequest(error.to_string()))?;
    Ok(Json(serde_json::to_value(receipt).unwrap_or_default()))
}

async fn activate_pack(
    State(state): State<AppState>,
    Path(pack_id): Path<String>,
    Json(body): Json<ActivatePackRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let cancel_token = CancellationToken::new();
    {
        let mut runs = state
            .pending_runs
            .lock()
            .map_err(|_| ApiError::Internal("pending operation lock is poisoned".to_string()))?;
        if runs.contains_key(&body.operation_id) {
            return Err(ApiError::BadRequest(format!(
                "operation is already running: {}",
                body.operation_id
            )));
        }
        runs.insert(body.operation_id.clone(), cancel_token.clone());
    }

    let (progress, mut progress_rx) = TurnEventSender::channel();
    let event_tx = state.event_tx.clone();
    let event_operation_id = body.operation_id.clone();
    let event_session_id = body.session_id.clone();
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
                let permission = SseEvent::PermissionRequest {
                    run_id: event_operation_id.clone(),
                    session_id: event_session_id.clone(),
                    request_id,
                    tool_name,
                    action_description,
                    reason,
                    content_preview,
                };
                let _ = event_tx.send(SseEnvelope::for_session(
                    permission,
                    None,
                    event_session_id.clone(),
                ));
            }
        }
    });

    let workspace_service = WorkspaceService::new(&state.config_dir);
    let session_id = SessionId(body.session_id.clone());
    let result = async {
        CapabilityPackService::grant_activation(
            &state.container,
            &workspace_service,
            &pack_id,
            FilePath::new(&body.workspace_path),
            &session_id,
            Some(&progress),
            Some(&cancel_token),
        )
        .await?;
        CapabilityPackService::activate_granted(
            &state.container,
            &workspace_service,
            &pack_id,
            FilePath::new(&body.workspace_path),
        )
        .await
    }
    .await;

    drop(progress);
    let _ = forwarder.await;
    if let Ok(mut runs) = state.pending_runs.lock() {
        runs.remove(&body.operation_id);
    }
    let receipt = result.map_err(|error| ApiError::BadRequest(error.to_string()))?;
    Ok(Json(serde_json::to_value(receipt).unwrap_or_default()))
}

async fn revoke_pack(
    State(state): State<AppState>,
    Path(pack_id): Path<String>,
    Json(body): Json<RevokePackRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let receipt = CapabilityPackService::revoke_activation(
        &state.container,
        &WorkspaceService::new(&state.config_dir),
        &pack_id,
        FilePath::new(&body.workspace_path),
    )
    .await
    .map_err(|error| ApiError::BadRequest(error.to_string()))?;
    Ok(Json(serde_json::to_value(receipt).unwrap_or_default()))
}

async fn activate_granted_pack(
    State(state): State<AppState>,
    Path(pack_id): Path<String>,
    Json(body): Json<RevokePackRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let receipt = CapabilityPackService::activate_granted(
        &state.container,
        &WorkspaceService::new(&state.config_dir),
        &pack_id,
        FilePath::new(&body.workspace_path),
    )
    .await
    .map_err(|error| ApiError::BadRequest(error.to_string()))?;
    Ok(Json(serde_json::to_value(receipt).unwrap_or_default()))
}

async fn rollback_pack(
    State(state): State<AppState>,
    Path(pack_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let receipt = CapabilityPackService::rollback(&state.container, &pack_id)
        .await
        .map_err(|error| ApiError::BadRequest(error.to_string()))?;
    Ok(Json(serde_json::to_value(receipt).unwrap_or_default()))
}

async fn remove_pack(
    State(state): State<AppState>,
    Path(pack_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let receipt = CapabilityPackService::remove(&state.container, &pack_id)
        .await
        .map_err(|error| ApiError::BadRequest(error.to_string()))?;
    Ok(Json(serde_json::to_value(receipt).unwrap_or_default()))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/capability-packs", get(list_packs))
        .route("/api/v1/capability-packs/inspect", post(inspect_pack))
        .route("/api/v1/capability-packs/install", post(install_pack))
        .route(
            "/api/v1/capability-packs/{pack_id}/activate",
            post(activate_pack),
        )
        .route(
            "/api/v1/capability-packs/{pack_id}/activate-granted",
            post(activate_granted_pack),
        )
        .route(
            "/api/v1/capability-packs/{pack_id}/revoke",
            post(revoke_pack),
        )
        .route(
            "/api/v1/capability-packs/{pack_id}/rollback",
            post(rollback_pack),
        )
        .route("/api/v1/capability-packs/{pack_id}", delete(remove_pack))
}
