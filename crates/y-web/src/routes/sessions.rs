//! Session management endpoints.
//!
//! Mirrors all session-related Tauri commands from the GUI.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use y_core::session::{SessionFilter, SessionState};
use y_core::types::SessionId;
use y_service::{
    decode_session_prompt_config, encode_session_prompt_config, SessionPromptConfig, SessionService,
};

use crate::error::ApiError;
use crate::state::AppState;

// ---------------------------------------------------------------------------
// Request / Response types
// ---------------------------------------------------------------------------

/// Query params for `GET /api/v1/sessions`.
#[derive(Debug, Deserialize)]
pub struct ListSessionsQuery {
    /// Filter by state: "Active", "Archived", or unset for all.
    pub state: Option<String>,
    /// Filter by agent ID.
    pub agent_id: Option<String>,
    /// Restrict resumable history to one canonical workspace.
    pub workspace_path: Option<String>,
}

/// Request body for `POST /api/v1/sessions`.
#[derive(Debug, Deserialize)]
pub struct CreateSessionRequest {
    pub title: Option<String>,
    pub agent_id: Option<String>,
    pub workspace_path: Option<String>,
}

/// Request body for `POST /api/v1/sessions/:id/fork`.
#[derive(Debug, Deserialize)]
pub struct ForkRequest {
    pub message_index: usize,
    pub title: Option<String>,
}

/// Request body for `PUT /api/v1/sessions/:id/rename`.
#[derive(Debug, Deserialize)]
pub struct RenameRequest {
    pub title: Option<String>,
}

/// Request body for `PUT /api/v1/sessions/:id/context-reset`.
#[derive(Debug, Deserialize)]
pub struct ContextResetRequest {
    pub index: Option<u32>,
}

/// Request body for `PUT /api/v1/sessions/:id/custom-prompt`.
#[derive(Debug, Deserialize)]
pub struct CustomPromptRequest {
    pub prompt: Option<String>,
}

/// Request body for `PUT /api/v1/sessions/:id/prompt-config`.
#[derive(Debug, Deserialize)]
pub struct PromptConfigRequest {
    pub config: SessionPromptConfig,
}

/// Request body for `POST /api/v1/sessions/:id/truncate`.
#[derive(Debug, Deserialize)]
pub struct TruncateRequest {
    pub keep_count: usize,
}

/// Minimal success message.
#[derive(Serialize)]
pub struct MessageResponse {
    pub message: String,
}

/// Query params for message listing.
#[derive(Debug, Deserialize)]
pub struct ListMessagesQuery {
    pub last: Option<usize>,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /api/v1/sessions`
async fn list_sessions(
    State(state): State<AppState>,
    Query(query): Query<ListSessionsQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let workspaces = y_service::WorkspaceService::new(&state.config_dir);
    y_service::SessionService::backfill_legacy_assignments(
        &state.container.session_manager,
        &workspaces,
    )
    .await
    .map_err(|error| ApiError::Internal(format!("{error}")))?;
    let agent_id = query.agent_id.map(y_core::types::AgentId::from_string);
    let filter = SessionFilter {
        state: match query.state.as_deref() {
            Some("Archived") => Some(SessionState::Archived),
            _ => Some(SessionState::Active),
        },
        agent_id: agent_id.clone(),
        ..Default::default()
    };

    let sessions = if let Some(workspace_path) = query.workspace_path {
        y_service::SessionService::list_resumable_sessions(
            &state.container.session_manager,
            std::path::Path::new(&workspace_path),
            agent_id,
        )
        .await
        .map_err(|error| ApiError::BadRequest(format!("{error}")))?
    } else {
        state
            .container
            .session_manager
            .list_sessions(&filter)
            .await
            .map_err(|e| ApiError::Internal(format!("{e}")))?
    };

    let infos = SessionService::session_infos(&state.container.session_manager, sessions).await;
    Ok(Json(infos))
}

/// `POST /api/v1/sessions`
async fn create_session(
    State(state): State<AppState>,
    Json(body): Json<Option<CreateSessionRequest>>,
) -> Result<impl IntoResponse, ApiError> {
    let (title, agent_id, workspace_path) = match body {
        Some(b) => (b.title, b.agent_id, b.workspace_path),
        None => (None, None, None),
    };
    let info = SessionService::create_main_session(
        &state.container.session_manager,
        title,
        agent_id,
        workspace_path.as_deref(),
    )
    .await
    .map_err(|error| ApiError::Internal(format!("{error}")))?;
    Ok((StatusCode::CREATED, Json(info)))
}

/// `GET /api/v1/sessions/:id`
async fn get_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let id = SessionId(session_id.clone());
    let session = state
        .container
        .session_manager
        .get_session(&id)
        .await
        .map_err(|_| ApiError::NotFound(format!("session {session_id} not found")))?;

    Ok(Json(
        SessionService::session_info(&state.container.session_manager, session).await,
    ))
}

/// `GET /api/v1/sessions/:id/children`
///
/// Direct child (sub-agent) sessions, oldest first — powers drill-in into
/// plan phase / loop round / delegated-task transcripts.
async fn list_child_sessions(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let sid = SessionId(session_id.clone());
    let mapped = SessionService::child_session_infos(&state.container.session_manager, &sid)
        .await
        .map_err(|_| ApiError::NotFound(format!("session {session_id} not found")))?;

    Ok(Json(mapped))
}

/// `GET /api/v1/sessions/:id/messages`
async fn list_messages(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(params): Query<ListMessagesQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let id = SessionId(session_id.clone());
    let selected =
        SessionService::message_infos(&state.container.session_manager, &id, params.last)
            .await
            .map_err(|_| ApiError::NotFound(format!("session {session_id} not found")))?;

    Ok(Json(selected))
}

/// `DELETE /api/v1/sessions/:id`
async fn delete_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let id = SessionId(session_id.clone());
    state
        .container
        .session_manager
        .delete_session(&id)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to delete session: {e}")))?;
    state.container.cleanup_session_state(&id).await;

    Ok(Json(MessageResponse {
        message: format!("session {session_id} deleted"),
    }))
}

/// `POST /api/v1/sessions/:id/archive`
async fn archive_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let id = SessionId(session_id.clone());
    state
        .container
        .session_manager
        .transition_state(&id, SessionState::Archived)
        .await
        .map_err(|_| ApiError::NotFound(format!("session {session_id} not found")))?;

    Ok(Json(MessageResponse {
        message: format!("session {session_id} archived"),
    }))
}

/// `POST /api/v1/sessions/:id/truncate`
async fn truncate_messages(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(body): Json<TruncateRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let sid = SessionId(session_id);
    state
        .container
        .session_manager
        .display_transcript_store()
        .truncate(&sid, body.keep_count)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to truncate display transcript: {e}")))?;
    state
        .container
        .session_manager
        .transcript_store()
        .truncate(&sid, body.keep_count)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to truncate context transcript: {e}")))?;

    Ok(Json(MessageResponse {
        message: "truncated".to_string(),
    }))
}

/// `GET /api/v1/sessions/:id/context-reset`
async fn get_context_reset(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let sid = SessionId(session_id);
    let index = state
        .container
        .session_manager
        .get_context_reset_index(&sid)
        .await
        .map_err(|e| ApiError::Internal(format!("{e}")))?;

    Ok(Json(serde_json::json!({ "index": index })))
}

/// `PUT /api/v1/sessions/:id/context-reset`
async fn set_context_reset(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(body): Json<ContextResetRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let sid = SessionId(session_id);
    state
        .container
        .session_manager
        .set_context_reset_index(&sid, body.index)
        .await
        .map_err(|e| ApiError::Internal(format!("{e}")))?;

    Ok(Json(MessageResponse {
        message: "context reset updated".to_string(),
    }))
}

/// `GET /api/v1/sessions/:id/custom-prompt`
async fn get_custom_prompt(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let sid = SessionId(session_id);
    let stored = state
        .container
        .session_manager
        .get_custom_system_prompt(&sid)
        .await
        .map_err(|e| ApiError::Internal(format!("{e}")))?;
    let prompt = decode_session_prompt_config(stored).system_prompt;

    Ok(Json(serde_json::json!({ "prompt": prompt })))
}

/// `PUT /api/v1/sessions/:id/custom-prompt`
async fn set_custom_prompt(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(body): Json<CustomPromptRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let sid = SessionId(session_id);
    state
        .container
        .session_manager
        .set_custom_system_prompt(
            &sid,
            encode_session_prompt_config(&SessionPromptConfig {
                system_prompt: body.prompt,
                prompt_section_ids: Vec::new(),
                template_id: None,
            }),
        )
        .await
        .map_err(|e| ApiError::Internal(format!("{e}")))?;

    Ok(Json(MessageResponse {
        message: "custom prompt updated".to_string(),
    }))
}

/// `GET /api/v1/sessions/:id/prompt-config`
async fn get_prompt_config(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let sid = SessionId(session_id);
    let stored = state
        .container
        .session_manager
        .get_custom_system_prompt(&sid)
        .await
        .map_err(|e| ApiError::Internal(format!("{e}")))?;

    Ok(Json(decode_session_prompt_config(stored)))
}

/// `PUT /api/v1/sessions/:id/prompt-config`
async fn set_prompt_config(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(body): Json<PromptConfigRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let sid = SessionId(session_id);
    state
        .container
        .session_manager
        .set_custom_system_prompt(&sid, encode_session_prompt_config(&body.config))
        .await
        .map_err(|e| ApiError::Internal(format!("{e}")))?;

    Ok(Json(MessageResponse {
        message: "prompt config updated".to_string(),
    }))
}

/// `POST /api/v1/sessions/:id/fork`
async fn fork_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(body): Json<ForkRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let sid = SessionId(session_id);
    let fork = state
        .container
        .session_manager
        .fork_session(&sid, body.message_index, body.title)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to fork session: {e}")))?;

    let info = SessionService::session_info(&state.container.session_manager, fork).await;
    Ok((StatusCode::CREATED, Json(info)))
}

/// `PUT /api/v1/sessions/:id/rename`
async fn rename_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(body): Json<RenameRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let sid = SessionId(session_id);
    state
        .container
        .session_manager
        .set_manual_title(&sid, body.title)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to rename session: {e}")))?;

    Ok(Json(MessageResponse {
        message: "renamed".to_string(),
    }))
}

/// `POST /api/v1/sessions/:id/branch`
async fn branch_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(body): Json<Option<serde_json::Value>>,
) -> Result<impl IntoResponse, ApiError> {
    let id = SessionId(session_id.clone());
    let label = body.and_then(|b| b.get("label").and_then(|v| v.as_str()).map(String::from));
    let branch = state
        .container
        .session_manager
        .branch(&id, label)
        .await
        .map_err(|e| ApiError::Internal(format!("branch failed: {e}")))?;

    Ok((
        StatusCode::CREATED,
        Json(serde_json::to_value(branch).unwrap_or_default()),
    ))
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

/// Session route group.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/sessions", get(list_sessions).post(create_session))
        .route(
            "/api/v1/sessions/{session_id}",
            get(get_session).delete(delete_session),
        )
        .route("/api/v1/sessions/{session_id}/messages", get(list_messages))
        .route(
            "/api/v1/sessions/{session_id}/children",
            get(list_child_sessions),
        )
        .route(
            "/api/v1/sessions/{session_id}/archive",
            post(archive_session),
        )
        .route("/api/v1/sessions/{session_id}/branch", post(branch_session))
        .route(
            "/api/v1/sessions/{session_id}/truncate",
            post(truncate_messages),
        )
        .route(
            "/api/v1/sessions/{session_id}/context-reset",
            get(get_context_reset).put(set_context_reset),
        )
        .route(
            "/api/v1/sessions/{session_id}/custom-prompt",
            get(get_custom_prompt).put(set_custom_prompt),
        )
        .route(
            "/api/v1/sessions/{session_id}/prompt-config",
            get(get_prompt_config).put(set_prompt_config),
        )
        .route("/api/v1/sessions/{session_id}/fork", post(fork_session))
        .route("/api/v1/sessions/{session_id}/rename", put(rename_session))
}
