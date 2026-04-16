//! Workflow management REST endpoints.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use y_service::scheduler_service::SchedulerService;
use y_service::workflow_service::{
    CreateWorkflowRequest, UpdateWorkflowRequest, WorkflowService, WorkflowServiceError,
};

use crate::error::ApiError;
use crate::state::AppState;

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

/// Request body for `POST /api/v1/workflows/validate`.
#[derive(Debug, Deserialize)]
pub struct ValidateRequest {
    /// Workflow definition body.
    pub definition: String,
    /// Format: "`expression_dsl`" or "toml".
    #[serde(default = "default_format")]
    pub format: String,
}

fn default_format() -> String {
    "expression_dsl".to_string()
}

// ---------------------------------------------------------------------------
// Error conversion
// ---------------------------------------------------------------------------

impl From<WorkflowServiceError> for ApiError {
    fn from(err: WorkflowServiceError) -> Self {
        match err {
            WorkflowServiceError::NotFound { id } => ApiError::NotFound(id),
            WorkflowServiceError::Validation { message } => ApiError::BadRequest(message),
            WorkflowServiceError::Storage(e) => ApiError::Internal(e.to_string()),
        }
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /api/v1/workflows`
async fn list_workflows(State(state): State<AppState>) -> Result<impl IntoResponse, ApiError> {
    let rows = WorkflowService::list(&state.container.workflow_store).await?;
    Ok(Json(serde_json::to_value(rows).unwrap_or_default()))
}

/// `GET /api/v1/workflows/:id`
async fn get_workflow(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let row = WorkflowService::get(&state.container.workflow_store, &id).await?;
    Ok(Json(serde_json::to_value(row).unwrap_or_default()))
}

/// `POST /api/v1/workflows`
async fn create_workflow(
    State(state): State<AppState>,
    Json(body): Json<CreateWorkflowRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let row = WorkflowService::create(&state.container.workflow_store, &body).await?;
    Ok((
        StatusCode::CREATED,
        Json(serde_json::to_value(row).unwrap_or_default()),
    ))
}

/// `PUT /api/v1/workflows/:id`
async fn update_workflow(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<UpdateWorkflowRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let row = WorkflowService::update(&state.container.workflow_store, &id, &body).await?;
    Ok(Json(serde_json::to_value(row).unwrap_or_default()))
}

/// `DELETE /api/v1/workflows/:id`
async fn delete_workflow(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let deleted = WorkflowService::delete(&state.container.workflow_store, &id).await?;
    if deleted {
        Ok(Json(serde_json::json!({"message": "deleted"})))
    } else {
        Err(ApiError::NotFound(id))
    }
}

/// `POST /api/v1/workflows/validate`
async fn validate_definition(
    Json(body): Json<ValidateRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let result = WorkflowService::validate_definition(&body.definition, &body.format);
    Ok(Json(serde_json::to_value(result).unwrap_or_default()))
}

/// `GET /api/v1/workflows/:id/dag`
async fn get_dag(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let dag = WorkflowService::get_dag_visualization(&state.container.workflow_store, &id).await?;
    Ok(Json(serde_json::to_value(dag).unwrap_or_default()))
}

/// `POST /api/v1/workflows/:id/execute` -- manually execute a workflow.
async fn execute_workflow(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let wf = WorkflowService::get(&state.container.workflow_store, &id).await?;
    let execution =
        SchedulerService::execute_workflow(&state.container.scheduler_manager, &wf.id, &wf.name)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(serde_json::to_value(execution).unwrap_or_default()))
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

/// Workflow route group.
pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/workflows",
            get(list_workflows).post(create_workflow),
        )
        .route(
            "/api/v1/workflows/{workflow_id}",
            get(get_workflow)
                .put(update_workflow)
                .delete(delete_workflow),
        )
        .route("/api/v1/workflows/validate", post(validate_definition))
        .route("/api/v1/workflows/{workflow_id}/dag", get(get_dag))
        .route(
            "/api/v1/workflows/{workflow_id}/execute",
            post(execute_workflow),
        )
}
