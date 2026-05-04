//! Chat turn execution endpoints.
//!
//! Mirrors all chat-related Tauri commands from the GUI including async
//! streaming (via SSE), cancel, undo, resend, checkpoints, branch restore,
//! context compaction, user interaction answers, and permission decisions.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use y_core::provider::RequestMode;
use y_core::session::ChatMessageStore;
use y_core::types::SessionId;
use y_service::chat_types::{
    parse_thinking, ChatCheckpointInfo, CompactResult, MessageWithStatus, RestoreResult, UndoResult,
};
use y_service::event_sink::EventSink;
use y_service::{
    ChatService, PermissionPromptResponse, PrepareTurnError, PrepareTurnRequest, PreparedTurn,
    ResendTurnRequest, TurnEvent, WorkspaceService,
};

use crate::error::ApiError;
use crate::routes::events::SseEvent;
use crate::state::{AppState, TurnMeta};

// ---------------------------------------------------------------------------
// Request / Response types
// ---------------------------------------------------------------------------

/// Request body for `POST /api/v1/chat` (synchronous) and `POST /api/v1/chat/send` (async).
#[derive(Debug, Deserialize)]
pub struct ChatRequest {
    pub message: String,
    pub session_id: Option<String>,
    pub provider_id: Option<String>,
    pub request_mode: Option<RequestMode>,
    pub skills: Option<Vec<String>>,
    pub knowledge_collections: Option<Vec<String>>,
    pub context_start_index: Option<usize>,
    pub thinking_effort: Option<String>,
    pub attachments: Option<Vec<serde_json::Value>>,
    pub plan_mode: Option<String>,
    pub mcp_mode: Option<String>,
    pub mcp_servers: Option<Vec<String>>,
    pub image_generation_options: Option<y_core::provider::ImageGenerationOptions>,
}

/// Returned when an async chat turn is started.
#[derive(Debug, Serialize)]
pub struct ChatStarted {
    pub session_id: String,
    pub run_id: String,
}

/// Synchronous chat response (kept for API compatibility).
#[derive(Debug, Serialize)]
pub struct ChatResponse {
    pub content: String,
    pub model: String,
    pub session_id: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
    pub tool_calls: Vec<ToolCallRecord>,
    pub iterations: usize,
}

/// Tool call record in the response.
#[derive(Debug, Serialize)]
pub struct ToolCallRecord {
    pub name: String,
    pub success: bool,
    pub duration_ms: u64,
}

/// Request body for `POST /api/v1/chat/cancel`.
#[derive(Debug, Deserialize)]
pub struct CancelRequest {
    pub run_id: String,
}

/// Request body for `POST /api/v1/chat/undo`.
#[derive(Debug, Deserialize)]
pub struct UndoRequest {
    pub session_id: String,
    pub checkpoint_id: String,
}

/// Request body for `POST /api/v1/chat/resend`.
#[derive(Debug, Deserialize)]
pub struct ResendRequest {
    pub session_id: String,
    pub checkpoint_id: String,
    pub provider_id: Option<String>,
    pub request_mode: Option<RequestMode>,
    pub knowledge_collections: Option<Vec<String>>,
    pub thinking_effort: Option<String>,
    pub plan_mode: Option<String>,
}

/// Request body for `POST /api/v1/chat/find-checkpoint`.
#[derive(Debug, Deserialize)]
pub struct FindCheckpointRequest {
    pub session_id: String,
    pub user_message_content: String,
    pub message_id: Option<String>,
}

/// Request body for `POST /api/v1/chat/restore-branch`.
#[derive(Debug, Deserialize)]
pub struct RestoreBranchRequest {
    pub session_id: String,
    pub checkpoint_id: String,
}

/// Request body for `POST /api/v1/chat/answer-question`.
#[derive(Debug, Deserialize)]
pub struct AnswerQuestionRequest {
    pub interaction_id: String,
    pub answers: serde_json::Value,
}

/// Request body for `POST /api/v1/chat/answer-permission`.
#[derive(Debug, Deserialize)]
pub struct AnswerPermissionRequest {
    pub request_id: String,
    pub decision: PermissionPromptResponse,
}

// ---------------------------------------------------------------------------
// SseEventSink -- EventSink implementation for SSE transport
// ---------------------------------------------------------------------------

/// SSE-based [`EventSink`] implementation that translates chat lifecycle
/// events into [`SseEvent`] variants sent over a broadcast channel.
struct SseEventSink(tokio::sync::broadcast::Sender<SseEvent>);

impl EventSink for SseEventSink {
    fn emit_started(&self, run_id: &str, session_id: &str) {
        let _ = self.0.send(SseEvent::ChatStarted {
            run_id: run_id.to_owned(),
            session_id: session_id.to_owned(),
        });
    }

    fn emit_progress(&self, run_id: &str, event: &TurnEvent) {
        if let Ok(json) = serde_json::to_value(event) {
            let _ = self.0.send(SseEvent::ChatProgress {
                run_id: run_id.to_owned(),
                event: json,
            });
        }
    }

    fn emit_ask_user(
        &self,
        run_id: &str,
        session_id: &str,
        interaction_id: &str,
        questions: &serde_json::Value,
    ) {
        let _ = self.0.send(SseEvent::AskUser {
            run_id: run_id.to_owned(),
            session_id: session_id.to_owned(),
            interaction_id: interaction_id.to_owned(),
            questions: questions.clone(),
        });
    }

    fn emit_permission_request(
        &self,
        run_id: &str,
        session_id: &str,
        request_id: &str,
        tool_name: &str,
        action_description: &str,
        reason: &str,
        content_preview: Option<&str>,
    ) {
        let _ = self.0.send(SseEvent::PermissionRequest {
            run_id: run_id.to_owned(),
            session_id: session_id.to_owned(),
            request_id: request_id.to_owned(),
            tool_name: tool_name.to_owned(),
            action_description: action_description.to_owned(),
            reason: reason.to_owned(),
            content_preview: content_preview.map(String::from),
        });
    }

    fn emit_complete(&self, payload: &serde_json::Value) {
        let _ = self.0.send(SseEvent::ChatComplete(payload.clone()));
    }

    fn emit_error(&self, run_id: &str, session_id: &str, error: &str) {
        let _ = self.0.send(SseEvent::ChatError {
            run_id: run_id.to_owned(),
            session_id: session_id.to_owned(),
            error: error.to_owned(),
        });
    }

    fn emit_title_updated(&self, session_id: &str, title: &str) {
        let _ = self.0.send(SseEvent::TitleUpdated {
            session_id: session_id.to_owned(),
            title: title.to_owned(),
        });
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `POST /api/v1/chat` -- synchronous chat turn (existing API, kept for compatibility).
async fn chat_turn(
    State(state): State<AppState>,
    Json(body): Json<ChatRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if body.message.trim().is_empty() {
        return Err(ApiError::BadRequest("message must not be empty".into()));
    }

    let thinking = parse_thinking(body.thinking_effort);
    let user_message_metadata = body
        .attachments
        .map(|atts| serde_json::json!({ "attachments": atts }));

    let mut prepared = ChatService::prepare_turn(
        &state.container,
        PrepareTurnRequest {
            session_id: body.session_id.map(SessionId),
            user_input: body.message,
            provider_id: body.provider_id,
            request_mode: body.request_mode,
            skills: body.skills,
            knowledge_collections: body.knowledge_collections,
            thinking,
            user_message_metadata,
            plan_mode: body.plan_mode,
            mcp_mode: body.mcp_mode,
            mcp_servers: body.mcp_servers,
            image_generation_options: body.image_generation_options,
        },
    )
    .await
    .map_err(|e| match e {
        PrepareTurnError::SessionNotFound(msg) => ApiError::NotFound(msg),
        other => ApiError::Internal(other.to_string()),
    })?;
    apply_prepared_working_directory(&state, &mut prepared).await;

    let session_id = prepared.session_id.clone();
    let input = prepared.as_turn_input();

    let result = ChatService::execute_turn(&state.container, &input)
        .await
        .map_err(|e| ApiError::Internal(format!("{e}")))?;

    Ok(Json(ChatResponse {
        content: result.content,
        model: result.model,
        session_id: session_id.0,
        input_tokens: result.input_tokens,
        output_tokens: result.output_tokens,
        cost_usd: result.cost_usd,
        tool_calls: result
            .tool_calls_executed
            .iter()
            .map(|tc| ToolCallRecord {
                name: tc.name.clone(),
                success: tc.success,
                duration_ms: tc.duration_ms,
            })
            .collect(),
        iterations: result.iterations,
    }))
}

/// `POST /api/v1/chat/send` -- async chat turn, returns immediately, streams via SSE.
async fn chat_send(
    State(state): State<AppState>,
    Json(body): Json<ChatRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if body.message.trim().is_empty() {
        return Err(ApiError::BadRequest("message must not be empty".into()));
    }

    let thinking = parse_thinking(body.thinking_effort);
    let user_message_metadata = body
        .attachments
        .map(|atts| serde_json::json!({ "attachments": atts }));

    let mut prepared = ChatService::prepare_turn(
        &state.container,
        PrepareTurnRequest {
            session_id: body.session_id.map(SessionId),
            user_input: body.message,
            provider_id: body.provider_id,
            request_mode: body.request_mode,
            skills: body.skills,
            knowledge_collections: body.knowledge_collections,
            thinking,
            user_message_metadata,
            plan_mode: body.plan_mode,
            mcp_mode: body.mcp_mode,
            mcp_servers: body.mcp_servers,
            image_generation_options: body.image_generation_options,
        },
    )
    .await
    .map_err(|e| match e {
        PrepareTurnError::SessionNotFound(msg) => ApiError::NotFound(msg),
        other => ApiError::Internal(other.to_string()),
    })?;

    // Apply context reset if specified.
    if let Some(start_idx) = body.context_start_index {
        if start_idx < prepared.history.len() {
            prepared.history.drain(..start_idx);
        }
    }
    apply_prepared_working_directory(&state, &mut prepared).await;

    let run_id = uuid::Uuid::new_v4().to_string();
    let result_sid = prepared.session_id.0.clone();
    let result_run_id = run_id.clone();

    // Emit chat:started.
    let _ = state.event_tx.send(SseEvent::ChatStarted {
        run_id: run_id.clone(),
        session_id: result_sid.clone(),
    });

    // Register cancellation token.
    let cancel_token = CancellationToken::new();
    if let Ok(mut runs) = state.pending_runs.lock() {
        runs.insert(run_id.clone(), cancel_token.clone());
    }

    y_service::chat_worker::spawn_llm_worker(
        SseEventSink(state.event_tx.clone()),
        state.container.clone(),
        prepared,
        run_id,
        Arc::clone(&state.turn_meta_cache),
        Arc::clone(&state.pending_runs),
        cancel_token,
        true,
    );

    Ok(Json(ChatStarted {
        session_id: result_sid,
        run_id: result_run_id,
    }))
}

/// `POST /api/v1/chat/cancel`
async fn chat_cancel(
    State(state): State<AppState>,
    Json(body): Json<CancelRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let token = {
        let mut runs = state
            .pending_runs
            .lock()
            .map_err(|_| ApiError::Internal("lock poisoned".into()))?;
        runs.remove(&body.run_id)
    };
    if let Some(tok) = token {
        tok.cancel();
    }

    let _ = state.event_tx.send(SseEvent::ChatError {
        run_id: body.run_id,
        session_id: String::new(),
        error: "Cancelled".to_string(),
    });

    Ok(Json(serde_json::json!({"message": "cancelled"})))
}

/// `POST /api/v1/chat/undo`
async fn chat_undo(
    State(state): State<AppState>,
    Json(body): Json<UndoRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let sid = SessionId(body.session_id.clone());
    let result = state
        .container
        .chat_checkpoint_manager
        .rollback_to(&sid, &body.checkpoint_id)
        .await
        .map_err(|e| ApiError::Internal(format!("{e}")))?;

    if let Ok(mut cache) = state.turn_meta_cache.lock() {
        cache.remove(&body.session_id);
    }

    Ok(Json(UndoResult {
        messages_removed: result.messages_removed,
        restored_turn_number: result.rolled_back_to_turn,
        files_restored: u32::try_from(result.scopes_rolled_back.len()).unwrap_or(0),
    }))
}

/// `POST /api/v1/chat/resend`
async fn chat_resend(
    State(state): State<AppState>,
    Json(body): Json<ResendRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let thinking = parse_thinking(body.thinking_effort);

    let mut prepared = ChatService::prepare_resend_turn(
        &state.container,
        ResendTurnRequest {
            session_id: SessionId(body.session_id.clone()),
            checkpoint_id: body.checkpoint_id,
            provider_id: body.provider_id,
            request_mode: body.request_mode,
            knowledge_collections: body.knowledge_collections,
            thinking,
            plan_mode: body.plan_mode,
        },
    )
    .await
    .map_err(|e| ApiError::Internal(format!("{e}")))?;
    apply_prepared_working_directory(&state, &mut prepared).await;

    let run_id = uuid::Uuid::new_v4().to_string();
    let result_sid = body.session_id.clone();
    let result_run_id = run_id.clone();

    let _ = state.event_tx.send(SseEvent::ChatStarted {
        run_id: run_id.clone(),
        session_id: result_sid.clone(),
    });

    let cancel_token = CancellationToken::new();
    if let Ok(mut runs) = state.pending_runs.lock() {
        runs.insert(run_id.clone(), cancel_token.clone());
    }

    if let Ok(mut cache) = state.turn_meta_cache.lock() {
        cache.remove(&body.session_id);
    }

    y_service::chat_worker::spawn_llm_worker(
        SseEventSink(state.event_tx.clone()),
        state.container.clone(),
        prepared,
        run_id,
        Arc::clone(&state.turn_meta_cache),
        Arc::clone(&state.pending_runs),
        cancel_token,
        false,
    );

    Ok(Json(ChatStarted {
        session_id: result_sid,
        run_id: result_run_id,
    }))
}

async fn apply_prepared_working_directory(state: &AppState, prepared: &mut PreparedTurn) {
    let workspace_path =
        WorkspaceService::new(&state.config_dir).resolve_workspace_path(&prepared.session_id.0);
    let working_directory = normalize_directory(prepared.working_directory.clone())
        .or_else(|| normalize_directory(workspace_path));

    prepared.working_directory.clone_from(&working_directory);

    let mut prompt_context = state.container.prompt_context.write().await;
    prompt_context.working_directory = working_directory;
}

fn normalize_directory(path: Option<String>) -> Option<String> {
    path.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    })
}

/// `GET /api/v1/chat/checkpoints/:session_id`
async fn list_checkpoints(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let sid = SessionId(session_id);
    let checkpoints = state
        .container
        .chat_checkpoint_manager
        .list_checkpoints(&sid)
        .await
        .map_err(|e| ApiError::Internal(format!("{e}")))?;

    let infos: Vec<ChatCheckpointInfo> = checkpoints
        .into_iter()
        .map(|cp| ChatCheckpointInfo {
            checkpoint_id: cp.checkpoint_id,
            session_id: cp.session_id.0,
            turn_number: cp.turn_number,
            message_count_before: cp.message_count_before,
            created_at: cp.created_at.to_rfc3339(),
        })
        .collect();

    Ok(Json(infos))
}

/// `POST /api/v1/chat/find-checkpoint`
async fn find_checkpoint(
    State(state): State<AppState>,
    Json(body): Json<FindCheckpointRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let sid = SessionId(body.session_id);

    let messages = state
        .container
        .session_manager
        .read_display_transcript(&sid)
        .await
        .map_err(|e| ApiError::Internal(format!("{e}")))?;

    let message_index = body
        .message_id
        .as_ref()
        .and_then(|mid| {
            messages
                .iter()
                .enumerate()
                .find(|(_, m)| &m.message_id == mid)
                .map(|(idx, _)| idx)
        })
        .or_else(|| {
            messages
                .iter()
                .enumerate()
                .rev()
                .find(|(_, m)| {
                    m.role == y_core::types::Role::User && m.content == body.user_message_content
                })
                .map(|(idx, _)| idx)
        });

    let Some(message_index) = message_index else {
        return Ok(Json(serde_json::json!(null)));
    };

    let checkpoints = state
        .container
        .chat_checkpoint_manager
        .list_checkpoints(&sid)
        .await
        .map_err(|e| ApiError::Internal(format!("{e}")))?;

    let matched = checkpoints
        .iter()
        .find(|cp| cp.message_count_before as usize == message_index);

    Ok(Json(
        serde_json::to_value(matched.map(|cp| ChatCheckpointInfo {
            checkpoint_id: cp.checkpoint_id.clone(),
            session_id: cp.session_id.0.clone(),
            turn_number: cp.turn_number,
            message_count_before: cp.message_count_before,
            created_at: cp.created_at.to_rfc3339(),
        }))
        .unwrap_or_default(),
    ))
}

/// `GET /api/v1/chat/messages-with-status/:session_id`
async fn messages_with_status(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let sid = SessionId(session_id);
    let records = state
        .container
        .chat_message_store
        .list_by_session(&sid)
        .await
        .map_err(|e| ApiError::Internal(format!("{e}")))?;

    let results: Vec<MessageWithStatus> = records
        .into_iter()
        .map(|r| {
            let status = match r.status {
                y_core::session::ChatMessageStatus::Active => "active",
                y_core::session::ChatMessageStatus::Tombstone => "tombstone",
                y_core::session::ChatMessageStatus::Pruned => "pruned",
            };
            MessageWithStatus {
                id: r.id,
                role: r.role,
                content: r.content,
                status: status.to_string(),
                checkpoint_id: r.checkpoint_id,
                model: r.model,
                input_tokens: r.input_tokens,
                output_tokens: r.output_tokens,
                cost_usd: r.cost_usd,
                context_window: r.context_window,
                created_at: r.created_at.to_rfc3339(),
            }
        })
        .collect();

    Ok(Json(results))
}

/// `POST /api/v1/chat/restore-branch`
async fn restore_branch(
    State(state): State<AppState>,
    Json(body): Json<RestoreBranchRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let sid = SessionId(body.session_id.clone());
    let (tombstoned_count, restored_count) = state
        .container
        .chat_message_store
        .swap_branches(&sid, &body.checkpoint_id)
        .await
        .map_err(|e| ApiError::Internal(format!("{e}")))?;

    if let Ok(mut cache) = state.turn_meta_cache.lock() {
        cache.remove(&body.session_id);
    }

    Ok(Json(RestoreResult {
        tombstoned_count,
        restored_count,
    }))
}

/// `POST /api/v1/chat/compact/:session_id`
async fn context_compact(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let sid = SessionId(session_id);
    let report = y_service::context_optimization::ContextOptimizationService::compact_now(
        &state.container,
        &sid,
    )
    .await
    .map_err(|e| ApiError::Internal(format!("{e}")))?;

    Ok(Json(CompactResult {
        messages_pruned: report.messages_pruned,
        messages_compacted: report.messages_compacted,
        tokens_saved: report.pruning_tokens_saved + report.compaction_tokens_saved,
        summary: report.compaction_summary,
    }))
}

/// `POST /api/v1/chat/answer-question`
async fn answer_question(
    State(state): State<AppState>,
    Json(body): Json<AnswerQuestionRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let delivered =
        y_service::user_interaction_orchestrator::UserInteractionOrchestrator::deliver_answer(
            &body.interaction_id,
            body.answers,
            &state.container.pending_interactions,
        )
        .await;

    Ok(Json(serde_json::json!({ "delivered": delivered })))
}

/// `POST /api/v1/chat/answer-permission`
async fn answer_permission(
    State(state): State<AppState>,
    Json(body): Json<AnswerPermissionRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let delivered = {
        let mut map = state.container.pending_permissions.lock().await;
        if let Some(sender) = map.remove(&body.request_id) {
            sender.send(body.decision).is_ok()
        } else {
            false
        }
    };

    Ok(Json(serde_json::json!({ "delivered": delivered })))
}

/// `GET /api/v1/chat/last-turn-meta/:session_id`
async fn last_turn_meta(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    // Tier 1: in-memory cache.
    {
        let cache = state
            .turn_meta_cache
            .lock()
            .map_err(|_| ApiError::Internal("lock poisoned".into()))?;
        if let Some(meta) = cache.get(&session_id) {
            return Ok(Json(serde_json::to_value(meta).unwrap_or_default()));
        }
    }

    // Tier 2: diagnostics database.
    let summary = ChatService::get_last_turn_meta(&state.container, &session_id)
        .await
        .map_err(ApiError::Internal)?;

    match summary {
        Some(s) => {
            let meta = TurnMeta {
                provider_id: s.provider_id,
                model: s.model,
                input_tokens: s.input_tokens,
                output_tokens: s.output_tokens,
                cost_usd: s.cost_usd,
                context_window: s.context_window,
                context_tokens_used: s.context_tokens_used,
            };
            if let Ok(mut cache) = state.turn_meta_cache.lock() {
                cache.insert(session_id, meta.clone());
            }
            Ok(Json(serde_json::to_value(meta).unwrap_or_default()))
        }
        None => Ok(Json(serde_json::Value::Null)),
    }
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

/// Chat route group.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/chat", post(chat_turn))
        .route("/api/v1/chat/send", post(chat_send))
        .route("/api/v1/chat/cancel", post(chat_cancel))
        .route("/api/v1/chat/undo", post(chat_undo))
        .route("/api/v1/chat/resend", post(chat_resend))
        .route(
            "/api/v1/chat/checkpoints/{session_id}",
            get(list_checkpoints),
        )
        .route("/api/v1/chat/find-checkpoint", post(find_checkpoint))
        .route(
            "/api/v1/chat/messages-with-status/{session_id}",
            get(messages_with_status),
        )
        .route("/api/v1/chat/restore-branch", post(restore_branch))
        .route("/api/v1/chat/compact/{session_id}", post(context_compact))
        .route("/api/v1/chat/answer-question", post(answer_question))
        .route("/api/v1/chat/answer-permission", post(answer_permission))
        .route(
            "/api/v1/chat/last-turn-meta/{session_id}",
            get(last_turn_meta),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chat_request_deserializes_shared_frontend_image_options() {
        let request: ChatRequest = serde_json::from_value(serde_json::json!({
            "message": "make an image",
            "request_mode": "image_generation",
            "image_generation_options": {
                "watermark": true,
                "max_images": 2,
                "size": "1024x1024"
            }
        }))
        .expect("chat request should deserialize");

        assert_eq!(request.request_mode, Some(RequestMode::ImageGeneration));
        let options = request
            .image_generation_options
            .expect("image generation options should be preserved");
        assert!(options.watermark);
        assert_eq!(options.max_images, 2);
        assert_eq!(options.size.as_deref(), Some("1024x1024"));
    }
}
