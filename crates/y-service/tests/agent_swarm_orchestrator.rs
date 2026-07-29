#![cfg(feature = "agent_swarm")]

use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use tokio::sync::{Mutex, Notify};
use tokio_util::sync::CancellationToken;
use y_agent::AgentRegistry;
use y_core::agent::{AgentDelegator, ContextStrategyHint, DelegationError, DelegationOutput};
use y_service::agent_swarm_orchestrator::AgentSwarmOrchestrator;

#[derive(Debug, Default)]
struct RecordingDelegator {
    calls: AtomicUsize,
    active: AtomicUsize,
    max_active: AtomicUsize,
}

#[async_trait]
impl AgentDelegator for RecordingDelegator {
    async fn delegate(
        &self,
        _agent_name: &str,
        input: serde_json::Value,
        _context_strategy: ContextStrategyHint,
        _session_id: Option<uuid::Uuid>,
    ) -> Result<DelegationOutput, DelegationError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_active.fetch_max(active, Ordering::SeqCst);

        let task = input["task"].as_str().unwrap_or_default().to_string();
        let delay_ms = if task.contains("slow") { 30 } else { 5 };
        tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
        self.active.fetch_sub(1, Ordering::SeqCst);

        if task.contains("fail") {
            return Err(DelegationError::DelegationFailed {
                message: format!("failed: {task}"),
            });
        }

        Ok(DelegationOutput {
            text: format!("done: {task}"),
            tokens_used: 3,
            input_tokens: 2,
            output_tokens: 1,
            model_used: "mock".to_string(),
            duration_ms: delay_ms,
            workspace_isolation: None,
        })
    }
}

#[tokio::test]
async fn test_agent_swarm_runs_bounded_and_aggregates_partial_results_in_input_order() {
    let delegator = RecordingDelegator::default();
    let registry = Mutex::new(AgentRegistry::new());
    let output = AgentSwarmOrchestrator::handle(
        &serde_json::json!({
            "description": "Review modules",
            "agent_name": "general-purpose",
            "prompt_template": "{{item}}",
            "items": ["slow first", "fail second", "fast third"]
        }),
        &delegator,
        &registry,
        None,
        2,
        None,
    )
    .await
    .expect("partial child failure should not fail the swarm tool");

    assert!(output.success);
    assert_eq!(delegator.max_active.load(Ordering::SeqCst), 2);
    assert_eq!(output.content["summary"]["completed"], 2);
    assert_eq!(output.content["summary"]["failed"], 1);
    assert_eq!(output.content["results"][0]["item"], "slow first");
    assert_eq!(output.content["results"][0]["status"], "completed");
    assert_eq!(output.content["results"][1]["item"], "fail second");
    assert_eq!(output.content["results"][1]["status"], "failed");
    assert_eq!(output.content["results"][2]["item"], "fast third");
}

#[tokio::test]
async fn test_agent_swarm_validates_entire_batch_before_starting_children() {
    let delegator = RecordingDelegator::default();
    let registry = Mutex::new(AgentRegistry::new());
    let result = AgentSwarmOrchestrator::handle(
        &serde_json::json!({
            "description": "Review modules",
            "agent_name": "general-purpose",
            "prompt_template": "Review {{item}}",
            "items": ["same", "same"]
        }),
        &delegator,
        &registry,
        None,
        2,
        None,
    )
    .await;

    assert!(result.is_err());
    assert_eq!(delegator.calls.load(Ordering::SeqCst), 0);
}

#[derive(Debug)]
struct CancelAwareDelegator {
    started: Notify,
    cancel: CancellationToken,
}

#[async_trait]
impl AgentDelegator for CancelAwareDelegator {
    async fn delegate(
        &self,
        _agent_name: &str,
        _input: serde_json::Value,
        _context_strategy: ContextStrategyHint,
        _session_id: Option<uuid::Uuid>,
    ) -> Result<DelegationOutput, DelegationError> {
        self.started.notify_one();
        self.cancel.cancelled().await;
        Err(DelegationError::DelegationFailed {
            message: "cancelled".to_string(),
        })
    }
}

#[tokio::test]
async fn test_agent_swarm_distinguishes_started_and_not_started_cancellation() {
    let cancel = CancellationToken::new();
    let delegator = std::sync::Arc::new(CancelAwareDelegator {
        started: Notify::new(),
        cancel: cancel.clone(),
    });
    let registry = std::sync::Arc::new(Mutex::new(AgentRegistry::new()));
    let run = {
        let delegator = std::sync::Arc::clone(&delegator);
        let registry = std::sync::Arc::clone(&registry);
        let cancel = cancel.clone();
        tokio::spawn(async move {
            AgentSwarmOrchestrator::handle(
                &serde_json::json!({
                    "description": "Review modules",
                    "agent_name": "general-purpose",
                    "prompt_template": "Review {{item}}",
                    "items": ["one", "two", "three"]
                }),
                delegator.as_ref(),
                registry.as_ref(),
                None,
                1,
                Some(&cancel),
            )
            .await
        })
    };

    delegator.started.notified().await;
    cancel.cancel();
    let output = run.await.unwrap().unwrap();

    assert_eq!(output.content["summary"]["aborted"], 3);
    assert_eq!(output.content["results"][0]["state"], "started");
    assert_eq!(output.content["results"][1]["state"], "not_started");
    assert_eq!(output.content["results"][2]["state"], "not_started");
}
