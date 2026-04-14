//! Cross-phase integration tests for multi-agent autonomy.
//!
//! These tests verify end-to-end scenarios spanning multiple phases.

use y_agent::agent::config::MultiAgentConfig;
use y_agent::agent::context::ContextMessage;
use y_agent::agent::definition::{AgentMode, ContextStrategy};
use y_agent::agent::delegation::DelegationProtocol;
use y_agent::agent::dynamic_agent::{
    make_dynamic_agent, CreatorPermissionSnapshot, DynamicAgentStore, DynamicAgentStoreBackend,
};
use y_agent::agent::executor::AgentExecutor;
use y_agent::agent::gap::{AgentGapDetector, AgentGapType, GapResolution};
use y_agent::agent::patterns::micro_pipeline::{MicroPipeline, PipelineStep, WorkingMemory};
use y_agent::agent::patterns::peer_to_peer::{ConcatReducer, PeerToPeerPattern, SharedChannel};
use y_agent::agent::patterns::sequential::SequentialPattern;
use y_agent::agent::pool::AgentPool;
use y_agent::agent::registry::AgentRegistry;
use y_agent::agent::task_tool::{execute_task_tool, TaskToolParams};

/// T-MA-INT-01: Full delegation lifecycle: Registry → Pool → Executor → Result.
#[test]
fn test_full_delegation_lifecycle() {
    let registry = AgentRegistry::new();
    let mut pool = AgentPool::new(MultiAgentConfig::default());

    // Prepare
    let prepared = AgentExecutor::prepare(
        &registry,
        &mut pool,
        "tool-engineer",
        "Build a new linting tool",
        &[ContextMessage::user("We need better linting")],
        None,
        ContextStrategy::Summary,
        Some("deleg-001".to_string()),
    )
    .unwrap();

    assert_eq!(prepared.definition.id, "tool-engineer");
    assert!(!prepared.context_messages.is_empty());

    // Execute
    let output = AgentExecutor::execute_simulated(&mut pool, &prepared).unwrap();
    assert!(output.success);
    assert_eq!(output.agent_id, "tool-engineer");
    assert!(!output.output.is_empty());
}

/// T-MA-INT-02: Sequential pattern with 2 agents.
#[test]
fn test_sequential_pattern_two_agents() {
    let protocol = DelegationProtocol::new(MultiAgentConfig::default());
    let tasks = vec![
        protocol.create_task("researcher", "Find best practices"),
        protocol.create_task("writer", "Document the findings"),
    ];

    let results = SequentialPattern::execute(&protocol, tasks).unwrap();
    assert_eq!(results.len(), 2);
    assert!(results[0].success);
    assert!(results[1].success);
    assert_eq!(results[0].agent_id, "researcher");
    assert_eq!(results[1].agent_id, "writer");
}

/// T-MA-INT-04: Gap detection → resolution attempt.
#[test]
fn test_gap_detection_and_resolution() {
    let registry = AgentRegistry::new();

    // Detect gap for non-existent agent
    let gap = AgentGapDetector::detect(&registry, "test-runner-agent", &[], None);
    assert!(gap.is_some());
    assert!(matches!(
        gap.as_ref().unwrap(),
        AgentGapType::AgentNotFound { .. }
    ));

    // Attempt resolution → HITL required (no orchestrator integration)
    let resolution = AgentGapDetector::resolve(&registry, gap.as_ref().unwrap());
    assert!(matches!(resolution, GapResolution::HitlRequired { .. }));
}

/// T-MA-INT-07: Permission inheritance end-to-end.
#[test]
fn test_permission_inheritance_e2e() {
    let store = DynamicAgentStore::new();
    let creator = CreatorPermissionSnapshot {
        tools_allowed: vec!["FileRead".to_string(), "SearchCode".to_string()],
        max_iterations: 50,
        max_tool_calls: 100,
        max_tokens: 8192,
        delegation_depth: 3,
    };

    // Create parent agent (depth 3 → effective depth 2)
    let parent = make_dynamic_agent(
        "parent-agent",
        "A parent agent",
        "root",
        &["FileRead".to_string()],
        &creator,
    );
    let parent = store.create(parent).unwrap();
    assert_eq!(parent.effective_permissions.delegation_depth, 2);
    assert!(parent
        .effective_permissions
        .tools_allowed
        .contains(&"FileRead".to_string()));

    // Create child using parent's permissions (depth 2 → effective depth 1)
    let child_creator = CreatorPermissionSnapshot {
        tools_allowed: parent.effective_permissions.tools_allowed.clone(),
        max_iterations: parent.effective_permissions.max_iterations,
        max_tool_calls: parent.effective_permissions.max_tool_calls,
        max_tokens: parent.effective_permissions.max_tokens,
        delegation_depth: parent.effective_permissions.delegation_depth,
    };

    let child = make_dynamic_agent(
        "child-agent",
        "A child agent",
        &parent.id,
        &["FileRead".to_string()],
        &child_creator,
    );
    let child = store.create(child).unwrap();
    assert_eq!(child.effective_permissions.delegation_depth, 1);

    // Grandchild (depth 1 → effective depth 0) → should be rejected by validation
    let grandchild_creator = CreatorPermissionSnapshot {
        tools_allowed: child.effective_permissions.tools_allowed.clone(),
        max_iterations: child.effective_permissions.max_iterations,
        max_tool_calls: child.effective_permissions.max_tool_calls,
        max_tokens: child.effective_permissions.max_tokens,
        delegation_depth: child.effective_permissions.delegation_depth,
    };

    let grandchild = make_dynamic_agent(
        "grandchild-agent",
        "A grandchild agent",
        &child.id,
        &["FileRead".to_string()],
        &grandchild_creator,
    );
    // Grandchild has delegation_depth 0 → validation rejects dynamic agents at depth 0
    assert!(store.create(grandchild).is_err());
    assert_eq!(store.count(), 2);
}

/// T-MA-INT-08: Task tool in-conversation delegation with depth limit.
#[test]
fn test_task_tool_with_depth_limit() {
    let registry = AgentRegistry::new();
    let mut pool = AgentPool::new(MultiAgentConfig::default());

    // Depth 0, max 3 → should succeed
    let result = execute_task_tool(
        &registry,
        &mut pool,
        &TaskToolParams {
            agent_name: "agent-architect".to_string(),
            prompt: "Design a test agent".to_string(),
            mode: Some(AgentMode::Plan),
            context_strategy: None,
        },
        &[],
        0,
        3,
    )
    .unwrap();
    assert!(result.success);

    // Depth 3, max 3 → should be rejected
    let err = execute_task_tool(
        &registry,
        &mut pool,
        &TaskToolParams {
            agent_name: "agent-architect".to_string(),
            prompt: "nested too deep".to_string(),
            mode: None,
            context_strategy: None,
        },
        &[],
        3,
        3,
    );
    assert!(err.is_err());
}

/// T-MA-INT: Micro pipeline end-to-end with WM slot chaining.
#[test]
fn test_micro_pipeline_e2e() {
    let protocol = DelegationProtocol::new(MultiAgentConfig::default());

    let mut wm = WorkingMemory::new();
    wm.set("code", "fn main() { todo!() }");

    let steps = vec![
        PipelineStep::new(
            "analyzer",
            "Analyze: {code}",
            vec!["code".to_string()],
            "analysis",
        ),
        PipelineStep::new(
            "reviewer",
            "Review: {analysis}",
            vec!["analysis".to_string()],
            "review",
        ),
        PipelineStep::new(
            "reporter",
            "Report: {review}",
            vec!["review".to_string()],
            "report",
        ),
    ];

    let result = MicroPipeline::execute(&protocol, &steps, wm).unwrap();
    assert_eq!(result.step_results.len(), 3);
    assert!(result.working_memory.has("code"));
    assert!(result.working_memory.has("analysis"));
    assert!(result.working_memory.has("review"));
    assert!(result.working_memory.has("report"));
}

/// T-MA-INT: P2P pattern end-to-end.
#[test]
fn test_peer_to_peer_e2e() {
    let protocol = DelegationProtocol::new(MultiAgentConfig::default());
    let channel = SharedChannel::new("research");

    let tasks = vec![
        protocol.create_task("researcher-1", "Research topic A"),
        protocol.create_task("researcher-2", "Research topic B"),
    ];

    let output = PeerToPeerPattern::execute(&protocol, &tasks, &channel, &ConcatReducer).unwrap();
    assert!(!output.is_empty());
    assert_eq!(channel.len(), 2);
}
