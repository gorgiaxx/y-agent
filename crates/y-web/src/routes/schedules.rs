//! Schedule management REST endpoints.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};

use y_service::scheduler_service::{
    CreateScheduleRequest, SchedulerService, SchedulerServiceError, UpdateScheduleRequest,
};

use crate::error::ApiError;
use crate::state::AppState;

// ---------------------------------------------------------------------------
// Error conversion
// ---------------------------------------------------------------------------

impl From<SchedulerServiceError> for ApiError {
    fn from(err: SchedulerServiceError) -> Self {
        match err {
            SchedulerServiceError::NotFound { id } => ApiError::NotFound(id),
            SchedulerServiceError::Validation { message } => ApiError::BadRequest(message),
            SchedulerServiceError::Internal(e) => ApiError::Internal(e),
        }
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /api/v1/schedules`
async fn list_schedules(State(state): State<AppState>) -> Result<impl IntoResponse, ApiError> {
    let list = SchedulerService::list(&state.container.scheduler_manager).await;
    Ok(Json(serde_json::to_value(list).unwrap_or_default()))
}

/// `GET /api/v1/schedules/:id`
async fn get_schedule(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let schedule = SchedulerService::get(&state.container.scheduler_manager, &id).await?;
    Ok(Json(serde_json::to_value(schedule).unwrap_or_default()))
}

/// `POST /api/v1/schedules`
async fn create_schedule(
    State(state): State<AppState>,
    Json(body): Json<CreateScheduleRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let schedule = SchedulerService::create(
        &state.container.scheduler_manager,
        &body,
        Some(&state.container.schedule_store),
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(serde_json::to_value(schedule).unwrap_or_default()),
    ))
}

/// `PUT /api/v1/schedules/:id`
async fn update_schedule(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<UpdateScheduleRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let schedule = SchedulerService::update(
        &state.container.scheduler_manager,
        &id,
        &body,
        Some(&state.container.schedule_store),
    )
    .await?;
    Ok(Json(serde_json::to_value(schedule).unwrap_or_default()))
}

/// `DELETE /api/v1/schedules/:id`
async fn delete_schedule(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let deleted = SchedulerService::delete(
        &state.container.scheduler_manager,
        &id,
        Some(&state.container.schedule_store),
    )
    .await?;
    if deleted {
        Ok(Json(serde_json::json!({"message": "deleted"})))
    } else {
        Err(ApiError::NotFound(id))
    }
}

/// `POST /api/v1/schedules/:id/pause`
async fn pause_schedule(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    SchedulerService::pause(
        &state.container.scheduler_manager,
        &id,
        Some(&state.container.schedule_store),
    )
    .await?;
    Ok(Json(serde_json::json!({"message": "paused"})))
}

/// `POST /api/v1/schedules/:id/resume`
async fn resume_schedule(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    SchedulerService::resume(
        &state.container.scheduler_manager,
        &id,
        Some(&state.container.schedule_store),
    )
    .await?;
    Ok(Json(serde_json::json!({"message": "resumed"})))
}

// ---------------------------------------------------------------------------
// Execution history handlers
// ---------------------------------------------------------------------------

/// `GET /api/v1/schedules/:id/executions`
async fn execution_history(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let list = SchedulerService::execution_history(&state.container.scheduler_manager, &id).await;
    Ok(Json(serde_json::to_value(list).unwrap_or_default()))
}

/// `GET /api/v1/schedules/executions/:execution_id`
async fn get_execution(
    State(state): State<AppState>,
    Path(execution_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let execution =
        SchedulerService::get_execution(&state.container.scheduler_manager, &execution_id).await?;
    Ok(Json(serde_json::to_value(execution).unwrap_or_default()))
}

/// `POST /api/v1/schedules/:id/trigger`
async fn trigger_now(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let execution = SchedulerService::trigger_now(&state.container.scheduler_manager, &id).await?;
    Ok(Json(serde_json::to_value(execution).unwrap_or_default()))
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

/// Schedule route group.
pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/schedules",
            get(list_schedules).post(create_schedule),
        )
        .route(
            "/api/v1/schedules/executions/{execution_id}",
            get(get_execution),
        )
        .route(
            "/api/v1/schedules/{schedule_id}",
            get(get_schedule)
                .put(update_schedule)
                .delete(delete_schedule),
        )
        .route(
            "/api/v1/schedules/{schedule_id}/pause",
            post(pause_schedule),
        )
        .route(
            "/api/v1/schedules/{schedule_id}/resume",
            post(resume_schedule),
        )
        .route(
            "/api/v1/schedules/{schedule_id}/executions",
            get(execution_history),
        )
        .route("/api/v1/schedules/{schedule_id}/trigger", post(trigger_now))
}
