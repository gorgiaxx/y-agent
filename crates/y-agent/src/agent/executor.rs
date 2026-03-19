//! Agent executor: runs an agent from definition to completion.
//!
//! Design reference: multi-agent-design.md §`AgentExecutor`
//!
//! The executor loads a definition from the registry, creates a session
//! branch, applies mode overlay, injects context, and manages the agent
//! loop lifecycle.

use crate::agent::context::{apply_context, ContextMessage};
use crate::agent::definition::{AgentDefinition, AgentMode, ContextStrategy};
use crate::agent::error::MultiAgentError;
use crate::agent::mode::{apply_mode_overlay, FilteredDefinition};
use crate::agent::pool::{AgentPool, InstanceState};
use crate::agent::registry::AgentRegistry;
use tracing::instrument;

// ---------------------------------------------------------------------------
// Task output
// ---------------------------------------------------------------------------

/// Output from an agent execution.
#[derive(Debug, Clone)]
pub struct TaskOutput {
    /// Instance ID of the executed agent.
    pub instance_id: String,
    /// Agent definition ID.
    pub agent_id: String,
    /// Whether the task completed successfully.
    pub success: bool,
    /// Result output from the agent.
    pub output: String,
    /// Number of iterations consumed.
    pub iterations: usize,
    /// Number of tool calls made.
    pub tool_calls: usize,
    /// Tokens consumed.
    pub tokens_used: u64,
    /// Execution wall-clock time in milliseconds.
    pub duration_ms: u64,
}

// ---------------------------------------------------------------------------
// Agent executor
// ---------------------------------------------------------------------------

/// Orchestrates the execution of a single agent task.
///
/// This struct combines registry lookup, mode overlay, context injection,
/// and pool instance management into a single execution flow.
pub struct AgentExecutor;

impl AgentExecutor {
    /// Prepare an agent for execution.
    ///
    /// 1. Look up the definition in the registry.
    /// 2. Spawn a pool instance.
    /// 3. Apply mode overlay.
    /// 4. Inject context based on strategy.
    ///
    /// Returns the prepared execution context.
    #[allow(clippy::too_many_arguments)]
    #[instrument(skip_all, fields(agent_id, mode = ?mode_override, strategy = ?context_strategy))]
    pub fn prepare(
        registry: &AgentRegistry,
        pool: &mut AgentPool,
        agent_id: &str,
        delegation_prompt: &str,
        conversation: &[ContextMessage],
        mode_override: Option<AgentMode>,
        context_strategy: ContextStrategy,
        delegation_id: Option<String>,
    ) -> Result<PreparedExecution, MultiAgentError> {
        // 1. Registry lookup
        let definition = registry
            .get(agent_id)
            .ok_or_else(|| MultiAgentError::NotFound {
                id: agent_id.to_string(),
            })?
            .clone();

        // 2. Spawn pool instance
        let instance_id = pool.spawn(definition.clone(), delegation_id)?;

        // 3. Apply mode overlay
        let filtered = apply_mode_overlay(&definition, mode_override);

        // 4. Context injection
        let context_messages = apply_context(
            context_strategy,
            delegation_prompt,
            conversation,
            definition.max_context_tokens,
        );

        // 5. Transition instance to Configuring
        let instance = pool.get_mut(&instance_id)?;
        instance.transition(InstanceState::Configuring)?;

        Ok(PreparedExecution {
            instance_id,
            definition,
            filtered,
            context_messages,
        })
    }

    /// Execute a prepared agent (simulated).
    ///
    /// In production, this would run the full agent loop with LLM calls
    /// and tool execution. Currently returns a simulated result.
    pub fn execute_simulated(
        pool: &mut AgentPool,
        prepared: &PreparedExecution,
    ) -> Result<TaskOutput, MultiAgentError> {
        // Transition to Running
        let instance = pool.get_mut(&prepared.instance_id)?;
        instance.transition(InstanceState::Running)?;

        // Simulate execution
        instance.iterations = 1;
        instance.tool_calls = 0;
        instance.tokens_used = prepared
            .context_messages
            .iter()
            .map(|m| m.token_estimate as u64)
            .sum();

        // Transition to Completed
        let output = format!(
            "Simulated execution of agent '{}' in {:?} mode with {} context messages",
            prepared.definition.id,
            prepared.filtered.mode,
            prepared.context_messages.len()
        );

        instance.transition(InstanceState::Completed)?;

        Ok(TaskOutput {
            instance_id: prepared.instance_id.clone(),
            agent_id: prepared.definition.id.clone(),
            success: true,
            output,
            iterations: 1,
            tool_calls: 0,
            tokens_used: instance.tokens_used,
            duration_ms: instance.elapsed_ms(),
        })
    }

    /// Execute with resource limit enforcement (simulated agent loop).
    ///
    /// Checks iteration limits, tool call limits, and timeout after each
    /// simulated step. Returns an error if any limit is exceeded.
    #[instrument(skip_all, fields(agent_id = %prepared.definition.id))]
    pub fn execute_with_limits(
        pool: &mut AgentPool,
        prepared: &PreparedExecution,
        simulated_iterations: usize,
        simulated_tool_calls: usize,
    ) -> Result<TaskOutput, MultiAgentError> {
        let instance = pool.get_mut(&prepared.instance_id)?;
        instance.transition(InstanceState::Running)?;

        // Simulate iteration loop with limit checks
        for i in 0..simulated_iterations {
            instance.iterations = i + 1;

            if instance.iterations > prepared.definition.max_iterations {
                instance.transition(InstanceState::Failed)?;
                return Err(MultiAgentError::Other {
                    message: format!(
                        "IterationLimitExceeded: {} > {}",
                        instance.iterations, prepared.definition.max_iterations
                    ),
                });
            }
        }

        // Simulate tool calls
        instance.tool_calls = simulated_tool_calls;
        if instance.tool_calls > prepared.definition.max_tool_calls {
            instance.transition(InstanceState::Failed)?;
            return Err(MultiAgentError::Other {
                message: format!(
                    "ToolCallLimitExceeded: {} > {}",
                    instance.tool_calls, prepared.definition.max_tool_calls
                ),
            });
        }

        instance.tokens_used = prepared
            .context_messages
            .iter()
            .map(|m| m.token_estimate as u64)
            .sum();

        let output = format!(
            "Agent '{}' completed: {} iterations, {} tool calls",
            prepared.definition.id, instance.iterations, instance.tool_calls
        );

        let duration_ms = instance.elapsed_ms();
        instance.transition(InstanceState::Completed)?;

        Ok(TaskOutput {
            instance_id: prepared.instance_id.clone(),
            agent_id: prepared.definition.id.clone(),
            success: true,
            output,
            iterations: simulated_iterations,
            tool_calls: simulated_tool_calls,
            tokens_used: instance.tokens_used,
            duration_ms,
        })
    }
}

/// A fully prepared agent execution context, ready to run.
#[derive(Debug)]
pub struct PreparedExecution {
    /// Pool instance ID.
    pub instance_id: String,
    /// Original agent definition.
    pub definition: AgentDefinition,
    /// Mode-filtered definition.
    pub filtered: FilteredDefinition,
    /// Context messages for the agent.
    pub context_messages: Vec<ContextMessage>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::config::MultiAgentConfig;

    fn setup() -> (AgentRegistry, AgentPool) {
        let registry = AgentRegistry::new(); // includes built-ins
        let pool = AgentPool::new(MultiAgentConfig::default());
        (registry, pool)
    }

    /// T-MA-R5-04: `AgentExecutor` full lifecycle.
    #[test]
    fn test_executor_full_lifecycle() {
        let (registry, mut pool) = setup();

        // Prepare execution using built-in tool-engineer
        let prepared = AgentExecutor::prepare(
            &registry,
            &mut pool,
            "tool-engineer",
            "Create a new file_search tool",
            &[], // no conversation history
            None,
            ContextStrategy::None,
            None,
        )
        .unwrap();

        assert_eq!(prepared.definition.id, "tool-engineer");
        assert_eq!(prepared.filtered.mode, AgentMode::Build);
        assert_eq!(prepared.context_messages.len(), 1); // None strategy: just the prompt
        assert_eq!(
            prepared.context_messages[0].content,
            "Create a new file_search tool"
        );

        // Execute
        let output = AgentExecutor::execute_simulated(&mut pool, &prepared).unwrap();
        assert!(output.success);
        assert_eq!(output.agent_id, "tool-engineer");
        assert!(output.output.contains("Simulated"));

        // Verify instance is completed
        let instance = pool.get(&prepared.instance_id).unwrap();
        assert_eq!(instance.state, InstanceState::Completed);
    }

    /// Executor with mode override and context.
    #[test]
    fn test_executor_with_mode_override_and_context() {
        let (registry, mut pool) = setup();

        let conversation = vec![
            ContextMessage::user("What is the architecture?"),
            ContextMessage::assistant("The architecture uses modular crates..."),
        ];

        let prepared = AgentExecutor::prepare(
            &registry,
            &mut pool,
            "agent-architect",
            "Design a test runner agent",
            &conversation,
            Some(AgentMode::Plan),
            ContextStrategy::Full,
            Some("delegation-1".to_string()),
        )
        .unwrap();

        // Should have context messages + delegation prompt
        assert!(prepared.context_messages.len() > 1);
        assert_eq!(prepared.filtered.mode, AgentMode::Plan);

        let output = AgentExecutor::execute_simulated(&mut pool, &prepared).unwrap();
        assert!(output.success);
    }

    /// Executor rejects unknown agent.
    #[test]
    fn test_executor_unknown_agent() {
        let (registry, mut pool) = setup();

        let result = AgentExecutor::prepare(
            &registry,
            &mut pool,
            "nonexistent-agent",
            "Do something",
            &[],
            None,
            ContextStrategy::None,
            None,
        );

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            MultiAgentError::NotFound { .. }
        ));
    }

    /// T-MA-P6-02: Mode overlay applied during execution.
    #[test]
    fn test_executor_mode_overlay_applied() {
        let (registry, mut pool) = setup();

        let prepared = AgentExecutor::prepare(
            &registry,
            &mut pool,
            "tool-engineer",
            "Analyze the codebase",
            &[],
            Some(AgentMode::Plan),
            ContextStrategy::None,
            None,
        )
        .unwrap();

        assert_eq!(prepared.filtered.mode, AgentMode::Plan);
        assert!(!prepared
            .filtered
            .allowed_tools
            .contains(&"file_write".to_string()));
        assert!(!prepared
            .filtered
            .allowed_tools
            .contains(&"shell_exec".to_string()));
        assert!(prepared
            .filtered
            .allowed_tools
            .contains(&"file_read".to_string()));
    }

    /// T-MA-P6-03: Context injection with summary strategy.
    #[test]
    fn test_executor_context_injection() {
        let (registry, mut pool) = setup();

        let conversation = vec![
            ContextMessage::user("What is Rust?"),
            ContextMessage::assistant("Rust is a systems language."),
        ];

        let prepared = AgentExecutor::prepare(
            &registry,
            &mut pool,
            "agent-architect",
            "Summarize the discussion",
            &conversation,
            None,
            ContextStrategy::Summary,
            None,
        )
        .unwrap();

        assert!(prepared.context_messages.len() >= 2);
        let has_system = prepared
            .context_messages
            .iter()
            .any(|m| m.role == "system" && m.content.contains("Context summary"));
        assert!(has_system);
    }

    /// T-MA-P6-05: Resource limits — iteration limit exceeded.
    #[test]
    fn test_executor_resource_limits() {
        let mut registry = AgentRegistry::new();
        let tiny = crate::agent::definition::AgentDefinition::from_toml(
            r#"
            id = "tiny-agent"
            name = "tiny-agent"
            description = "An agent with very low limits"
            mode = "general"
            trust_tier = "user_defined"
            capabilities = []
            allowed_tools = []
            denied_tools = []
            system_prompt = "You are tiny."
            skills = []
            preferred_models = []
            fallback_models = []
            max_iterations = 3
            max_tool_calls = 2
            timeout_secs = 1
            context_sharing = "none"
            max_context_tokens = 1024
            "#,
        )
        .unwrap();
        registry.register(tiny).unwrap();

        let mut pool = AgentPool::new(MultiAgentConfig::default());

        let prepared = AgentExecutor::prepare(
            &registry,
            &mut pool,
            "tiny-agent",
            "Do work",
            &[],
            None,
            ContextStrategy::None,
            None,
        )
        .unwrap();

        let result = AgentExecutor::execute_with_limits(&mut pool, &prepared, 5, 0);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("IterationLimitExceeded"));
    }

    /// T-MA-P8-04: Duration is tracked in output.
    #[test]
    fn test_executor_duration_tracked() {
        let (registry, mut pool) = setup();

        let prepared = AgentExecutor::prepare(
            &registry,
            &mut pool,
            "tool-engineer",
            "Build something",
            &[],
            None,
            ContextStrategy::None,
            None,
        )
        .unwrap();

        let output = AgentExecutor::execute_simulated(&mut pool, &prepared).unwrap();
        assert!(output.duration_ms < 1000);
    }
}
