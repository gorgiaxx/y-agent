//! Built-in `task` tool: in-conversation agent delegation.
//!
//! Design reference: multi-agent-design.md §Task Tool
//!
//! The `task` tool allows an agent to delegate work to another agent
//! within a conversation. It wraps the delegation protocol and executor.

use serde::{Deserialize, Serialize};

use crate::agent::context::ContextMessage;
use crate::agent::definition::{AgentMode, ContextStrategy};
use crate::agent::error::MultiAgentError;
use crate::agent::executor::{AgentExecutor, TaskOutput};
use crate::agent::pool::AgentPool;
use crate::agent::registry::AgentRegistry;

// ---------------------------------------------------------------------------
// Task tool parameters
// ---------------------------------------------------------------------------

/// Parameters for the built-in `task` tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskToolParams {
    /// Target agent name/ID.
    pub agent_name: String,
    /// Task prompt to delegate.
    pub prompt: String,
    /// Mode override (optional; uses agent's default if not set).
    #[serde(default)]
    pub mode: Option<AgentMode>,
    /// Context strategy override (optional; uses agent's default if not set).
    #[serde(default)]
    pub context_strategy: Option<ContextStrategy>,
}

/// Result returned from the `task` tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskToolResult {
    /// Whether the delegated task succeeded.
    pub success: bool,
    /// Output from the delegated agent.
    pub output: String,
    /// Agent that executed the task.
    pub agent_id: String,
    /// Iterations consumed.
    pub iterations: usize,
    /// Tool calls made.
    pub tool_calls: usize,
}

impl From<TaskOutput> for TaskToolResult {
    fn from(output: TaskOutput) -> Self {
        Self {
            success: output.success,
            output: output.output,
            agent_id: output.agent_id,
            iterations: output.iterations,
            tool_calls: output.tool_calls,
        }
    }
}

// ---------------------------------------------------------------------------
// Task tool execution
// ---------------------------------------------------------------------------

/// Execute the `task` tool: delegate work to another agent.
///
/// # Arguments
///
/// * `registry` - Agent definition registry
/// * `pool` - Agent instance pool
/// * `params` - Task tool parameters
/// * `conversation` - Current conversation history for context injection
/// * `current_depth` - Current delegation depth (for depth checking)
/// * `max_depth` - Maximum allowed delegation depth
pub fn execute_task_tool(
    registry: &AgentRegistry,
    pool: &mut AgentPool,
    params: &TaskToolParams,
    conversation: &[ContextMessage],
    current_depth: usize,
    max_depth: usize,
) -> Result<TaskToolResult, MultiAgentError> {
    // Depth check
    if current_depth >= max_depth {
        return Err(MultiAgentError::DelegationDepthExceeded {
            depth: current_depth + 1,
            max: max_depth,
        });
    }

    // Look up agent to determine default context strategy
    let definition = registry
        .get(&params.agent_name)
        .ok_or_else(|| MultiAgentError::NotFound {
            id: params.agent_name.clone(),
        })?;

    let context_strategy = params
        .context_strategy
        .unwrap_or(definition.context_sharing);

    // Prepare and execute
    let prepared = AgentExecutor::prepare(
        registry,
        pool,
        &params.agent_name,
        &params.prompt,
        conversation,
        params.mode,
        context_strategy,
        None,
    )?;

    let output = AgentExecutor::execute_simulated(pool, &prepared)?;
    Ok(TaskToolResult::from(output))
}

/// JSON Schema definition for the `task` tool (for tool registry integration).
pub fn task_tool_schema() -> serde_json::Value {
    serde_json::json!({
        "name": "task",
        "description": "Delegate a task to another agent. The agent will execute the task and return the result.",
        "parameters": {
            "type": "object",
            "properties": {
                "agent_name": {
                    "type": "string",
                    "description": "Name or ID of the target agent"
                },
                "prompt": {
                    "type": "string",
                    "description": "Task description/prompt to delegate"
                },
                "mode": {
                    "type": "string",
                    "enum": ["build", "plan", "explore", "general"],
                    "description": "Optional mode override for the delegation"
                },
                "context_strategy": {
                    "type": "string",
                    "enum": ["none", "summary", "filtered", "full"],
                    "description": "Optional context sharing strategy override"
                }
            },
            "required": ["agent_name", "prompt"]
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::config::MultiAgentConfig;

    fn setup() -> (AgentRegistry, AgentPool) {
        let registry = AgentRegistry::new();
        let pool = AgentPool::new(MultiAgentConfig::default());
        (registry, pool)
    }

    /// T-MA-R5-05: task tool parameter parsing and delegation.
    #[test]
    fn test_task_tool_execution() {
        let (registry, mut pool) = setup();

        let result = execute_task_tool(
            &registry,
            &mut pool,
            &TaskToolParams {
                agent_name: "tool-engineer".to_string(),
                prompt: "Build a new search tool".to_string(),
                mode: None,
                context_strategy: None,
            },
            &[], // no conversation
            0,   // depth 0
            3,   // max depth 3
        )
        .unwrap();

        assert!(result.success);
        assert_eq!(result.agent_id, "tool-engineer");
        assert!(!result.output.is_empty());
    }

    /// T-MA-R5-06: task tool nested invocation triggers depth check.
    #[test]
    fn test_task_tool_depth_check() {
        let (registry, mut pool) = setup();

        let result = execute_task_tool(
            &registry,
            &mut pool,
            &TaskToolParams {
                agent_name: "tool-engineer".to_string(),
                prompt: "deeply nested task".to_string(),
                mode: None,
                context_strategy: None,
            },
            &[],
            3, // current depth == max
            3, // max depth
        );

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            MultiAgentError::DelegationDepthExceeded { .. }
        ));
    }

    /// task tool with mode override.
    #[test]
    fn test_task_tool_mode_override() {
        let (registry, mut pool) = setup();

        let result = execute_task_tool(
            &registry,
            &mut pool,
            &TaskToolParams {
                agent_name: "agent-architect".to_string(),
                prompt: "Design a testing agent".to_string(),
                mode: Some(AgentMode::Plan),
                context_strategy: Some(ContextStrategy::Summary),
            },
            &[ContextMessage::user("We need better testing")],
            0,
            3,
        )
        .unwrap();

        assert!(result.success);
        assert_eq!(result.agent_id, "agent-architect");
    }

    /// task tool rejects unknown agent.
    #[test]
    fn test_task_tool_unknown_agent() {
        let (registry, mut pool) = setup();

        let result = execute_task_tool(
            &registry,
            &mut pool,
            &TaskToolParams {
                agent_name: "nonexistent".to_string(),
                prompt: "do something".to_string(),
                mode: None,
                context_strategy: None,
            },
            &[],
            0,
            3,
        );

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            MultiAgentError::NotFound { .. }
        ));
    }

    /// Schema is valid JSON.
    #[test]
    fn test_task_tool_schema() {
        let schema = task_tool_schema();
        assert_eq!(schema["name"], "task");
        assert!(schema["parameters"]["properties"]["agent_name"].is_object());
        assert!(schema["parameters"]["properties"]["prompt"].is_object());
    }
}
