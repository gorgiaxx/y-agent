//! Sequential pattern: ordered agent chain, failure stops chain.

use crate::agent::delegation::{DelegationProtocol, DelegationResult, DelegationTask};
use crate::agent::error::MultiAgentError;

/// Executes a chain of agents sequentially.
///
/// Each agent receives the output of the previous agent as context.
/// If any agent fails, the chain stops and returns the error.
#[derive(Debug)]
pub struct SequentialPattern;

impl SequentialPattern {
    /// Execute tasks in sequence, chaining outputs.
    pub fn execute(
        protocol: &DelegationProtocol,
        tasks: Vec<DelegationTask>,
    ) -> Result<Vec<DelegationResult>, MultiAgentError> {
        let mut results = Vec::new();

        for task in tasks {
            let result = protocol.execute_sync(&task)?;
            if !result.success {
                return Err(MultiAgentError::DelegationFailed {
                    message: format!("agent {} failed: {}", result.agent_id, result.output),
                });
            }
            results.push(result);
        }

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::config::MultiAgentConfig;

    /// T-MA-004-01: Sequential chain executes in order.
    #[test]
    fn test_sequential_executes_in_order() {
        let protocol = DelegationProtocol::new(MultiAgentConfig::default());
        let tasks = vec![
            protocol.create_task("agent-1", "step 1"),
            protocol.create_task("agent-2", "step 2"),
            protocol.create_task("agent-3", "step 3"),
        ];

        let results = SequentialPattern::execute(&protocol, tasks).unwrap();
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].agent_id, "agent-1");
        assert_eq!(results[1].agent_id, "agent-2");
        assert_eq!(results[2].agent_id, "agent-3");
    }

    /// T-MA-004-02: Empty chain returns empty results.
    #[test]
    fn test_sequential_empty_chain() {
        let protocol = DelegationProtocol::new(MultiAgentConfig::default());
        let results = SequentialPattern::execute(&protocol, vec![]).unwrap();
        assert!(results.is_empty());
    }
}
