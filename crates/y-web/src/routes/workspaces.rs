//! Workspace management endpoints.
//!
//! Mirrors all workspace-related Tauri commands from the GUI.

use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::Deserialize;

use y_service::{WorkspaceRecord, WorkspaceService, WorkspaceTrustDecision};

use crate::error::ApiError;
use crate::state::AppState;

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct CreateWorkspaceRequest {
    pub name: String,
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateWorkspaceRequest {
    pub name: String,
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct AssignSessionRequest {
    pub workspace_id: String,
    pub session_id: String,
}

#[derive(Debug, Deserialize)]
pub struct UnassignSessionRequest {
    pub session_id: String,
}

#[derive(Debug, Deserialize)]
pub struct WorkspaceTrustRequest {
    pub path: String,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn svc(state: &AppState) -> WorkspaceService {
    WorkspaceService::new(&state.config_dir)
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /api/v1/workspaces`
async fn list_workspaces(State(state): State<AppState>) -> Json<Vec<WorkspaceRecord>> {
    Json(svc(&state).list())
}

/// `POST /api/v1/workspaces`
async fn create_workspace(
    State(state): State<AppState>,
    Json(body): Json<CreateWorkspaceRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let record = svc(&state)
        .create(body.name, body.path)
        .map_err(|e| ApiError::Internal(format!("Failed to create workspace: {e}")))?;
    Ok((StatusCode::CREATED, Json(record)))
}

/// `PUT /api/v1/workspaces/:id`
async fn update_workspace(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<UpdateWorkspaceRequest>,
) -> Result<impl IntoResponse, ApiError> {
    svc(&state)
        .update(&id, body.name, body.path)
        .map_err(|e| ApiError::Internal(format!("Failed to update workspace: {e}")))?;
    Ok(Json(serde_json::json!({"message": "updated"})))
}

/// `DELETE /api/v1/workspaces/:id`
async fn delete_workspace(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    svc(&state)
        .delete(&id)
        .map_err(|e| ApiError::Internal(format!("Failed to delete workspace: {e}")))?;
    Ok(Json(serde_json::json!({"message": "deleted"})))
}

/// `GET /api/v1/workspaces/session-map`
async fn session_map(State(state): State<AppState>) -> Json<HashMap<String, String>> {
    Json(svc(&state).session_map())
}

/// `POST /api/v1/workspaces/assign`
async fn assign_session(
    State(state): State<AppState>,
    Json(body): Json<AssignSessionRequest>,
) -> Result<impl IntoResponse, ApiError> {
    svc(&state)
        .assign_session(body.workspace_id, body.session_id)
        .map_err(|e| ApiError::Internal(format!("Failed to assign session: {e}")))?;
    Ok(Json(serde_json::json!({"message": "assigned"})))
}

/// `POST /api/v1/workspaces/unassign`
async fn unassign_session(
    State(state): State<AppState>,
    Json(body): Json<UnassignSessionRequest>,
) -> Result<impl IntoResponse, ApiError> {
    svc(&state)
        .unassign_session(&body.session_id)
        .map_err(|e| ApiError::Internal(format!("Failed to unassign session: {e}")))?;
    Ok(Json(serde_json::json!({"message": "unassigned"})))
}

/// `GET /api/v1/workspaces/trust-status?path=...`
async fn workspace_trust_status(
    State(state): State<AppState>,
    Query(query): Query<WorkspaceTrustRequest>,
) -> Result<Json<WorkspaceTrustDecision>, ApiError> {
    svc(&state)
        .workspace_trust(std::path::Path::new(&query.path))
        .map(Json)
        .map_err(|error| {
            ApiError::BadRequest(format!("Failed to resolve workspace trust: {error}"))
        })
}

/// `POST /api/v1/workspaces/trust`
async fn trust_workspace(
    State(state): State<AppState>,
    Json(body): Json<WorkspaceTrustRequest>,
) -> Result<Json<WorkspaceTrustDecision>, ApiError> {
    svc(&state)
        .trust_workspace(std::path::Path::new(&body.path))
        .map(Json)
        .map_err(|error| ApiError::BadRequest(format!("Failed to trust workspace: {error}")))
}

/// `POST /api/v1/workspaces/untrust`
async fn untrust_workspace(
    State(state): State<AppState>,
    Json(body): Json<WorkspaceTrustRequest>,
) -> Result<Json<WorkspaceTrustDecision>, ApiError> {
    svc(&state)
        .untrust_workspace(std::path::Path::new(&body.path))
        .map(Json)
        .map_err(|error| ApiError::BadRequest(format!("Failed to block workspace: {error}")))
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

/// Workspace route group.
pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/workspaces",
            get(list_workspaces).post(create_workspace),
        )
        .route("/api/v1/workspaces/session-map", get(session_map))
        .route("/api/v1/workspaces/assign", post(assign_session))
        .route("/api/v1/workspaces/unassign", post(unassign_session))
        .route(
            "/api/v1/workspaces/trust-status",
            get(workspace_trust_status),
        )
        .route("/api/v1/workspaces/trust", post(trust_workspace))
        .route("/api/v1/workspaces/untrust", post(untrust_workspace))
        .route(
            "/api/v1/workspaces/{id}",
            put(update_workspace).delete(delete_workspace),
        )
}
