//! Handoff-artifact and rigor behavior of the workflow executor.
//!
//! These exercise the public orchestrator API end to end: a registered executor
//! returns real outputs, and the executor decides whether the node completed.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::Utc;

use y_agent::orchestrator::artifact::{ConfidenceLevel, NodeKind, Rigor, UPSTREAM_ARTIFACTS_INPUT};
use y_agent::orchestrator::checkpoint::{CheckpointStore, TaskOutput};
use y_agent::orchestrator::dag::{TaskDag, TaskId, TaskNode, TaskType};
use y_agent::orchestrator::executor::{
    ExecutionConfig, WorkflowExecuteError, WorkflowExecutor, WorkflowState,
};
use y_agent::orchestrator::task_executor::{TaskExecuteError, TaskExecutor};
use y_agent::orchestrator::WorkflowContext;

/// Inputs observed by [`FixedOutputExecutor`], keyed by task id.
type SeenInputs = Arc<Mutex<HashMap<TaskId, HashMap<String, serde_json::Value>>>>;

/// A stub executor that returns a fixed output for every task and records the
/// inputs it was handed.
struct FixedOutputExecutor {
    output: serde_json::Value,
    seen_inputs: SeenInputs,
}

impl FixedOutputExecutor {
    fn new(output: serde_json::Value) -> Self {
        Self {
            output,
            seen_inputs: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

#[async_trait]
impl TaskExecutor for FixedOutputExecutor {
    async fn execute(
        &self,
        task: &TaskNode,
        inputs: HashMap<String, serde_json::Value>,
        _ctx: &WorkflowContext,
    ) -> Result<TaskOutput, TaskExecuteError> {
        self.seen_inputs
            .lock()
            .expect("input recorder poisoned")
            .insert(task.id.clone(), inputs);
        Ok(TaskOutput {
            task_id: task.id.clone(),
            output: self.output.clone(),
            completed_at: Utc::now(),
        })
    }

    fn supports(&self, _task_type: &TaskType) -> bool {
        true
    }
}

fn task(id: &str, deps: &[&str], kind: NodeKind) -> TaskNode {
    TaskNode {
        id: id.into(),
        name: id.into(),
        dependencies: deps.iter().map(|dep| (*dep).to_string()).collect(),
        node_kind: kind,
        ..TaskNode::default()
    }
}

fn complete_artifact_output() -> serde_json::Value {
    serde_json::json!({
        "artifact": {
            "summary": "inspected the retry path",
            "findings": ["429 bypasses retry"],
            "evidence": ["crates/y-provider/src/pool.rs:444"],
            "what_i_did_not_check": ["the streaming path"],
            "confidence": "high"
        }
    })
}

fn deep() -> ExecutionConfig {
    ExecutionConfig {
        rigor: Rigor::Deep,
        ..ExecutionConfig::default()
    }
}

async fn run(executor: &mut WorkflowExecutor, dag: &TaskDag) -> Result<(), WorkflowExecuteError> {
    let mut checkpoints = CheckpointStore::new();
    let workflow_inputs = serde_json::Map::new();
    executor
        .execute(
            dag,
            &mut checkpoints,
            &workflow_inputs,
            &HashMap::new(),
            &HashMap::new(),
        )
        .await
}

/// Light rigor must not change existing behavior: artifact-free outputs pass.
#[tokio::test]
async fn light_rigor_accepts_artifact_free_nodes() {
    let mut dag = TaskDag::new();
    dag.add_task(task("a", &[], NodeKind::Unspecified))
        .expect("add task");

    let mut executor = WorkflowExecutor::new(ExecutionConfig::default());
    executor.register_executor(Arc::new(FixedOutputExecutor::new(
        serde_json::json!({"status": "ok"}),
    )));

    run(&mut executor, &dag).await.expect("light rigor passes");
    assert_eq!(executor.state, WorkflowState::Completed);
    assert!(executor.get_artifact("a").is_none());
}

/// Deep rigor rejects a node that completes without a handoff artifact, routing
/// the rejection through the ordinary failure strategy.
#[tokio::test]
async fn deep_rigor_fails_a_node_without_an_artifact() {
    let mut dag = TaskDag::new();
    dag.add_task(task("a", &[], NodeKind::Explore))
        .expect("add task");

    let mut executor = WorkflowExecutor::new(deep());
    executor.register_executor(Arc::new(FixedOutputExecutor::new(
        serde_json::json!({"status": "ok"}),
    )));

    let error = run(&mut executor, &dag)
        .await
        .expect_err("deep rigor fails");
    assert!(
        error.to_string().contains("handoff artifact rejected"),
        "{error}"
    );
    assert_eq!(executor.state, WorkflowState::Failed);
}

/// Deep rigor rejects a node that never declared its semantic role.
#[tokio::test]
async fn deep_rigor_fails_an_undeclared_node_kind() {
    let mut dag = TaskDag::new();
    dag.add_task(task("a", &[], NodeKind::Unspecified))
        .expect("add task");

    let mut executor = WorkflowExecutor::new(deep());
    executor.register_executor(Arc::new(FixedOutputExecutor::new(
        complete_artifact_output(),
    )));

    let error = run(&mut executor, &dag)
        .await
        .expect_err("deep rigor fails");
    assert!(error.to_string().contains("unspecified"), "{error}");
}

/// Deep rigor accepts a complete artifact and retains it for downstream use.
#[tokio::test]
async fn deep_rigor_retains_a_complete_artifact() {
    let mut dag = TaskDag::new();
    dag.add_task(task("a", &[], NodeKind::Verify))
        .expect("add task");

    let mut executor = WorkflowExecutor::new(deep());
    executor.register_executor(Arc::new(FixedOutputExecutor::new(
        complete_artifact_output(),
    )));

    run(&mut executor, &dag)
        .await
        .expect("complete artifact passes");

    let artifact = executor.get_artifact("a").expect("artifact retained");
    assert_eq!(artifact.what_i_did_not_check, vec!["the streaming path"]);
    assert_eq!(artifact.confidence_level(), ConfidenceLevel::High);
}

/// A dependent node receives its predecessors' artifacts without declaring any
/// input mapping; a root node receives none.
#[tokio::test]
async fn artifacts_flow_forward_along_edges() {
    let mut dag = TaskDag::new();
    dag.add_task(task("a", &[], NodeKind::Explore))
        .expect("add root");
    dag.add_task(task("b", &["a"], NodeKind::Implement))
        .expect("add dependent");

    let stub = Arc::new(FixedOutputExecutor::new(complete_artifact_output()));
    let seen = stub.seen_inputs.clone();
    let mut executor = WorkflowExecutor::new(ExecutionConfig::default());
    executor.register_executor(stub);

    run(&mut executor, &dag).await.expect("execution succeeds");

    let recorded = seen.lock().expect("input recorder poisoned");
    assert!(
        !recorded
            .get("a")
            .expect("root ran")
            .contains_key(UPSTREAM_ARTIFACTS_INPUT),
        "a root node has no upstream artifacts"
    );

    let upstream = recorded
        .get("b")
        .expect("dependent ran")
        .get(UPSTREAM_ARTIFACTS_INPUT)
        .expect("dependent received upstream artifacts");
    assert_eq!(
        upstream
            .get("a")
            .and_then(|artifact| artifact.get("summary"))
            .and_then(serde_json::Value::as_str),
        Some("inspected the retry path")
    );
}

/// The synchronous path fabricates placeholder outputs, so it must refuse deep
/// rigor rather than silently downgrading the caller's request.
#[test]
fn sync_execution_refuses_deep_rigor() {
    let mut dag = TaskDag::new();
    dag.add_task(task("a", &[], NodeKind::Explore))
        .expect("add task");

    let mut executor = WorkflowExecutor::new(deep());
    let mut checkpoints = CheckpointStore::new();

    let error = executor
        .execute_sync(&dag, &mut checkpoints)
        .expect_err("sync path cannot satisfy deep rigor");
    assert!(error.to_string().contains("deep rigor"), "{error}");
}
