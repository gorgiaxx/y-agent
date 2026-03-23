//! Scheduled task management service.
//!
//! Wraps `y-scheduler::SchedulerManager` with a service-layer API for
//! GUI / REST consumption. Provides CRUD, pause/resume, and schedule listing.

use serde::{Deserialize, Serialize};

use y_scheduler::{Schedule, SchedulePolicies, SchedulerConfig, SchedulerManager, TriggerConfig};

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
    pub async fn create(
        manager: &SchedulerManager,
        req: &CreateScheduleRequest,
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

        manager.register(schedule).await;

        Self::get(manager, &id).await
    }

    /// Update an existing schedule.
    pub async fn update(
        manager: &SchedulerManager,
        id: &str,
        req: &UpdateScheduleRequest,
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

        // Remove + re-register to replace.
        manager.remove(id).await;
        manager.register(updated).await;

        Self::get(manager, id).await
    }

    /// Delete a schedule.
    pub async fn delete(
        manager: &SchedulerManager,
        id: &str,
    ) -> Result<bool, SchedulerServiceError> {
        Ok(manager.remove(id).await)
    }

    /// Pause a schedule (disable without removing).
    pub async fn pause(manager: &SchedulerManager, id: &str) -> Result<(), SchedulerServiceError> {
        if manager.pause(id).await {
            Ok(())
        } else {
            Err(SchedulerServiceError::NotFound { id: id.to_string() })
        }
    }

    /// Resume a paused schedule.
    pub async fn resume(manager: &SchedulerManager, id: &str) -> Result<(), SchedulerServiceError> {
        if manager.resume(id).await {
            Ok(())
        } else {
            Err(SchedulerServiceError::NotFound { id: id.to_string() })
        }
    }
}

// ---------------------------------------------------------------------------
// Conversion helpers
// ---------------------------------------------------------------------------

fn schedule_to_summary(schedule: &Schedule) -> ScheduleSummary {
    let trigger_type = match &schedule.trigger {
        TriggerConfig::Cron { .. } => "cron",
        TriggerConfig::Interval { .. } => "interval",
        TriggerConfig::Event { .. } => "event",
        TriggerConfig::OneTime { .. } => "onetime",
    };

    ScheduleSummary {
        id: schedule.id.clone(),
        name: schedule.name.clone(),
        enabled: schedule.enabled,
        trigger_type: trigger_type.to_string(),
        workflow_id: schedule.workflow_id.clone(),
        description: schedule.description.clone(),
        tags: schedule.tags.clone(),
        created_at: schedule.created_at.to_rfc3339(),
        last_fire: schedule.last_fire.map(|t| t.to_rfc3339()),
    }
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
        let result = SchedulerService::create(&manager, &req).await.unwrap();
        assert_eq!(result.name, "test-schedule");
        assert_eq!(result.trigger_type, "interval");
        assert_eq!(result.workflow_id, "wf-123");
        assert!(result.enabled);
    }

    #[tokio::test]
    async fn test_list_schedules() {
        let manager = SchedulerService::create_manager();
        let req = make_create_req();
        SchedulerService::create(&manager, &req).await.unwrap();
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
        let created = SchedulerService::create(&manager, &req).await.unwrap();
        let deleted = SchedulerService::delete(&manager, &created.id)
            .await
            .unwrap();
        assert!(deleted);
        assert!(SchedulerService::list(&manager).await.is_empty());
    }

    #[tokio::test]
    async fn test_pause_resume_schedule() {
        let manager = SchedulerService::create_manager();
        let req = make_create_req();
        let created = SchedulerService::create(&manager, &req).await.unwrap();

        SchedulerService::pause(&manager, &created.id)
            .await
            .unwrap();
        let paused = SchedulerService::get(&manager, &created.id).await.unwrap();
        assert!(!paused.enabled);

        SchedulerService::resume(&manager, &created.id)
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
        let result = SchedulerService::create(&manager, &req).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_update_schedule() {
        let manager = SchedulerService::create_manager();
        let req = make_create_req();
        let created = SchedulerService::create(&manager, &req).await.unwrap();

        let update = UpdateScheduleRequest {
            name: Some("updated-name".to_string()),
            trigger: None,
            workflow_id: None,
            parameter_values: None,
            policies: None,
            description: Some("updated desc".to_string()),
            tags: None,
        };
        let updated = SchedulerService::update(&manager, &created.id, &update)
            .await
            .unwrap();
        assert_eq!(updated.name, "updated-name");
        assert_eq!(updated.description, "updated desc");
    }
}
