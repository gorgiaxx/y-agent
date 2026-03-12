//! Delegation protocol: task handoff between agents.
//!
//! Design reference: multi-agent-design.md §Delegation Protocol
//!
//! The delegation protocol manages task handoff between agents including:
//! - Context sharing strategy selection
//! - Mode override per delegation
//! - Depth tracking to prevent circular delegation
//! - Timeout enforcement

use crate::agent::config::MultiAgentConfig;
use crate::agent::definition::{AgentMode, ContextStrategy};
use crate::agent::error::MultiAgentError;

/// A task to be delegated to another agent.
#[derive(Debug, Clone)]
pub struct DelegationTask {
    /// Target agent ID.
    pub agent_id: String,
    /// Task description or prompt.
    pub task: String,
    /// Maximum execution time (ms).
    pub timeout_ms: Option<u64>,
    /// Override the target agent's default mode for this delegation.
    pub mode_override: Option<AgentMode>,
    /// Context sharing strategy for this delegation.
    pub context_strategy: ContextStrategy,
    /// Current delegation depth (0 = top-level, increments for child delegations).
    pub depth: usize,
    /// ID of the agent that initiated this delegation.
    pub parent_agent_id: Option<String>,
}

/// Result from a delegated task.
#[derive(Debug, Clone)]
pub struct DelegationResult {
    /// The agent that executed the task.
    pub agent_id: String,
    /// Task output/result.
    pub output: String,
    /// Whether the task completed successfully.
    pub success: bool,
    /// Number of agent loop iterations used.
    pub iterations: usize,
    /// Approximate token count consumed.
    pub tokens_used: usize,
}

/// Manages task delegation between agents.
///
/// Design reference: multi-agent-design.md §Delegation Protocol
#[derive(Debug)]
pub struct DelegationProtocol {
    config: MultiAgentConfig,
}

impl DelegationProtocol {
    pub fn new(config: MultiAgentConfig) -> Self {
        Self { config }
    }

    /// Create a delegation task with the configured timeout.
    ///
    /// The task starts at `depth = 0` (top-level) with `ContextStrategy::None`.
    /// Callers should set `depth`, `mode_override`, and `context_strategy`
    /// as needed after construction.
    pub fn create_task(&self, agent_id: &str, task: &str) -> DelegationTask {
        DelegationTask {
            agent_id: agent_id.to_string(),
            task: task.to_string(),
            timeout_ms: Some(self.config.delegation_timeout_ms),
            mode_override: None,
            context_strategy: ContextStrategy::None,
            depth: 0,
            parent_agent_id: None,
        }
    }

    /// Create a child delegation task (increments depth, sets parent).
    pub fn create_child_task(
        &self,
        parent_id: &str,
        target_id: &str,
        task: &str,
        parent_depth: usize,
    ) -> DelegationTask {
        DelegationTask {
            agent_id: target_id.to_string(),
            task: task.to_string(),
            timeout_ms: Some(self.config.delegation_timeout_ms),
            mode_override: None,
            context_strategy: ContextStrategy::None,
            depth: parent_depth + 1,
            parent_agent_id: Some(parent_id.to_string()),
        }
    }

    /// Maximum delegation depth from configuration.
    pub fn max_depth(&self) -> usize {
        self.config.max_delegation_depth
    }

    /// Validate a delegation task.
    ///
    /// Validates required fields and enforces depth limit.
    pub fn validate_task(&self, task: &DelegationTask) -> Result<(), MultiAgentError> {
        if task.agent_id.is_empty() {
            return Err(MultiAgentError::DelegationFailed {
                message: "target agent_id is empty".to_string(),
            });
        }
        if task.task.is_empty() {
            return Err(MultiAgentError::DelegationFailed {
                message: "task description is empty".to_string(),
            });
        }
        if task.depth > self.config.max_delegation_depth {
            return Err(MultiAgentError::DelegationDepthExceeded {
                depth: task.depth,
                max: self.config.max_delegation_depth,
            });
        }
        Ok(())
    }

    /// Simulate executing a delegation (for testing).
    /// In production, this would create a child session and run the agent.
    pub fn execute_sync(&self, task: &DelegationTask) -> Result<DelegationResult, MultiAgentError> {
        self.validate_task(task)?;

        Ok(DelegationResult {
            agent_id: task.agent_id.clone(),
            output: format!("simulated result for task: {}", task.task),
            success: true,
            iterations: 1,
            tokens_used: 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// T-MA-003-01: Delegation creates a child task.
    #[test]
    fn test_delegation_create_task() {
        let protocol = DelegationProtocol::new(MultiAgentConfig::default());
        let task = protocol.create_task("agent-1", "review this code");

        assert_eq!(task.agent_id, "agent-1");
        assert_eq!(task.task, "review this code");
        assert!(task.timeout_ms.is_some());
        assert_eq!(task.depth, 0);
        assert!(task.parent_agent_id.is_none());
        assert_eq!(task.context_strategy, ContextStrategy::None);
        assert!(task.mode_override.is_none());
    }

    /// T-MA-003-02: Delegation validates required fields.
    #[test]
    fn test_delegation_validates() {
        let protocol = DelegationProtocol::new(MultiAgentConfig::default());
        let bad_task = DelegationTask {
            agent_id: String::new(),
            task: "test".to_string(),
            timeout_ms: None,
            mode_override: None,
            context_strategy: ContextStrategy::None,
            depth: 0,
            parent_agent_id: None,
        };
        assert!(protocol.validate_task(&bad_task).is_err());
    }

    /// T-MA-003-03: Successful delegation returns result.
    #[test]
    fn test_delegation_execute() {
        let protocol = DelegationProtocol::new(MultiAgentConfig::default());
        let task = protocol.create_task("agent-1", "analyze code");
        let result = protocol.execute_sync(&task).unwrap();

        assert_eq!(result.agent_id, "agent-1");
        assert!(result.success);
        assert!(!result.output.is_empty());
        assert_eq!(result.iterations, 1);
    }

    /// T-MA-003-04: Delegation timeout configuration.
    #[test]
    fn test_delegation_timeout_config() {
        let config = MultiAgentConfig {
            delegation_timeout_ms: 5000,
            ..Default::default()
        };
        let protocol = DelegationProtocol::new(config);
        let task = protocol.create_task("agent-1", "quick task");
        assert_eq!(task.timeout_ms, Some(5000));
    }

    /// T-MA-003-05: Delegation depth exceeded is rejected.
    #[test]
    fn test_delegation_depth_exceeded() {
        let config = MultiAgentConfig {
            max_delegation_depth: 2,
            ..Default::default()
        };
        let protocol = DelegationProtocol::new(config);
        let task = DelegationTask {
            agent_id: "deep-agent".to_string(),
            task: "nested task".to_string(),
            timeout_ms: Some(1000),
            mode_override: None,
            context_strategy: ContextStrategy::None,
            depth: 3,
            parent_agent_id: Some("parent".to_string()),
        };
        let err = protocol.validate_task(&task).unwrap_err();
        assert!(matches!(
            err,
            MultiAgentError::DelegationDepthExceeded { .. }
        ));
    }

    /// T-MA-003-06: Child task increments depth correctly.
    #[test]
    fn test_create_child_task() {
        let protocol = DelegationProtocol::new(MultiAgentConfig::default());
        let child = protocol.create_child_task("parent-agent", "child-agent", "sub-task", 1);

        assert_eq!(child.agent_id, "child-agent");
        assert_eq!(child.depth, 2);
        assert_eq!(child.parent_agent_id.as_deref(), Some("parent-agent"));
    }

    /// T-MA-003-07: Mode override can be set on delegation.
    #[test]
    fn test_delegation_mode_override() {
        let protocol = DelegationProtocol::new(MultiAgentConfig::default());
        let mut task = protocol.create_task("agent-1", "plan something");
        task.mode_override = Some(AgentMode::Plan);
        task.context_strategy = ContextStrategy::Summary;

        assert_eq!(task.mode_override, Some(AgentMode::Plan));
        assert_eq!(task.context_strategy, ContextStrategy::Summary);

        // Should still validate and execute fine.
        let result = protocol.execute_sync(&task).unwrap();
        assert!(result.success);
    }
}
