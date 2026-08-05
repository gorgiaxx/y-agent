//! Provider migration endpoints.
//!
//! Mirrors the `provider_migration_detect` / `provider_migration_run` Tauri
//! commands so the shared frontend can quick-import external agent CLI provider
//! configs over HTTP when y-web runs locally. All business logic lives in
//! [`y_service::provider_migration`].

use std::path::PathBuf;

use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use crate::error::ApiError;
use crate::state::AppState;

/// Request body for running a migration.
#[derive(Debug, Deserialize)]
pub struct MigrationRunRequest {
    pub source_id: String,
    pub selected_ids: Vec<String>,
}

/// Resolve the user home directory, mirroring the Tauri layer's resolution
/// (prefer `USERPROFILE` on Windows, else `HOME`).
fn home_dir() -> Option<PathBuf> {
    if cfg!(target_os = "windows") {
        if let Some(home) = std::env::var("USERPROFILE")
            .ok()
            .filter(|v| !v.trim().is_empty())
        {
            return Some(PathBuf::from(home));
        }
    }
    std::env::var("HOME")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .map(PathBuf::from)
}

/// `GET /api/v1/provider-migration/detect` -- detect migratable sources.
async fn detect(State(state): State<AppState>) -> Result<impl IntoResponse, ApiError> {
    let home =
        home_dir().ok_or_else(|| ApiError::Internal("Could not resolve home directory".into()))?;
    let sources = y_service::provider_migration::detect_sources(&home, &state.config_dir);
    Ok(Json(sources))
}

/// `POST /api/v1/provider-migration/run` -- migrate selected providers.
async fn run(
    State(state): State<AppState>,
    Json(body): Json<MigrationRunRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let home =
        home_dir().ok_or_else(|| ApiError::Internal("Could not resolve home directory".into()))?;
    let report = y_service::provider_migration::migrate_source(
        &home,
        &state.config_dir,
        &body.source_id,
        &body.selected_ids,
    )
    .map_err(ApiError::Internal)?;
    Ok(Json(report))
}

/// Provider migration route group.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/provider-migration/detect", get(detect))
        .route("/api/v1/provider-migration/run", post(run))
}
