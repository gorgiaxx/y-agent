//! `OrchestratorDispatcher`: the production implementation of
//! [`WorkflowDispatcher`] that runs real workflow DAG execution.
//!
//! Loads a workflow template from storage, parses the definition into a
//! task DAG, creates a [`WorkflowExecutor`] with all four task executors
//! registered, and runs the DAG to completion.

use std::sync::Arc;

use async_trait::async_trait;
use tracing::{debug, warn};

use y_agent::orchestrator::checkpoint::CheckpointStore;
use y_agent::orchestrator::executor::{ExecutionConfig, WorkflowExecutor};
use y_agent::orchestrator::toml_parser::WorkflowDefinition;
use y_scheduler::dispatcher::{DispatchError, DispatchResult, WorkflowDispatcher};

use crate::workflow_executors::{
    FallbackLlmExecutor, LlmCallExecutor, SubAgentExecutor, ToolExecutionExecutor,
};
use crate::workflow_service::WorkflowService;
use crate::ServiceContainer;

/// Production dispatcher: loads workflow from storage, parses DAG, executes
/// with real task executors.
pub struct OrchestratorDispatcher {
    container: Arc<ServiceContainer>,
}

impl OrchestratorDispatcher {
    /// Create a new dispatcher wired to the service container.
    pub fn new(container: Arc<ServiceContainer>) -> Self {
        Self { container }
    }
}

#[async_trait]
impl WorkflowDispatcher for OrchestratorDispatcher {
    async fn dispatch(
        &self,
        workflow_id: &str,
        parameter_values: serde_json::Value,
    ) -> Result<DispatchResult, DispatchError> {
        let start = std::time::Instant::now();

        // 1. Load workflow from storage.
        let row = WorkflowService::get(&self.container.workflow_store, workflow_id)
            .await
            .map_err(|_| DispatchError::WorkflowNotFound {
                id: workflow_id.to_string(),
            })?;

        // 2. Parse definition into DAG.
        let def = match row.format.as_str() {
            "expression_dsl" => WorkflowDefinition::Expression(row.definition.clone()),
            "toml" => WorkflowDefinition::Toml(row.definition.clone()),
            other => {
                return Err(DispatchError::ParseError {
                    message: format!("unsupported format: {other}"),
                });
            }
        };

        let parsed = def.parse().map_err(|e| DispatchError::ParseError {
            message: e.to_string(),
        })?;

        // 3. Create executor and register all task executors.
        let mut executor = WorkflowExecutor::new(ExecutionConfig::default());
        executor.register_executor(Arc::new(LlmCallExecutor::new(Arc::clone(&self.container))));
        executor.register_executor(Arc::new(ToolExecutionExecutor::new(Arc::clone(
            &self.container,
        ))));
        executor.register_executor(Arc::new(SubAgentExecutor::new(Arc::clone(&self.container))));
        executor.register_executor(Arc::new(FallbackLlmExecutor::new(Arc::clone(
            &self.container,
        ))));

        // 4. Build workflow inputs from parameter_values.
        let workflow_inputs: serde_json::Map<String, serde_json::Value> =
            if let Some(obj) = parameter_values.as_object() {
                obj.clone()
            } else {
                serde_json::Map::new()
            };

        let mut checkpoint_store = CheckpointStore::new();

        // 5. Execute the DAG.
        let exec_result = executor
            .execute(
                &parsed.dag,
                &mut checkpoint_store,
                &workflow_inputs,
                &parsed.input_mappings,
                &parsed.output_mappings,
            )
            .await;

        let duration_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(0);

        match exec_result {
            Ok(()) => {
                // 6. Collect outputs into DispatchResult.
                let outputs = executor.all_outputs();
                let output_json: serde_json::Value = outputs
                    .iter()
                    .map(|(id, out)| (id.clone(), out.output.clone()))
                    .collect::<serde_json::Map<String, serde_json::Value>>()
                    .into();

                let summary = format!(
                    "Workflow '{}' completed: {} tasks executed in {}ms",
                    row.name,
                    outputs.len(),
                    duration_ms,
                );

                debug!(
                    workflow_id = %workflow_id,
                    tasks = outputs.len(),
                    duration_ms,
                    "Workflow dispatch completed"
                );

                Ok(DispatchResult {
                    success: true,
                    summary,
                    output: output_json,
                    duration_ms,
                    error: None,
                })
            }
            Err(e) => {
                let error_msg = e.to_string();
                warn!(
                    workflow_id = %workflow_id,
                    error = %error_msg,
                    "Workflow dispatch failed"
                );

                Ok(DispatchResult {
                    success: false,
                    summary: format!("Workflow '{}' failed: {}", row.name, error_msg),
                    output: serde_json::Value::Null,
                    duration_ms,
                    error: Some(error_msg),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Compile-time check: `OrchestratorDispatcher` implements `WorkflowDispatcher`.
    #[allow(dead_code)]
    fn assert_implements_trait(_: Arc<dyn WorkflowDispatcher>) {}
}
