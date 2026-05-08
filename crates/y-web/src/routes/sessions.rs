//! Session management endpoints.
//!
//! Mirrors all session-related Tauri commands from the GUI.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use y_core::session::{CreateSessionOptions, SessionFilter, SessionState, SessionType};
use y_core::types::SessionId;
use y_service::{
    decode_session_prompt_config, encode_session_prompt_config, session_prompt_config_has_content,
    SessionPromptConfig,
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
}

/// Request body for `POST /api/v1/sessions`.
#[derive(Debug, Deserialize)]
pub struct CreateSessionRequest {
    pub title: Option<String>,
    pub agent_id: Option<String>,
}

/// Session info returned to clients.
#[derive(Debug, Serialize)]
pub struct SessionInfo {
    pub id: String,
    pub agent_id: Option<String>,
    pub title: Option<String>,
    pub manual_title: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub message_count: usize,
    pub has_custom_prompt: bool,
}

/// A message in the session transcript.
#[derive(Debug, Serialize)]
pub struct MessageInfo {
    pub id: String,
    pub role: String,
    pub content: String,
    pub timestamp: String,
    pub tool_calls: Vec<ToolCallBrief>,
    #[serde(skip_serializing_if = "serde_json::Value::is_null")]
    pub metadata: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skills: Option<Vec<String>>,
}

/// Brief tool call info for display.
#[derive(Debug, Serialize)]
pub struct ToolCallBrief {
    pub id: String,
    pub name: String,
    pub arguments: String,
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
// Helpers
// ---------------------------------------------------------------------------

fn is_user_visible_session(session_type: &SessionType, state: &SessionState) -> bool {
    session_type.is_user_facing() && *state == SessionState::Active
}

fn session_to_info(s: &y_core::session::SessionNode, has_custom_prompt: bool) -> SessionInfo {
    SessionInfo {
        id: s.id.0.clone(),
        agent_id: s.agent_id.as_ref().map(|id| id.0.clone()),
        title: s.title.clone(),
        manual_title: s.manual_title.clone(),
        created_at: s.created_at.to_rfc3339(),
        updated_at: s.updated_at.to_rfc3339(),
        message_count: s.message_count as usize,
        has_custom_prompt,
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /api/v1/sessions`
async fn list_sessions(
    State(state): State<AppState>,
    Query(query): Query<ListSessionsQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let filter = SessionFilter {
        state: match query.state.as_deref() {
            Some("Archived") => Some(SessionState::Archived),
            _ => Some(SessionState::Active),
        },
        agent_id: query.agent_id.map(y_core::types::AgentId::from_string),
        ..Default::default()
    };

    let sessions = state
        .container
        .session_manager
        .list_sessions(&filter)
        .await
        .map_err(|e| ApiError::Internal(format!("{e}")))?;

    // Check which sessions have custom prompt composition.
    let mut custom_prompt_ids = std::collections::HashSet::new();
    for s in &sessions {
        if let Ok(stored) = state
            .container
            .session_manager
            .get_custom_system_prompt(&s.id)
            .await
        {
            let config = decode_session_prompt_config(stored);
            if session_prompt_config_has_content(&config) {
                custom_prompt_ids.insert(s.id.0.clone());
            }
        }
    }

    let mut infos: Vec<SessionInfo> = sessions
        .into_iter()
        .filter(|s| is_user_visible_session(&s.session_type, &s.state))
        .map(|s| {
            let has_custom = custom_prompt_ids.contains(&s.id.0);
            session_to_info(&s, has_custom)
        })
        .collect();

    // Sort by updated_at descending (newest first).
    infos.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

    Ok(Json(infos))
}

/// `POST /api/v1/sessions`
async fn create_session(
    State(state): State<AppState>,
    Json(body): Json<Option<CreateSessionRequest>>,
) -> Result<impl IntoResponse, ApiError> {
    let (title, agent_id) = match body {
        Some(b) => (b.title, b.agent_id),
        None => (None, None),
    };
    let session = state
        .container
        .session_manager
        .create_session(CreateSessionOptions {
            parent_id: None,
            session_type: SessionType::Main,
            agent_id: agent_id.map(y_core::types::AgentId::from_string),
            title,
        })
        .await
        .map_err(|e| ApiError::Internal(format!("{e}")))?;

    let info = session_to_info(&session, false);
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

    let has_custom = state
        .container
        .session_manager
        .get_custom_system_prompt(&id)
        .await
        .ok()
        .map(decode_session_prompt_config)
        .is_some_and(|config| session_prompt_config_has_content(&config));

    Ok(Json(session_to_info(&session, has_custom)))
}

/// `GET /api/v1/sessions/:id/messages`
async fn list_messages(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(params): Query<ListMessagesQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let id = SessionId(session_id.clone());
    let messages = state
        .container
        .session_manager
        .read_display_transcript(&id)
        .await
        .map_err(|_| ApiError::NotFound(format!("session {session_id} not found")))?;

    let mapped: Vec<MessageInfo> = messages
        .iter()
        .map(|m| {
            let skills = m
                .metadata
                .get("skills")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect::<Vec<String>>()
                })
                .filter(|v| !v.is_empty());

            MessageInfo {
                id: m.message_id.clone(),
                role: format!("{:?}", m.role).to_lowercase(),
                content: m.content.clone(),
                timestamp: m.timestamp.to_rfc3339(),
                tool_calls: m
                    .tool_calls
                    .iter()
                    .map(|tc| ToolCallBrief {
                        id: tc.id.clone(),
                        name: tc.name.clone(),
                        arguments: tc.arguments.to_string(),
                    })
                    .collect(),
                metadata: m.metadata.clone(),
                skills,
            }
        })
        .collect();

    let selected: Vec<_> = if let Some(n) = params.last {
        mapped
            .into_iter()
            .rev()
            .take(n)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    } else {
        mapped
    };

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

    Ok((StatusCode::CREATED, Json(session_to_info(&fork, false))))
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
