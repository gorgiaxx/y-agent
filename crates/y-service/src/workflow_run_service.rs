//! Service-owned execution of reusable workflow templates.

use crate::scheduler_service::{ExecutionSummary, SchedulerService, SchedulerServiceError};
use crate::workflow_service::{WorkflowService, WorkflowServiceError};
use crate::ServiceContainer;

/// Resolves and executes an existing workflow by ID or unique name.
pub struct WorkflowRunService;

impl WorkflowRunService {
    pub async fn run(
        container: &ServiceContainer,
        identifier: &str,
        parameters: serde_json::Value,
    ) -> Result<ExecutionSummary, WorkflowRunError> {
        if identifier.trim().is_empty() {
            return Err(WorkflowRunError::InvalidIdentifier);
        }
        if !parameters.is_object() {
            return Err(WorkflowRunError::InvalidParameters);
        }

        let workflow = WorkflowService::get(&container.workflow_store, identifier).await?;
        SchedulerService::execute_workflow_with_parameters(
            &container.scheduler_manager,
            &workflow.id,
            &workflow.name,
            parameters,
        )
        .await
        .map_err(WorkflowRunError::Scheduler)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum WorkflowRunError {
    #[error("workflow identifier must not be blank")]
    InvalidIdentifier,
    #[error("workflow parameters must be a JSON object")]
    InvalidParameters,
    #[error(transparent)]
    Workflow(#[from] WorkflowServiceError),
    #[error("workflow execution failed: {0}")]
    Scheduler(SchedulerServiceError),
}
