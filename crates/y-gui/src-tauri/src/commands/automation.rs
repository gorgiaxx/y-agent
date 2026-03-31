//! Workflow and schedule management command handlers for the GUI Automation tab.

use serde::Deserialize;
use tauri::State;

use y_service::scheduler_service::{
    CreateScheduleRequest, ExecutionSummary, ScheduleSummary, SchedulerService,
    UpdateScheduleRequest,
};
use y_service::workflow_service::{
    CreateWorkflowRequest, DagVisualization, UpdateWorkflowRequest, ValidationResult,
    WorkflowService,
};
use y_service::{SchedulePolicies, TriggerConfig, WorkflowRow};

use crate::state::AppState;

// ---------------------------------------------------------------------------
// Workflow commands
// ---------------------------------------------------------------------------

/// `workflow_list` -- list all workflow templates.
#[tauri::command]
pub async fn workflow_list(state: State<'_, AppState>) -> Result<Vec<WorkflowRow>, String> {
    WorkflowService::list(&state.container.workflow_store)
        .await
        .map_err(|e| e.to_string())
}

/// `workflow_get` -- get a single workflow template by ID or name.
#[tauri::command]
pub async fn workflow_get(state: State<'_, AppState>, id: String) -> Result<WorkflowRow, String> {
    WorkflowService::get(&state.container.workflow_store, &id)
        .await
        .map_err(|e| e.to_string())
}

/// `workflow_create` -- create a new workflow template.
#[tauri::command]
pub async fn workflow_create(
    state: State<'_, AppState>,
    name: String,
    definition: String,
    format: String,
    description: Option<String>,
    tags: Option<String>,
) -> Result<WorkflowRow, String> {
    let req = CreateWorkflowRequest {
        name,
        definition,
        format,
        description,
        tags,
    };
    WorkflowService::create(&state.container.workflow_store, &req)
        .await
        .map_err(|e| e.to_string())
}

/// `workflow_update` -- update an existing workflow template.
#[tauri::command]
pub async fn workflow_update(
    state: State<'_, AppState>,
    id: String,
    definition: Option<String>,
    format: Option<String>,
    description: Option<String>,
    tags: Option<String>,
) -> Result<WorkflowRow, String> {
    let req = UpdateWorkflowRequest {
        definition,
        format,
        description,
        tags,
    };
    WorkflowService::update(&state.container.workflow_store, &id, &req)
        .await
        .map_err(|e| e.to_string())
}

/// `workflow_delete` -- delete a workflow template.
#[tauri::command]
pub async fn workflow_delete(state: State<'_, AppState>, id: String) -> Result<bool, String> {
    WorkflowService::delete(&state.container.workflow_store, &id)
        .await
        .map_err(|e| e.to_string())
}

/// `workflow_validate` -- validate a definition without persisting.
#[tauri::command]
pub async fn workflow_validate(
    definition: String,
    format: String,
) -> Result<ValidationResult, String> {
    Ok(WorkflowService::validate_definition(&definition, &format))
}

/// `workflow_dag` -- get DAG visualization for a stored workflow.
#[tauri::command]
pub async fn workflow_dag(
    state: State<'_, AppState>,
    id: String,
) -> Result<DagVisualization, String> {
    WorkflowService::get_dag_visualization(&state.container.workflow_store, &id)
        .await
        .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Schedule commands
// ---------------------------------------------------------------------------

/// `schedule_list` -- list all schedules.
#[tauri::command]
pub async fn schedule_list(state: State<'_, AppState>) -> Result<Vec<ScheduleSummary>, String> {
    Ok(SchedulerService::list(&state.container.scheduler_manager).await)
}

/// `schedule_get` -- get a single schedule by ID.
#[tauri::command]
pub async fn schedule_get(
    state: State<'_, AppState>,
    id: String,
) -> Result<ScheduleSummary, String> {
    SchedulerService::get(&state.container.scheduler_manager, &id)
        .await
        .map_err(|e| e.to_string())
}

/// Deserialized trigger configuration from the frontend.
#[derive(Debug, Clone, Deserialize)]
pub struct FrontendScheduleCreateRequest {
    pub name: String,
    pub trigger: TriggerConfig,
    pub workflow_id: String,
    #[serde(default)]
    pub parameter_values: serde_json::Value,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// `schedule_create` -- create a new schedule.
#[tauri::command]
pub async fn schedule_create(
    state: State<'_, AppState>,
    request: FrontendScheduleCreateRequest,
) -> Result<ScheduleSummary, String> {
    let req = CreateScheduleRequest {
        name: request.name,
        trigger: request.trigger,
        workflow_id: request.workflow_id,
        parameter_values: request.parameter_values,
        policies: SchedulePolicies::default(),
        description: request.description,
        tags: request.tags,
    };
    SchedulerService::create(
        &state.container.scheduler_manager,
        &req,
        Some(&state.container.schedule_store),
    )
    .await
    .map_err(|e| e.to_string())
}

/// `schedule_update` -- update an existing schedule.
#[tauri::command]
pub async fn schedule_update(
    state: State<'_, AppState>,
    id: String,
    request: UpdateScheduleRequest,
) -> Result<ScheduleSummary, String> {
    SchedulerService::update(
        &state.container.scheduler_manager,
        &id,
        &request,
        Some(&state.container.schedule_store),
    )
    .await
    .map_err(|e| e.to_string())
}

/// `schedule_delete` -- delete a schedule.
#[tauri::command]
pub async fn schedule_delete(state: State<'_, AppState>, id: String) -> Result<bool, String> {
    SchedulerService::delete(
        &state.container.scheduler_manager,
        &id,
        Some(&state.container.schedule_store),
    )
    .await
    .map_err(|e| e.to_string())
}

/// `schedule_pause` -- pause a schedule.
#[tauri::command]
pub async fn schedule_pause(state: State<'_, AppState>, id: String) -> Result<(), String> {
    SchedulerService::pause(
        &state.container.scheduler_manager,
        &id,
        Some(&state.container.schedule_store),
    )
    .await
    .map_err(|e| e.to_string())
}

/// `schedule_resume` -- resume a paused schedule.
#[tauri::command]
pub async fn schedule_resume(state: State<'_, AppState>, id: String) -> Result<(), String> {
    SchedulerService::resume(
        &state.container.scheduler_manager,
        &id,
        Some(&state.container.schedule_store),
    )
    .await
    .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Execution history commands
// ---------------------------------------------------------------------------

/// `schedule_execution_history` -- get execution history for a schedule.
#[tauri::command]
pub async fn schedule_execution_history(
    state: State<'_, AppState>,
    schedule_id: String,
) -> Result<Vec<ExecutionSummary>, String> {
    Ok(SchedulerService::execution_history(&state.container.scheduler_manager, &schedule_id).await)
}

/// `schedule_execution_get` -- get a single execution record.
#[tauri::command]
pub async fn schedule_execution_get(
    state: State<'_, AppState>,
    execution_id: String,
) -> Result<ExecutionSummary, String> {
    SchedulerService::get_execution(&state.container.scheduler_manager, &execution_id)
        .await
        .map_err(|e| e.to_string())
}

/// `schedule_trigger_now` -- manually fire a schedule.
#[tauri::command]
pub async fn schedule_trigger_now(
    state: State<'_, AppState>,
    schedule_id: String,
) -> Result<ExecutionSummary, String> {
    SchedulerService::trigger_now(&state.container.scheduler_manager, &schedule_id)
        .await
        .map_err(|e| e.to_string())
}

/// `workflow_execute` -- manually execute/replay a workflow.
#[tauri::command]
pub async fn workflow_execute(
    state: State<'_, AppState>,
    workflow_id: String,
) -> Result<ExecutionSummary, String> {
    // Verify workflow exists and get its name.
    let wf = WorkflowService::get(&state.container.workflow_store, &workflow_id)
        .await
        .map_err(|e| e.to_string())?;

    SchedulerService::execute_workflow(&state.container.scheduler_manager, &wf.id, &wf.name)
        .await
        .map_err(|e| e.to_string())
}
