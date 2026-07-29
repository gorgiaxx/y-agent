//! Service-owned bounded fan-out for the `AgentSwarm` tool.

use futures::future::join_all;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use y_agent::AgentRegistry;
use y_core::agent::AgentDelegator;
use y_core::tool::{ToolError, ToolOutput};
use y_tools::builtin::agent_swarm::{
    prepare_agent_swarm, PreparedAgentSwarm, PreparedAgentSwarmTask,
};

use crate::task_delegation_orchestrator::TaskDelegationOrchestrator;

pub struct AgentSwarmOrchestrator;

impl AgentSwarmOrchestrator {
    pub async fn handle(
        arguments: &serde_json::Value,
        delegator: &dyn AgentDelegator,
        agent_registry: &Mutex<AgentRegistry>,
        session_id: Option<uuid::Uuid>,
        max_concurrency: usize,
        cancel: Option<&CancellationToken>,
    ) -> Result<ToolOutput, ToolError> {
        let prepared = prepare_agent_swarm(arguments)?;
        if max_concurrency == 0 {
            return Err(ToolError::RuntimeError {
                name: "AgentSwarm".to_string(),
                message: "agent swarm concurrency must be greater than zero".to_string(),
            });
        }

        let mut results = Vec::with_capacity(prepared.tasks.len());
        for tasks in prepared.tasks.chunks(max_concurrency) {
            results.extend(
                join_all(tasks.iter().map(|task| {
                    run_item(
                        &prepared,
                        task,
                        delegator,
                        agent_registry,
                        session_id,
                        cancel,
                    )
                }))
                .await,
            );
        }

        let completed = results
            .iter()
            .filter(|result| result.status == SwarmItemStatus::Completed)
            .count();
        let failed = results
            .iter()
            .filter(|result| result.status == SwarmItemStatus::Failed)
            .count();
        let aborted = results
            .iter()
            .filter(|result| result.status == SwarmItemStatus::Aborted)
            .count();
        let summary = serde_json::json!({
            "completed": completed,
            "failed": failed,
            "aborted": aborted,
        });

        let content_results = results
            .iter()
            .map(SwarmItemResult::content_value)
            .collect::<Vec<_>>();
        let metadata_results = results
            .iter()
            .map(SwarmItemResult::metadata_value)
            .collect::<Vec<_>>();
        let warnings = results
            .iter()
            .flat_map(|result| {
                result
                    .warnings
                    .iter()
                    .map(move |warning| format!("item {}: {warning}", result.index))
            })
            .collect();

        Ok(ToolOutput {
            success: true,
            content: serde_json::json!({
                "description": prepared.description,
                "summary": summary,
                "results": content_results,
            }),
            warnings,
            metadata: serde_json::json!({
                "action": "delegate_swarm",
                "summary": summary,
                "items": metadata_results,
            }),
        })
    }
}

async fn run_item(
    swarm: &PreparedAgentSwarm,
    task: &PreparedAgentSwarmTask,
    delegator: &dyn AgentDelegator,
    agent_registry: &Mutex<AgentRegistry>,
    session_id: Option<uuid::Uuid>,
    cancel: Option<&CancellationToken>,
) -> SwarmItemResult {
    if cancel.is_some_and(CancellationToken::is_cancelled) {
        return SwarmItemResult::aborted(task, "not_started");
    }

    let arguments = task_arguments(swarm, task);
    match TaskDelegationOrchestrator::handle(&arguments, delegator, agent_registry, session_id)
        .await
    {
        Ok(output) => SwarmItemResult {
            index: task.index,
            item: task.item.clone(),
            status: SwarmItemStatus::Completed,
            state: None,
            result: Some(output.content),
            error: None,
            warnings: output.warnings,
            metadata: output.metadata,
        },
        Err(_) if cancel.is_some_and(CancellationToken::is_cancelled) => {
            SwarmItemResult::aborted(task, "started")
        }
        Err(error) => SwarmItemResult {
            index: task.index,
            item: task.item.clone(),
            status: SwarmItemStatus::Failed,
            state: Some("started"),
            result: None,
            error: Some(error.to_string()),
            warnings: Vec::new(),
            metadata: serde_json::Value::Null,
        },
    }
}

fn task_arguments(swarm: &PreparedAgentSwarm, task: &PreparedAgentSwarmTask) -> serde_json::Value {
    let mut object = serde_json::Map::from_iter([
        (
            "agent_name".to_string(),
            serde_json::Value::String(swarm.agent_name.clone()),
        ),
        (
            "prompt".to_string(),
            serde_json::Value::String(task.prompt.clone()),
        ),
    ]);
    insert_optional_string(&mut object, "mode", swarm.mode.as_deref());
    insert_optional_string(
        &mut object,
        "context_strategy",
        swarm.context_strategy.as_deref(),
    );
    insert_optional_string(
        &mut object,
        "workspace_isolation",
        swarm.workspace_isolation.as_deref(),
    );
    if let Some(schema) = swarm.result_schema.as_ref() {
        object.insert("result_schema".to_string(), schema.clone());
    }
    serde_json::Value::Object(object)
}

fn insert_optional_string(
    object: &mut serde_json::Map<String, serde_json::Value>,
    key: &str,
    value: Option<&str>,
) {
    if let Some(value) = value {
        object.insert(
            key.to_string(),
            serde_json::Value::String(value.to_string()),
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SwarmItemStatus {
    Completed,
    Failed,
    Aborted,
}

impl SwarmItemStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Aborted => "aborted",
        }
    }
}

struct SwarmItemResult {
    index: usize,
    item: String,
    status: SwarmItemStatus,
    state: Option<&'static str>,
    result: Option<serde_json::Value>,
    error: Option<String>,
    warnings: Vec<String>,
    metadata: serde_json::Value,
}

impl SwarmItemResult {
    fn aborted(task: &PreparedAgentSwarmTask, state: &'static str) -> Self {
        Self {
            index: task.index,
            item: task.item.clone(),
            status: SwarmItemStatus::Aborted,
            state: Some(state),
            result: None,
            error: Some("agent swarm cancelled by user".to_string()),
            warnings: Vec::new(),
            metadata: serde_json::Value::Null,
        }
    }

    fn content_value(&self) -> serde_json::Value {
        let mut object = serde_json::Map::from_iter([
            ("index".to_string(), serde_json::json!(self.index)),
            ("item".to_string(), serde_json::json!(self.item)),
            (
                "status".to_string(),
                serde_json::json!(self.status.as_str()),
            ),
        ]);
        if let Some(state) = self.state {
            object.insert("state".to_string(), serde_json::json!(state));
        }
        if let Some(result) = self.result.as_ref() {
            object.insert("result".to_string(), result.clone());
        }
        if let Some(error) = self.error.as_ref() {
            object.insert("error".to_string(), serde_json::json!(error));
        }
        serde_json::Value::Object(object)
    }

    fn metadata_value(&self) -> serde_json::Value {
        serde_json::json!({
            "index": self.index,
            "item": self.item,
            "status": self.status.as_str(),
            "delegation": self.metadata,
        })
    }
}
