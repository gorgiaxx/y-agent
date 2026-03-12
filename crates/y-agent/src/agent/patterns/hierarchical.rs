//! Hierarchical pattern: supervisor delegates to workers.
//!
//! A supervisor agent coordinates multiple worker agents,
//! collecting and aggregating their results.

use crate::agent::delegation::{DelegationProtocol, DelegationResult, DelegationTask};
use crate::agent::error::MultiAgentError;

/// Supervisor delegates tasks to multiple workers and aggregates results.
#[derive(Debug)]
pub struct HierarchicalPattern;

impl HierarchicalPattern {
    /// Execute all worker tasks (simulated parallel, actually sequential in this impl).
    ///
    /// Returns all results, including failures.
    pub fn execute(
        protocol: &DelegationProtocol,
        tasks: Vec<DelegationTask>,
    ) -> Result<Vec<DelegationResult>, MultiAgentError> {
        let mut results = Vec::new();

        for task in tasks {
            match protocol.execute_sync(&task) {
                Ok(result) => results.push(result),
                Err(e) => {
                    results.push(DelegationResult {
                        agent_id: task.agent_id.clone(),
                        output: format!("worker failed: {e}"),
                        success: false,
                        iterations: 0,
                        tokens_used: 0,
                    });
                }
            }
        }

        Ok(results)
    }

    /// Check whether all workers succeeded.
    pub fn all_succeeded(results: &[DelegationResult]) -> bool {
        results.iter().all(|r| r.success)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::config::MultiAgentConfig;
    use crate::agent::definition::ContextStrategy;

    /// T-MA-004-03: Hierarchical pattern delegates to multiple workers.
    #[test]
    fn test_hierarchical_delegates() {
        let protocol = DelegationProtocol::new(MultiAgentConfig::default());
        let tasks = vec![
            protocol.create_task("worker-1", "subtask A"),
            protocol.create_task("worker-2", "subtask B"),
        ];

        let results = HierarchicalPattern::execute(&protocol, tasks).unwrap();
        assert_eq!(results.len(), 2);
        assert!(HierarchicalPattern::all_succeeded(&results));
    }

    /// T-MA-004-04: Hierarchical pattern collects partial failures.
    #[test]
    fn test_hierarchical_partial_failure() {
        let protocol = DelegationProtocol::new(MultiAgentConfig::default());
        // Valid task
        let tasks = vec![
            protocol.create_task("worker-1", "subtask A"),
            DelegationTask {
                agent_id: String::new(), // Invalid — will fail
                task: "subtask B".to_string(),
                timeout_ms: None,
                mode_override: None,
                context_strategy: ContextStrategy::None,
                depth: 0,
                parent_agent_id: None,
            },
        ];

        let results = HierarchicalPattern::execute(&protocol, tasks).unwrap();
        assert_eq!(results.len(), 2);
        assert!(results[0].success);
        assert!(!results[1].success);
        assert!(!HierarchicalPattern::all_succeeded(&results));
    }
}
