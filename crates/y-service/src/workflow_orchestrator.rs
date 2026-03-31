//! Workflow orchestrator: intercepts all workflow and schedule meta-tool calls
//! from the LLM and routes them through `WorkflowService` and `SchedulerService`
//! for persistence.
//!
//! ## Handled tools
//!
//! | Tool | Handler |
//! |------|---------|
//! | `WorkflowCreate` | [`WorkflowOrchestrator::handle_create`] |
//! | `WorkflowList` | [`WorkflowOrchestrator::handle_list`] |
//! | `WorkflowGet` | [`WorkflowOrchestrator::handle_get`] |
//! | `WorkflowUpdate` | [`WorkflowOrchestrator::handle_update`] |
//! | `WorkflowDelete` | [`WorkflowOrchestrator::handle_delete`] |
//! | `WorkflowValidate` | [`WorkflowOrchestrator::handle_validate`] |
//! | `ScheduleCreate` | [`WorkflowOrchestrator::handle_schedule_create`] |
//! | `ScheduleList` | [`WorkflowOrchestrator::handle_schedule_list`] |
//! | `SchedulePause` | [`WorkflowOrchestrator::handle_schedule_pause`] |
//! | `ScheduleResume` | [`WorkflowOrchestrator::handle_schedule_resume`] |
//! | `ScheduleDelete` | [`WorkflowOrchestrator::handle_schedule_delete`] |
//!
//! This mirrors the pattern established by `ToolSearchOrchestrator` and
//! `TaskDelegationOrchestrator`.

use y_core::tool::{ToolError, ToolOutput};
use y_scheduler::TriggerConfig;
use y_storage::workflow_store::WorkflowRow;

use crate::scheduler_service::{CreateScheduleRequest, SchedulerService};
use crate::workflow_service::{CreateWorkflowRequest, UpdateWorkflowRequest, WorkflowService};
use crate::ServiceContainer;

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

/// Extract a required string argument or return a validation error.
fn require_arg<'a>(arguments: &'a serde_json::Value, field: &str) -> Result<&'a str, ToolError> {
    arguments
        .get(field)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ToolError::ValidationError {
            message: format!("'{field}' is required"),
        })
}

/// Build a standard row summary for JSON output.
fn row_summary(r: &WorkflowRow) -> serde_json::Value {
    serde_json::json!({
        "id": r.id,
        "name": r.name,
        "format": r.format,
        "description": r.description,
        "tags": r.tags,
    })
}

// ---------------------------------------------------------------------------
// Orchestrator
// ---------------------------------------------------------------------------

/// Intercepts workflow/schedule meta-tool calls and fulfills them using
/// service-layer APIs.
pub struct WorkflowOrchestrator;

impl WorkflowOrchestrator {
    // -----------------------------------------------------------------------
    // Workflow handlers
    // -----------------------------------------------------------------------

    /// Handle a `WorkflowCreate` tool call.
    ///
    /// Validates and persists the workflow template, returning the created
    /// workflow summary to the agent.
    pub async fn handle_create(
        arguments: &serde_json::Value,
        container: &ServiceContainer,
    ) -> Result<ToolOutput, ToolError> {
        let name = require_arg(arguments, "name")?;
        let definition = require_arg(arguments, "definition")?;
        let format = arguments
            .get("format")
            .and_then(|v| v.as_str())
            .unwrap_or("expression_dsl");
        let description = arguments.get("description").and_then(|v| v.as_str());
        let tags = arguments.get("tags").and_then(|v| v.as_str());

        let req = CreateWorkflowRequest {
            name: name.to_string(),
            definition: definition.to_string(),
            format: format.to_string(),
            description: description.map(ToString::to_string),
            tags: tags.map(ToString::to_string),
        };

        let row = WorkflowService::create(&container.workflow_store, &req)
            .await
            .map_err(|e| ToolError::Other {
                message: format!("Failed to create workflow: {e}"),
            })?;

        Ok(ToolOutput {
            success: true,
            content: serde_json::json!({
                "action": "workflow_created",
                "id": row.id,
                "name": row.name,
                "format": row.format,
                "description": row.description,
                "tags": row.tags,
                "status": "created"
            }),
            warnings: vec![],
            metadata: serde_json::json!({}),
        })
    }

    /// Handle a `WorkflowList` tool call.
    ///
    /// Returns all workflow templates, optionally filtered by tag.
    pub async fn handle_list(
        arguments: &serde_json::Value,
        container: &ServiceContainer,
    ) -> Result<ToolOutput, ToolError> {
        let tag = arguments.get("tag").and_then(|v| v.as_str());

        let rows: Vec<WorkflowRow> = if let Some(tag) = tag {
            container
                .workflow_store
                .list_by_tag(tag)
                .await
                .map_err(|e| ToolError::Other {
                    message: format!("Failed to list workflows: {e}"),
                })?
        } else {
            WorkflowService::list(&container.workflow_store)
                .await
                .map_err(|e| ToolError::Other {
                    message: format!("Failed to list workflows: {e}"),
                })?
        };

        let summaries: Vec<serde_json::Value> = rows.iter().map(row_summary).collect();

        Ok(ToolOutput {
            success: true,
            content: serde_json::json!({
                "action": "WorkflowList",
                "count": summaries.len(),
                "workflows": summaries,
            }),
            warnings: vec![],
            metadata: serde_json::json!({}),
        })
    }

    /// Handle a `WorkflowGet` tool call.
    ///
    /// Returns full details of a workflow template by ID or name.
    pub async fn handle_get(
        arguments: &serde_json::Value,
        container: &ServiceContainer,
    ) -> Result<ToolOutput, ToolError> {
        let id = require_arg(arguments, "id")?;

        let row = WorkflowService::get(&container.workflow_store, id)
            .await
            .map_err(|e| ToolError::Other {
                message: format!("Workflow not found: {e}"),
            })?;

        Ok(ToolOutput {
            success: true,
            content: serde_json::json!({
                "action": "WorkflowGet",
                "id": row.id,
                "name": row.name,
                "definition": row.definition,
                "format": row.format,
                "description": row.description,
                "tags": row.tags,
                "created_at": row.created_at,
                "updated_at": row.updated_at,
            }),
            warnings: vec![],
            metadata: serde_json::json!({}),
        })
    }

    /// Handle a `WorkflowUpdate` tool call.
    ///
    /// Updates an existing workflow template and returns the updated row.
    pub async fn handle_update(
        arguments: &serde_json::Value,
        container: &ServiceContainer,
    ) -> Result<ToolOutput, ToolError> {
        let id = require_arg(arguments, "id")?;
        let definition = arguments.get("definition").and_then(|v| v.as_str());
        let format = arguments.get("format").and_then(|v| v.as_str());
        let description = arguments.get("description").and_then(|v| v.as_str());
        let tags = arguments.get("tags").and_then(|v| v.as_str());

        let req = UpdateWorkflowRequest {
            definition: definition.map(ToString::to_string),
            format: format.map(ToString::to_string),
            description: description.map(ToString::to_string),
            tags: tags.map(ToString::to_string),
        };

        let row = WorkflowService::update(&container.workflow_store, id, &req)
            .await
            .map_err(|e| ToolError::Other {
                message: format!("Failed to update workflow: {e}"),
            })?;

        Ok(ToolOutput {
            success: true,
            content: serde_json::json!({
                "action": "workflow_updated",
                "id": row.id,
                "name": row.name,
                "format": row.format,
                "description": row.description,
                "tags": row.tags,
                "status": "updated"
            }),
            warnings: vec![],
            metadata: serde_json::json!({}),
        })
    }

    /// Handle a `WorkflowDelete` tool call.
    ///
    /// Deletes a workflow template by ID.
    pub async fn handle_delete(
        arguments: &serde_json::Value,
        container: &ServiceContainer,
    ) -> Result<ToolOutput, ToolError> {
        let id = require_arg(arguments, "id")?;

        let deleted = WorkflowService::delete(&container.workflow_store, id)
            .await
            .map_err(|e| ToolError::Other {
                message: format!("Failed to delete workflow: {e}"),
            })?;

        Ok(ToolOutput {
            success: true,
            content: serde_json::json!({
                "action": "workflow_deleted",
                "id": id,
                "deleted": deleted,
                "status": if deleted { "deleted" } else { "not_found" }
            }),
            warnings: vec![],
            metadata: serde_json::json!({}),
        })
    }

    /// Handle a `WorkflowValidate` tool call.
    ///
    /// Validates a workflow definition without persisting it.
    pub fn handle_validate(
        arguments: &serde_json::Value,
        _container: &ServiceContainer,
    ) -> Result<ToolOutput, ToolError> {
        let definition = require_arg(arguments, "definition")?;
        let format = arguments
            .get("format")
            .and_then(|v| v.as_str())
            .unwrap_or("expression_dsl");

        let result = WorkflowService::validate_definition(definition, format);

        Ok(ToolOutput {
            success: true,
            content: serde_json::json!({
                "action": "WorkflowValidate",
                "valid": result.valid,
                "errors": result.errors,
                "ast_display": result.ast_display,
            }),
            warnings: vec![],
            metadata: serde_json::json!({}),
        })
    }

    // -----------------------------------------------------------------------
    // Schedule handlers
    // -----------------------------------------------------------------------

    /// Handle a `ScheduleCreate` tool call.
    ///
    /// Creates a scheduled task linked to a workflow.
    pub async fn handle_schedule_create(
        arguments: &serde_json::Value,
        container: &ServiceContainer,
    ) -> Result<ToolOutput, ToolError> {
        let name = require_arg(arguments, "name")?;
        let trigger_type = require_arg(arguments, "trigger_type")?;
        let trigger_value = require_arg(arguments, "trigger_value")?;
        let workflow_id = require_arg(arguments, "workflow_id")?;
        let description = arguments
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let trigger = parse_trigger(trigger_type, trigger_value)?;

        let req = CreateScheduleRequest {
            name: name.to_string(),
            trigger,
            workflow_id: workflow_id.to_string(),
            parameter_values: serde_json::json!({}),
            policies: y_scheduler::SchedulePolicies::default(),
            description: description.to_string(),
            tags: vec![],
        };

        let summary = SchedulerService::create(
            &container.scheduler_manager,
            &req,
            Some(&container.schedule_store),
        )
        .await
        .map_err(|e| ToolError::Other {
            message: format!("Failed to create schedule: {e}"),
        })?;

        Ok(ToolOutput {
            success: true,
            content: serde_json::json!({
                "action": "schedule_created",
                "id": summary.id,
                "name": summary.name,
                "trigger_type": summary.trigger_type,
                "workflow_id": summary.workflow_id,
                "enabled": summary.enabled,
                "status": "created"
            }),
            warnings: vec![],
            metadata: serde_json::json!({}),
        })
    }

    /// Handle a `ScheduleList` tool call.
    ///
    /// Lists all schedules, optionally filtered by workflow ID.
    pub async fn handle_schedule_list(
        arguments: &serde_json::Value,
        container: &ServiceContainer,
    ) -> Result<ToolOutput, ToolError> {
        let workflow_id = arguments.get("workflow_id").and_then(|v| v.as_str());

        let all = SchedulerService::list(&container.scheduler_manager).await;

        let filtered: Vec<serde_json::Value> = all
            .iter()
            .filter(|s| workflow_id.is_none_or(|wid| s.workflow_id == wid))
            .map(|s| {
                serde_json::json!({
                    "id": s.id,
                    "name": s.name,
                    "trigger_type": s.trigger_type,
                    "workflow_id": s.workflow_id,
                    "enabled": s.enabled,
                    "last_fire": s.last_fire,
                })
            })
            .collect();

        Ok(ToolOutput {
            success: true,
            content: serde_json::json!({
                "action": "ScheduleList",
                "count": filtered.len(),
                "schedules": filtered,
            }),
            warnings: vec![],
            metadata: serde_json::json!({}),
        })
    }

    /// Handle a `SchedulePause` tool call.
    pub async fn handle_schedule_pause(
        arguments: &serde_json::Value,
        container: &ServiceContainer,
    ) -> Result<ToolOutput, ToolError> {
        let id = require_arg(arguments, "id")?;

        SchedulerService::pause(
            &container.scheduler_manager,
            id,
            Some(&container.schedule_store),
        )
        .await
        .map_err(|e| ToolError::Other {
            message: format!("Failed to pause schedule: {e}"),
        })?;

        Ok(ToolOutput {
            success: true,
            content: serde_json::json!({
                "action": "schedule_paused",
                "id": id,
                "status": "paused"
            }),
            warnings: vec![],
            metadata: serde_json::json!({}),
        })
    }

    /// Handle a `ScheduleResume` tool call.
    pub async fn handle_schedule_resume(
        arguments: &serde_json::Value,
        container: &ServiceContainer,
    ) -> Result<ToolOutput, ToolError> {
        let id = require_arg(arguments, "id")?;

        SchedulerService::resume(
            &container.scheduler_manager,
            id,
            Some(&container.schedule_store),
        )
        .await
        .map_err(|e| ToolError::Other {
            message: format!("Failed to resume schedule: {e}"),
        })?;

        Ok(ToolOutput {
            success: true,
            content: serde_json::json!({
                "action": "schedule_resumed",
                "id": id,
                "status": "active"
            }),
            warnings: vec![],
            metadata: serde_json::json!({}),
        })
    }

    /// Handle a `ScheduleDelete` tool call.
    pub async fn handle_schedule_delete(
        arguments: &serde_json::Value,
        container: &ServiceContainer,
    ) -> Result<ToolOutput, ToolError> {
        let id = require_arg(arguments, "id")?;

        SchedulerService::delete(
            &container.scheduler_manager,
            id,
            Some(&container.schedule_store),
        )
        .await
        .map_err(|e| ToolError::Other {
            message: format!("Failed to delete schedule: {e}"),
        })?;

        Ok(ToolOutput {
            success: true,
            content: serde_json::json!({
                "action": "schedule_deleted",
                "id": id,
                "status": "deleted"
            }),
            warnings: vec![],
            metadata: serde_json::json!({}),
        })
    }
}

// ---------------------------------------------------------------------------
// Trigger parsing helper
// ---------------------------------------------------------------------------

/// Parse trigger type and value into a `TriggerConfig`.
fn parse_trigger(trigger_type: &str, trigger_value: &str) -> Result<TriggerConfig, ToolError> {
    match trigger_type {
        "cron" => Ok(TriggerConfig::Cron {
            expression: trigger_value.to_string(),
            timezone: "UTC".to_string(),
        }),
        "interval" => {
            let secs: u64 = trigger_value
                .parse()
                .map_err(|_| ToolError::ValidationError {
                    message: format!(
                        "Invalid interval value '{trigger_value}': expected seconds as integer"
                    ),
                })?;
            Ok(TriggerConfig::Interval {
                interval_secs: secs,
            })
        }
        "onetime" => {
            let secs: u64 = trigger_value
                .parse()
                .map_err(|_| ToolError::ValidationError {
                    message: format!(
                        "Invalid onetime delay '{trigger_value}': expected seconds as integer"
                    ),
                })?;
            let at =
                chrono::Utc::now() + chrono::Duration::seconds(i64::try_from(secs).unwrap_or(0));
            Ok(TriggerConfig::OneTime { at })
        }
        other => Err(ToolError::ValidationError {
            message: format!("Unsupported trigger_type '{other}'. Use: cron, interval, onetime"),
        }),
    }
}
