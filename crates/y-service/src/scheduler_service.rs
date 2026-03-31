//! Scheduled task management service.
//!
//! Wraps `y-scheduler::SchedulerManager` with a service-layer API for
//! GUI / REST consumption. Provides CRUD, pause/resume, and schedule listing.
//!
//! All mutations are written-through to `SqliteScheduleStore` so that
//! schedules survive application restarts. On startup,
//! [`SchedulerService::load_schedules_from_db`] hydrates the in-memory
//! store from `SQLite`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use y_scheduler::{Schedule, SchedulePolicies, SchedulerConfig, SchedulerManager, TriggerConfig};
use y_storage::SqliteScheduleStore;

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

/// Errors from scheduler service operations.
#[derive(Debug, thiserror::Error)]
pub enum SchedulerServiceError {
    #[error("schedule not found: {id}")]
    NotFound { id: String },

    #[error("validation failed: {message}")]
    Validation { message: String },

    #[error("scheduler error: {0}")]
    Internal(String),
}

// ---------------------------------------------------------------------------
// Request / Response types
// ---------------------------------------------------------------------------

/// Request to create a new schedule.
#[derive(Debug, Clone, Deserialize)]
pub struct CreateScheduleRequest {
    /// Human-readable name.
    pub name: String,
    /// Trigger configuration.
    pub trigger: TriggerConfig,
    /// ID of the workflow to execute.
    pub workflow_id: String,
    /// Parameter values for the workflow (JSON object).
    #[serde(default)]
    pub parameter_values: serde_json::Value,
    /// Execution policies.
    #[serde(default)]
    pub policies: SchedulePolicies,
    /// Optional description.
    #[serde(default)]
    pub description: String,
    /// Tags for filtering.
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Request to update an existing schedule.
#[derive(Debug, Clone, Deserialize)]
pub struct UpdateScheduleRequest {
    /// Updated name.
    pub name: Option<String>,
    /// Updated trigger.
    pub trigger: Option<TriggerConfig>,
    /// Updated workflow ID.
    pub workflow_id: Option<String>,
    /// Updated parameter values.
    pub parameter_values: Option<serde_json::Value>,
    /// Updated policies.
    pub policies: Option<SchedulePolicies>,
    /// Updated description.
    pub description: Option<String>,
    /// Updated tags.
    pub tags: Option<Vec<String>>,
}

/// Summary view of a schedule for listing.
#[derive(Debug, Clone, Serialize)]
pub struct ScheduleSummary {
    /// Unique schedule ID.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Whether the schedule is active.
    pub enabled: bool,
    /// Trigger type label (cron, interval, event, onetime).
    pub trigger_type: String,
    /// Human-readable trigger value (cron expression, interval seconds, etc.).
    pub trigger_value: String,
    /// Workflow to execute.
    pub workflow_id: String,
    /// Human-readable description.
    pub description: String,
    /// Tags for filtering.
    pub tags: Vec<String>,
    /// Creation timestamp (RFC 3339).
    pub created_at: String,
    /// Last fire timestamp (RFC 3339), if ever fired.
    pub last_fire: Option<String>,
}

/// Summary of an execution record for the frontend.
#[derive(Debug, Clone, Serialize)]
pub struct ExecutionSummary {
    /// Unique execution ID.
    pub execution_id: String,
    /// Originating schedule ID.
    pub schedule_id: String,
    /// Execution status (pending, running, completed, failed, skipped).
    pub status: String,
    /// When the trigger fired (RFC 3339).
    pub triggered_at: String,
    /// When execution started (RFC 3339).
    pub started_at: Option<String>,
    /// When execution completed (RFC 3339).
    pub completed_at: Option<String>,
    /// Duration in milliseconds (if completed).
    pub duration_ms: Option<u64>,
    /// Linked workflow execution ID.
    pub workflow_execution_id: Option<String>,
    /// Request/input summary (JSON): parameters, trigger context, workflow info.
    pub request_summary: serde_json::Value,
    /// Response/output summary (JSON): execution result, output content.
    pub response_summary: serde_json::Value,
    /// Human-readable error message when status is `failed`.
    pub error_message: Option<String>,
}

// ---------------------------------------------------------------------------
// Service
// ---------------------------------------------------------------------------

/// Scheduler service: CRUD + pause/resume for scheduled tasks.
pub struct SchedulerService;

impl SchedulerService {
    /// Create a new `SchedulerManager` with default config.
    ///
    /// This is a convenience for wiring in the `ServiceContainer`.
    pub fn create_manager() -> SchedulerManager {
        SchedulerManager::new(SchedulerConfig::default())
    }

    /// Hydrate the in-memory `SchedulerManager` from persisted `SQLite` rows.
    ///
    /// Called once during `ServiceContainer::from_config()` after both
    /// the manager and store are created. Converts each `ScheduleRow`
    /// back into a `Schedule` and registers it.
    pub async fn load_schedules_from_db(
        manager: &SchedulerManager,
        store: &SqliteScheduleStore,
    ) -> Result<usize, SchedulerServiceError> {
        let rows = store
            .list()
            .await
            .map_err(|e| SchedulerServiceError::Internal(format!("load schedules from DB: {e}")))?;

        let count = rows.len();
        for row in rows {
            match schedule_from_row(&row) {
                Ok(schedule) => {
                    manager.register(schedule).await;
                }
                Err(e) => {
                    warn!(
                        schedule_id = %row.id,
                        error = %e,
                        "Skipping corrupted schedule row"
                    );
                }
            }
        }

        if count > 0 {
            info!(count, "Hydrated in-memory scheduler from SQLite");
        }
        Ok(count)
    }

    /// List all schedules.
    pub async fn list(manager: &SchedulerManager) -> Vec<ScheduleSummary> {
        manager
            .list_schedules()
            .await
            .iter()
            .map(schedule_to_summary)
            .collect()
    }

    /// Get a single schedule by ID.
    pub async fn get(
        manager: &SchedulerManager,
        id: &str,
    ) -> Result<ScheduleSummary, SchedulerServiceError> {
        manager
            .get_schedule(id)
            .await
            .map(|s| schedule_to_summary(&s))
            .ok_or_else(|| SchedulerServiceError::NotFound { id: id.to_string() })
    }

    /// Create a new schedule.
    ///
    /// Registers in the in-memory manager AND persists to `SQLite`.
    pub async fn create(
        manager: &SchedulerManager,
        req: &CreateScheduleRequest,
        db_store: Option<&SqliteScheduleStore>,
    ) -> Result<ScheduleSummary, SchedulerServiceError> {
        if req.name.is_empty() {
            return Err(SchedulerServiceError::Validation {
                message: "schedule name cannot be empty".into(),
            });
        }
        if req.workflow_id.is_empty() {
            return Err(SchedulerServiceError::Validation {
                message: "workflow_id cannot be empty".into(),
            });
        }

        let id = uuid::Uuid::new_v4().to_string();
        let schedule = Schedule::new(
            id.clone(),
            req.name.clone(),
            req.trigger.clone(),
            req.workflow_id.clone(),
        )
        .with_params(req.parameter_values.clone())
        .with_policies(req.policies.clone())
        .with_description(req.description.clone())
        .with_tags(req.tags.clone());

        // Persist to SQLite first (fail fast on DB errors).
        if let Some(store) = db_store {
            let row = schedule_to_row(&schedule);
            store
                .save(&row)
                .await
                .map_err(|e| SchedulerServiceError::Internal(format!("persist schedule: {e}")))?;
        }

        manager.register(schedule).await;

        Self::get(manager, &id).await
    }

    /// Update an existing schedule.
    ///
    /// Updates the in-memory manager AND persists to `SQLite`.
    pub async fn update(
        manager: &SchedulerManager,
        id: &str,
        req: &UpdateScheduleRequest,
        db_store: Option<&SqliteScheduleStore>,
    ) -> Result<ScheduleSummary, SchedulerServiceError> {
        let existing = manager
            .get_schedule(id)
            .await
            .ok_or_else(|| SchedulerServiceError::NotFound { id: id.to_string() })?;

        let updated = Schedule::new(
            existing.id.clone(),
            req.name.clone().unwrap_or(existing.name),
            req.trigger.clone().unwrap_or(existing.trigger),
            req.workflow_id.clone().unwrap_or(existing.workflow_id),
        )
        .with_params(
            req.parameter_values
                .clone()
                .unwrap_or(existing.parameter_values),
        )
        .with_policies(req.policies.clone().unwrap_or(existing.policies))
        .with_description(req.description.clone().unwrap_or(existing.description))
        .with_tags(req.tags.clone().unwrap_or(existing.tags));

        // Persist to SQLite.
        if let Some(store) = db_store {
            let row = schedule_to_row(&updated);
            store.update(&row).await.map_err(|e| {
                SchedulerServiceError::Internal(format!("update schedule in DB: {e}"))
            })?;
        }

        // Remove + re-register to replace in-memory.
        manager.remove(id).await;
        manager.register(updated).await;

        Self::get(manager, id).await
    }

    /// Delete a schedule.
    ///
    /// Removes from the in-memory manager AND deletes from `SQLite`.
    pub async fn delete(
        manager: &SchedulerManager,
        id: &str,
        db_store: Option<&SqliteScheduleStore>,
    ) -> Result<bool, SchedulerServiceError> {
        // Delete from SQLite first.
        if let Some(store) = db_store {
            store.delete(id).await.map_err(|e| {
                SchedulerServiceError::Internal(format!("delete schedule from DB: {e}"))
            })?;
        }
        Ok(manager.remove(id).await)
    }

    /// Pause a schedule (disable without removing).
    ///
    /// Updates the in-memory manager AND persists enabled=false to `SQLite`.
    pub async fn pause(
        manager: &SchedulerManager,
        id: &str,
        db_store: Option<&SqliteScheduleStore>,
    ) -> Result<(), SchedulerServiceError> {
        if manager.pause(id).await {
            if let Some(store) = db_store {
                let _ = store.set_enabled(id, false).await;
            }
            Ok(())
        } else {
            Err(SchedulerServiceError::NotFound { id: id.to_string() })
        }
    }

    /// Resume a paused schedule.
    ///
    /// Updates the in-memory manager AND persists enabled=true to `SQLite`.
    pub async fn resume(
        manager: &SchedulerManager,
        id: &str,
        db_store: Option<&SqliteScheduleStore>,
    ) -> Result<(), SchedulerServiceError> {
        if manager.resume(id).await {
            if let Some(store) = db_store {
                let _ = store.set_enabled(id, true).await;
            }
            Ok(())
        } else {
            Err(SchedulerServiceError::NotFound { id: id.to_string() })
        }
    }

    // -----------------------------------------------------------------------
    // Execution history
    // -----------------------------------------------------------------------

    /// Get execution history for a schedule.
    pub async fn execution_history(
        manager: &SchedulerManager,
        schedule_id: &str,
    ) -> Vec<ExecutionSummary> {
        manager
            .execution_history(schedule_id)
            .await
            .into_iter()
            .map(|e| execution_to_summary(&e))
            .collect()
    }

    /// Get a single execution record by ID.
    pub async fn get_execution(
        manager: &SchedulerManager,
        execution_id: &str,
    ) -> Result<ExecutionSummary, SchedulerServiceError> {
        manager
            .get_execution(execution_id)
            .await
            .map(|e| execution_to_summary(&e))
            .ok_or_else(|| SchedulerServiceError::NotFound {
                id: execution_id.to_string(),
            })
    }

    /// Manually trigger a schedule execution (for "Trigger Now" / replay).
    ///
    /// When a `WorkflowDispatcher` is available (injected via
    /// `ServiceContainer::init_workflow_dispatcher`), creates a `Running`
    /// execution record and spawns a real workflow dispatch in the background.
    /// Returns the running record immediately (non-blocking for the GUI).
    ///
    /// Falls back to an instant placeholder completion when no dispatcher is
    /// injected (backward compat for tests).
    pub async fn trigger_now(
        manager: &SchedulerManager,
        schedule_id: &str,
    ) -> Result<ExecutionSummary, SchedulerServiceError> {
        // Verify schedule exists.
        let schedule = manager.get_schedule(schedule_id).await.ok_or_else(|| {
            SchedulerServiceError::NotFound {
                id: schedule_id.to_string(),
            }
        })?;

        let now = chrono::Utc::now();
        let execution_id = format!("exec-manual-{}", uuid::Uuid::new_v4());

        let request_summary = serde_json::json!({
            "schedule_id": schedule.id,
            "schedule_name": schedule.name,
            "workflow_id": schedule.workflow_id,
            "trigger": "manual",
            "parameter_values": schedule.parameter_values,
            "trigger_time": now.to_rfc3339(),
        });

        // Check if a dispatcher is available for real execution.
        if let Some(dispatcher) = manager.dispatcher().await {
            // Real dispatch path: create Running record, spawn async work.
            let execution = y_scheduler::ScheduleExecution {
                execution_id: execution_id.clone(),
                schedule_id: schedule_id.to_string(),
                triggered_at: now,
                started_at: Some(now),
                completed_at: None,
                status: y_scheduler::ExecutionStatus::Running,
                workflow_execution_id: None,
                request_summary,
                response_summary: serde_json::json!({}),
                error_message: None,
            };

            {
                let exec_store = manager.execution_store();
                let mut guard = exec_store.lock().await;
                guard.record(execution);
            }

            // Spawn real execution without blocking.
            let workflow_id = schedule.workflow_id.clone();
            let parameter_values = schedule.parameter_values.clone();
            let exec_store = std::sync::Arc::clone(manager.execution_store());
            let exec_id = execution_id.clone();

            tokio::spawn(async move {
                let dispatch_start = std::time::Instant::now();
                match dispatcher.dispatch(&workflow_id, parameter_values).await {
                    Ok(result) => {
                        let duration_ms =
                            u64::try_from(dispatch_start.elapsed().as_millis()).unwrap_or(0);
                        let mut store = exec_store.lock().await;
                        store.update(&exec_id, |rec| {
                            rec.status = if result.success {
                                y_scheduler::ExecutionStatus::Completed
                            } else {
                                y_scheduler::ExecutionStatus::Failed
                            };
                            rec.completed_at = Some(chrono::Utc::now());
                            rec.response_summary = serde_json::json!({
                                "status": if result.success { "completed" } else { "failed" },
                                "summary": result.summary,
                                "output": result.output,
                                "duration_ms": duration_ms,
                            });
                            if !result.success {
                                rec.error_message = result.error;
                            }
                        });
                    }
                    Err(e) => {
                        let mut store = exec_store.lock().await;
                        store.update(&exec_id, |rec| {
                            rec.status = y_scheduler::ExecutionStatus::Failed;
                            rec.completed_at = Some(chrono::Utc::now());
                            rec.response_summary = serde_json::json!({
                                "status": "failed",
                                "error": e.to_string(),
                            });
                            rec.error_message = Some(e.to_string());
                        });
                    }
                }
            });
        } else {
            // Placeholder path: instant completion (no dispatcher injected).
            let response_summary = serde_json::json!({
                "status": "completed",
                "message": "Manual trigger executed (placeholder)",
                "workflow_execution_id": format!("workflow-{execution_id}"),
            });

            let execution = y_scheduler::ScheduleExecution {
                execution_id: execution_id.clone(),
                schedule_id: schedule_id.to_string(),
                triggered_at: now,
                started_at: Some(now),
                completed_at: Some(now),
                status: y_scheduler::ExecutionStatus::Completed,
                workflow_execution_id: Some(format!("workflow-{execution_id}")),
                request_summary,
                response_summary,
                error_message: None,
            };

            let exec_store = manager.execution_store();
            let mut guard = exec_store.lock().await;
            guard.record(execution);
        }

        Self::get_execution(manager, &execution_id).await
    }

    /// Manually execute a workflow (for replay / manual run).
    ///
    /// When a `WorkflowDispatcher` is available, creates a `Running`
    /// execution record and spawns real workflow execution. Returns the
    /// running record immediately (non-blocking).
    ///
    /// Falls back to an instant placeholder when no dispatcher is injected.
    pub async fn execute_workflow(
        manager: &SchedulerManager,
        workflow_id: &str,
        workflow_name: &str,
    ) -> Result<ExecutionSummary, SchedulerServiceError> {
        let now = chrono::Utc::now();
        let execution_id = format!("exec-wf-{}", uuid::Uuid::new_v4());

        let request_summary = serde_json::json!({
            "workflow_id": workflow_id,
            "workflow_name": workflow_name,
            "trigger": "manual_execute",
            "trigger_time": now.to_rfc3339(),
        });

        if let Some(dispatcher) = manager.dispatcher().await {
            // Real dispatch path.
            let execution = y_scheduler::ScheduleExecution {
                execution_id: execution_id.clone(),
                schedule_id: format!("workflow-{workflow_id}"),
                triggered_at: now,
                started_at: Some(now),
                completed_at: None,
                status: y_scheduler::ExecutionStatus::Running,
                workflow_execution_id: None,
                request_summary,
                response_summary: serde_json::json!({}),
                error_message: None,
            };

            {
                let exec_store = manager.execution_store();
                let mut guard = exec_store.lock().await;
                guard.record(execution);
            }

            let wf_id = workflow_id.to_string();
            let exec_store = std::sync::Arc::clone(manager.execution_store());
            let exec_id = execution_id.clone();

            tokio::spawn(async move {
                let dispatch_start = std::time::Instant::now();
                match dispatcher
                    .dispatch(&wf_id, serde_json::Value::Object(serde_json::Map::new()))
                    .await
                {
                    Ok(result) => {
                        let duration_ms =
                            u64::try_from(dispatch_start.elapsed().as_millis()).unwrap_or(0);
                        let mut store = exec_store.lock().await;
                        store.update(&exec_id, |rec| {
                            rec.status = if result.success {
                                y_scheduler::ExecutionStatus::Completed
                            } else {
                                y_scheduler::ExecutionStatus::Failed
                            };
                            rec.completed_at = Some(chrono::Utc::now());
                            rec.response_summary = serde_json::json!({
                                "status": if result.success { "completed" } else { "failed" },
                                "summary": result.summary,
                                "output": result.output,
                                "duration_ms": duration_ms,
                            });
                            if !result.success {
                                rec.error_message = result.error;
                            }
                        });
                    }
                    Err(e) => {
                        let mut store = exec_store.lock().await;
                        store.update(&exec_id, |rec| {
                            rec.status = y_scheduler::ExecutionStatus::Failed;
                            rec.completed_at = Some(chrono::Utc::now());
                            rec.response_summary = serde_json::json!({
                                "status": "failed",
                                "error": e.to_string(),
                            });
                            rec.error_message = Some(e.to_string());
                        });
                    }
                }
            });
        } else {
            // Placeholder path.
            let response_summary = serde_json::json!({
                "status": "completed",
                "message": "Workflow executed manually (placeholder)",
            });

            let execution = y_scheduler::ScheduleExecution {
                execution_id: execution_id.clone(),
                schedule_id: format!("workflow-{workflow_id}"),
                triggered_at: now,
                started_at: Some(now),
                completed_at: Some(now),
                status: y_scheduler::ExecutionStatus::Completed,
                workflow_execution_id: Some(execution_id.clone()),
                request_summary,
                response_summary,
                error_message: None,
            };

            let exec_store = manager.execution_store();
            let mut guard = exec_store.lock().await;
            guard.record(execution);
        }

        Self::get_execution(manager, &execution_id).await
    }
}

// ---------------------------------------------------------------------------
// Conversion helpers
// ---------------------------------------------------------------------------

fn schedule_to_summary(schedule: &Schedule) -> ScheduleSummary {
    let (trigger_type, trigger_value) = trigger_type_and_value(&schedule.trigger);

    ScheduleSummary {
        id: schedule.id.clone(),
        name: schedule.name.clone(),
        enabled: schedule.enabled,
        trigger_type: trigger_type.to_string(),
        trigger_value,
        workflow_id: schedule.workflow_id.clone(),
        description: schedule.description.clone(),
        tags: schedule.tags.clone(),
        created_at: schedule.created_at.to_rfc3339(),
        last_fire: schedule.last_fire.map(|t| t.to_rfc3339()),
    }
}

/// Extract trigger type label and expression value from a `TriggerConfig`.
fn trigger_type_and_value(trigger: &TriggerConfig) -> (&'static str, String) {
    match trigger {
        TriggerConfig::Cron {
            expression,
            timezone,
        } => ("cron", format!("{expression} ({timezone})")),
        TriggerConfig::Interval { interval_secs } => ("interval", format!("{interval_secs}s")),
        TriggerConfig::Event { event_type, .. } => ("event", event_type.clone()),
        TriggerConfig::OneTime { at } => ("onetime", at.to_rfc3339()),
    }
}

fn execution_to_summary(exec: &y_scheduler::ScheduleExecution) -> ExecutionSummary {
    let duration_ms = match (exec.started_at, exec.completed_at) {
        (Some(start), Some(end)) => {
            let dur = end - start;
            Some(u64::try_from(dur.num_milliseconds()).unwrap_or(0))
        }
        _ => None,
    };

    ExecutionSummary {
        execution_id: exec.execution_id.clone(),
        schedule_id: exec.schedule_id.clone(),
        status: exec.status.to_string(),
        triggered_at: exec.triggered_at.to_rfc3339(),
        started_at: exec.started_at.map(|t| t.to_rfc3339()),
        completed_at: exec.completed_at.map(|t| t.to_rfc3339()),
        duration_ms,
        workflow_execution_id: exec.workflow_execution_id.clone(),
        request_summary: exec.request_summary.clone(),
        response_summary: exec.response_summary.clone(),
        error_message: exec.error_message.clone(),
    }
}

// ---------------------------------------------------------------------------
// Schedule <-> ScheduleRow conversion
// ---------------------------------------------------------------------------

/// Convert an in-memory `Schedule` to a `SQLite` `ScheduleRow` for persistence.
fn schedule_to_row(schedule: &Schedule) -> y_storage::ScheduleRow {
    let (schedule_type, schedule_expr) = match &schedule.trigger {
        TriggerConfig::Cron {
            expression,
            timezone,
        } => ("cron".to_string(), format!("{expression}|{timezone}")),
        TriggerConfig::Interval { interval_secs } => {
            ("interval".to_string(), interval_secs.to_string())
        }
        TriggerConfig::Event {
            event_type,
            debounce_secs,
        } => ("event".to_string(), format!("{event_type}|{debounce_secs}")),
        TriggerConfig::OneTime { at } => ("onetime".to_string(), at.to_rfc3339()),
    };

    let parameter_bindings = if schedule.parameter_values.is_null()
        || schedule.parameter_values == serde_json::json!({})
    {
        None
    } else {
        Some(schedule.parameter_values.to_string())
    };

    let tags_json = serde_json::to_string(&schedule.tags).unwrap_or_else(|_| "[]".to_string());

    let missed_policy = match schedule.policies.missed_policy {
        y_scheduler::MissedPolicy::Skip => "skip",
        y_scheduler::MissedPolicy::CatchUp => "catch_up",
        y_scheduler::MissedPolicy::Backfill => "backfill",
    }
    .to_string();
    let concurrency_policy = match schedule.policies.concurrency_policy {
        y_scheduler::ConcurrencyPolicy::SkipIfRunning => "skip",
        y_scheduler::ConcurrencyPolicy::Queue => "queue",
        y_scheduler::ConcurrencyPolicy::CancelPrevious | y_scheduler::ConcurrencyPolicy::Allow => {
            "replace"
        }
    }
    .to_string();

    y_storage::ScheduleRow {
        id: schedule.id.clone(),
        name: schedule.name.clone(),
        description: if schedule.description.is_empty() {
            None
        } else {
            Some(schedule.description.clone())
        },
        schedule_type,
        schedule_expr,
        workflow_id: schedule.workflow_id.clone(),
        parameter_bindings,
        parameter_schema: None,
        enabled: schedule.enabled,
        creator: "user".to_string(),
        missed_policy,
        concurrency_policy,
        max_executions_per_hour: i64::from(schedule.policies.max_executions_per_hour),
        tags: tags_json,
        last_fire: schedule.last_fire.map(|t| t.to_rfc3339()),
        created_at: schedule.created_at.to_rfc3339(),
        updated_at: schedule.updated_at.to_rfc3339(),
    }
}

/// Convert a `SQLite` `ScheduleRow` back to an in-memory `Schedule`.
///
/// Returns `Err` if the trigger configuration or timestamps cannot be parsed.
fn schedule_from_row(row: &y_storage::ScheduleRow) -> Result<Schedule, SchedulerServiceError> {
    let trigger = match row.schedule_type.as_str() {
        "cron" => {
            let parts: Vec<&str> = row.schedule_expr.splitn(2, '|').collect();
            let expression = parts.first().unwrap_or(&"").to_string();
            let timezone = parts.get(1).unwrap_or(&"UTC").to_string();
            TriggerConfig::Cron {
                expression,
                timezone,
            }
        }
        "interval" => {
            let interval_secs = row.schedule_expr.parse::<u64>().map_err(|e| {
                SchedulerServiceError::Internal(format!(
                    "invalid interval '{}': {e}",
                    row.schedule_expr
                ))
            })?;
            TriggerConfig::Interval { interval_secs }
        }
        "event" => {
            let parts: Vec<&str> = row.schedule_expr.splitn(2, '|').collect();
            let event_type = parts.first().unwrap_or(&"").to_string();
            let debounce_secs = parts
                .get(1)
                .copied()
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0);
            TriggerConfig::Event {
                event_type,
                debounce_secs,
            }
        }
        "onetime" => {
            let at = row.schedule_expr.parse::<DateTime<Utc>>().map_err(|e| {
                SchedulerServiceError::Internal(format!(
                    "invalid onetime timestamp '{}': {e}",
                    row.schedule_expr
                ))
            })?;
            TriggerConfig::OneTime { at }
        }
        other => {
            return Err(SchedulerServiceError::Internal(format!(
                "unknown schedule type: {other}"
            )));
        }
    };

    let parameter_values = row
        .parameter_bindings
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or(serde_json::json!({}));

    let missed_policy = match row.missed_policy.as_str() {
        "catch_up" | "catchup" => y_scheduler::MissedPolicy::CatchUp,
        "backfill" => y_scheduler::MissedPolicy::Backfill,
        _ => y_scheduler::MissedPolicy::Skip,
    };

    let concurrency_policy = match row.concurrency_policy.as_str() {
        "allow" => y_scheduler::ConcurrencyPolicy::Allow,
        "queue" => y_scheduler::ConcurrencyPolicy::Queue,
        "cancel_previous" => y_scheduler::ConcurrencyPolicy::CancelPrevious,
        _ => y_scheduler::ConcurrencyPolicy::SkipIfRunning,
    };

    let tags: Vec<String> = serde_json::from_str(&row.tags).unwrap_or_default();

    let created_at = row
        .created_at
        .parse::<DateTime<Utc>>()
        .unwrap_or_else(|_| Utc::now());
    let updated_at = row
        .updated_at
        .parse::<DateTime<Utc>>()
        .unwrap_or_else(|_| Utc::now());
    let last_fire = row
        .last_fire
        .as_deref()
        .and_then(|s: &str| s.parse::<DateTime<Utc>>().ok());

    let mut schedule = Schedule::new(
        row.id.clone(),
        row.name.clone(),
        trigger,
        row.workflow_id.clone(),
    )
    .with_params(parameter_values)
    .with_policies(SchedulePolicies {
        missed_policy,
        concurrency_policy,
        max_executions_per_hour: u32::try_from(row.max_executions_per_hour).unwrap_or(0),
    })
    .with_description(row.description.clone().unwrap_or_default())
    .with_tags(tags);

    schedule.enabled = row.enabled;
    schedule.created_at = created_at;
    schedule.updated_at = updated_at;
    schedule.last_fire = last_fire;

    Ok(schedule)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_create_req() -> CreateScheduleRequest {
        CreateScheduleRequest {
            name: "test-schedule".to_string(),
            trigger: TriggerConfig::Interval {
                interval_secs: 3600,
            },
            workflow_id: "wf-123".to_string(),
            parameter_values: serde_json::json!({"key": "value"}),
            policies: SchedulePolicies::default(),
            description: "A test schedule".to_string(),
            tags: vec!["test".to_string()],
        }
    }

    #[tokio::test]
    async fn test_create_schedule() {
        let manager = SchedulerService::create_manager();
        let req = make_create_req();
        let result = SchedulerService::create(&manager, &req, None)
            .await
            .unwrap();
        assert_eq!(result.name, "test-schedule");
        assert_eq!(result.trigger_type, "interval");
        assert_eq!(result.workflow_id, "wf-123");
        assert!(result.enabled);
    }

    #[tokio::test]
    async fn test_list_schedules() {
        let manager = SchedulerService::create_manager();
        let req = make_create_req();
        SchedulerService::create(&manager, &req, None)
            .await
            .unwrap();
        let list = SchedulerService::list(&manager).await;
        assert_eq!(list.len(), 1);
    }

    #[tokio::test]
    async fn test_get_schedule_not_found() {
        let manager = SchedulerService::create_manager();
        let result = SchedulerService::get(&manager, "nonexistent").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_delete_schedule() {
        let manager = SchedulerService::create_manager();
        let req = make_create_req();
        let created = SchedulerService::create(&manager, &req, None)
            .await
            .unwrap();
        let deleted = SchedulerService::delete(&manager, &created.id, None)
            .await
            .unwrap();
        assert!(deleted);
        assert!(SchedulerService::list(&manager).await.is_empty());
    }

    #[tokio::test]
    async fn test_pause_resume_schedule() {
        let manager = SchedulerService::create_manager();
        let req = make_create_req();
        let created = SchedulerService::create(&manager, &req, None)
            .await
            .unwrap();

        SchedulerService::pause(&manager, &created.id, None)
            .await
            .unwrap();
        let paused = SchedulerService::get(&manager, &created.id).await.unwrap();
        assert!(!paused.enabled);

        SchedulerService::resume(&manager, &created.id, None)
            .await
            .unwrap();
        let resumed = SchedulerService::get(&manager, &created.id).await.unwrap();
        assert!(resumed.enabled);
    }

    #[tokio::test]
    async fn test_create_empty_name_rejected() {
        let manager = SchedulerService::create_manager();
        let mut req = make_create_req();
        req.name = String::new();
        let result = SchedulerService::create(&manager, &req, None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_update_schedule() {
        let manager = SchedulerService::create_manager();
        let req = make_create_req();
        let created = SchedulerService::create(&manager, &req, None)
            .await
            .unwrap();

        let update = UpdateScheduleRequest {
            name: Some("updated-name".to_string()),
            trigger: None,
            workflow_id: None,
            parameter_values: None,
            policies: None,
            description: Some("updated desc".to_string()),
            tags: None,
        };
        let updated = SchedulerService::update(&manager, &created.id, &update, None)
            .await
            .unwrap();
        assert_eq!(updated.name, "updated-name");
        assert_eq!(updated.description, "updated desc");
    }

    // -----------------------------------------------------------------------
    // Conversion round-trip tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_schedule_row_roundtrip_interval() {
        let schedule = Schedule::new(
            "s1",
            "Interval Test",
            TriggerConfig::Interval { interval_secs: 300 },
            "wf-1",
        )
        .with_description("round-trip test")
        .with_tags(vec!["tag1".into(), "tag2".into()])
        .with_params(serde_json::json!({"key": "value"}));

        let row = schedule_to_row(&schedule);
        assert_eq!(row.schedule_type, "interval");
        assert_eq!(row.schedule_expr, "300");

        let restored = schedule_from_row(&row).unwrap();
        assert_eq!(restored.id, schedule.id);
        assert_eq!(restored.name, schedule.name);
        assert_eq!(restored.workflow_id, schedule.workflow_id);
        assert_eq!(restored.description, schedule.description);
        assert_eq!(restored.tags, schedule.tags);
        assert_eq!(restored.parameter_values, schedule.parameter_values);
        assert!(matches!(
            restored.trigger,
            TriggerConfig::Interval { interval_secs: 300 }
        ));
    }

    #[test]
    fn test_schedule_row_roundtrip_cron() {
        let schedule = Schedule::new(
            "s2",
            "Cron Test",
            TriggerConfig::Cron {
                expression: "0 2 * * *".into(),
                timezone: "Asia/Shanghai".into(),
            },
            "wf-2",
        );

        let row = schedule_to_row(&schedule);
        assert_eq!(row.schedule_type, "cron");
        assert_eq!(row.schedule_expr, "0 2 * * *|Asia/Shanghai");

        let restored = schedule_from_row(&row).unwrap();
        match &restored.trigger {
            TriggerConfig::Cron {
                expression,
                timezone,
            } => {
                assert_eq!(expression, "0 2 * * *");
                assert_eq!(timezone, "Asia/Shanghai");
            }
            other => panic!("expected Cron, got {other:?}"),
        }
    }

    #[test]
    fn test_schedule_row_roundtrip_event() {
        let schedule = Schedule::new(
            "s3",
            "Event Test",
            TriggerConfig::Event {
                event_type: "file_changed".into(),
                debounce_secs: 5,
            },
            "wf-3",
        );

        let row = schedule_to_row(&schedule);
        let restored = schedule_from_row(&row).unwrap();
        match &restored.trigger {
            TriggerConfig::Event {
                event_type,
                debounce_secs,
            } => {
                assert_eq!(event_type, "file_changed");
                assert_eq!(*debounce_secs, 5);
            }
            other => panic!("expected Event, got {other:?}"),
        }
    }

    #[test]
    fn test_schedule_row_roundtrip_disabled() {
        let mut schedule = Schedule::new(
            "s4",
            "Disabled Test",
            TriggerConfig::Interval { interval_secs: 60 },
            "wf-4",
        );
        schedule.enabled = false;

        let row = schedule_to_row(&schedule);
        let restored = schedule_from_row(&row).unwrap();
        assert!(!restored.enabled);
    }
}
