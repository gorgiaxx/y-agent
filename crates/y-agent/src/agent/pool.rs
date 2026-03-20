//! Agent pool: runtime instance management.
//!
//! Design reference: multi-agent-design.md §Agent Pool
//!
//! The pool manages `AgentInstance` runtime instances (as opposed to the
//! `AgentRegistry` which manages static definitions). It enforces
//! concurrency limits, tracks per-instance resource usage, and manages
//! the instance lifecycle state machine.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Instant;

use async_trait::async_trait;
use tokio::sync::Semaphore;
use uuid::Uuid;

use y_core::agent::{
    AgentDelegator, AgentRunConfig, AgentRunner, ContextStrategyHint, DelegationError,
    DelegationOutput,
};

use crate::agent::config::MultiAgentConfig;
use crate::agent::definition::AgentDefinition;
use crate::agent::error::MultiAgentError;
use crate::agent::registry::AgentRegistry;

// ---------------------------------------------------------------------------
// Instance lifecycle state machine
// ---------------------------------------------------------------------------

/// Lifecycle state of a runtime agent instance.
///
/// ```text
/// Creating → Configuring → Running → Completed | Failed | Interrupted
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstanceState {
    /// Instance is being created.
    Creating,
    /// Instance is being configured (mode overlay, context injection).
    Configuring,
    /// Instance is actively executing.
    Running,
    /// Instance completed its task successfully.
    Completed,
    /// Instance failed during execution.
    Failed,
    /// Instance was interrupted (e.g., by user or timeout).
    Interrupted,
}

impl InstanceState {
    /// Whether this state is terminal (no further transitions).
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Interrupted)
    }
}

// ---------------------------------------------------------------------------
// Agent instance
// ---------------------------------------------------------------------------

/// A runtime agent instance with resource tracking.
#[derive(Debug, Clone)]
pub struct AgentInstance {
    /// Unique instance ID.
    pub instance_id: String,
    /// The definition this instance was created from.
    pub definition: AgentDefinition,
    /// Current lifecycle state.
    pub state: InstanceState,
    /// Which delegation spawned this instance (if any).
    pub delegation_id: Option<String>,
    /// Number of loop iterations consumed.
    pub iterations: usize,
    /// Number of tool calls made.
    pub tool_calls: usize,
    /// Approximate tokens consumed.
    pub tokens_used: u64,
    /// When this instance was created (monotonic clock).
    pub start_time: Instant,
    /// Remaining delegation depth (0 = cannot delegate further).
    pub delegation_depth: u32,
}

impl AgentInstance {
    /// Create a new instance from a definition.
    fn new(definition: AgentDefinition, delegation_id: Option<String>) -> Self {
        Self {
            instance_id: Uuid::new_v4().to_string(),
            definition,
            state: InstanceState::Creating,
            delegation_id,
            iterations: 0,
            tool_calls: 0,
            tokens_used: 0,
            start_time: Instant::now(),
            delegation_depth: 0,
        }
    }

    /// Create with a specific delegation depth.
    fn with_depth(
        definition: AgentDefinition,
        delegation_id: Option<String>,
        delegation_depth: u32,
    ) -> Self {
        Self {
            delegation_depth,
            ..Self::new(definition, delegation_id)
        }
    }

    /// Elapsed time since creation in milliseconds.
    pub fn elapsed_ms(&self) -> u64 {
        u64::try_from(self.start_time.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// Transition to the next valid state.
    pub fn transition(&mut self, next: InstanceState) -> Result<(), MultiAgentError> {
        let valid = matches!(
            (self.state, next),
            (
                InstanceState::Creating,
                InstanceState::Configuring | InstanceState::Running
            ) | (InstanceState::Configuring, InstanceState::Running)
                | (
                    InstanceState::Running,
                    InstanceState::Completed | InstanceState::Failed | InstanceState::Interrupted
                )
        );

        if valid {
            self.state = next;
            Ok(())
        } else {
            Err(MultiAgentError::Other {
                message: format!("invalid state transition: {:?} -> {:?}", self.state, next),
            })
        }
    }
}

// ---------------------------------------------------------------------------
// Delegation tracker -- observability for delegated agent executions
// ---------------------------------------------------------------------------

/// A currently-active delegated agent execution.
#[derive(Debug, Clone)]
pub struct ActiveDelegation {
    /// Unique ID for this delegation.
    pub id: String,
    /// Name of the agent being executed (e.g. "title-generator").
    pub agent_name: String,
    /// When this delegation started.
    pub start_time: Instant,
}

/// Tracks active delegated agent executions.
///
/// `AgentPool::delegate()` bypasses the pool's `instances` `HashMap` and
/// calls `AgentRunner::run()` directly. This tracker provides interior-
/// mutable bookkeeping so that observability can see those executions.
#[derive(Debug, Default)]
pub struct DelegationTracker {
    active: std::sync::RwLock<Vec<ActiveDelegation>>,
}

impl DelegationTracker {
    /// Create a new empty tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a delegation as starting. Returns the delegation ID.
    fn register(&self, agent_name: &str) -> String {
        let id = Uuid::new_v4().to_string();
        let entry = ActiveDelegation {
            id: id.clone(),
            agent_name: agent_name.to_string(),
            start_time: Instant::now(),
        };
        if let Ok(mut active) = self.active.write() {
            active.push(entry);
        }
        id
    }

    /// Deregister a delegation by its ID.
    fn deregister(&self, id: &str) {
        if let Ok(mut active) = self.active.write() {
            active.retain(|d| d.id != id);
        }
    }

    /// Snapshot of all active delegations.
    pub fn active_delegations(&self) -> Vec<ActiveDelegation> {
        self.active.read().map(|v| v.clone()).unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------
// Agent pool
// ---------------------------------------------------------------------------

/// Manages the lifecycle of runtime agent instances.
///
/// Enforces concurrency limits via a semaphore and tracks per-instance
/// resource usage (iterations, tool calls, tokens). Also implements
/// `AgentDelegator` to serve as the cross-module invocation endpoint.
pub struct AgentPool {
    config: MultiAgentConfig,
    instances: HashMap<String, AgentInstance>,
    /// Semaphore controlling max concurrent running instances.
    concurrency_semaphore: Semaphore,
    /// Shared reference to the agent registry for definition lookup.
    registry: Arc<RwLock<AgentRegistry>>,
    /// Optional runner for executing agent LLM calls.
    /// Injected at startup via `set_runner()`.
    runner: Option<Arc<dyn AgentRunner>>,
    /// Tracks active delegations for observability.
    delegation_tracker: Arc<DelegationTracker>,
}

impl std::fmt::Debug for AgentPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentPool")
            .field("config", &self.config)
            .field("instances", &self.instances.len())
            .field("runner", &self.runner.is_some())
            .finish_non_exhaustive()
    }
}

impl AgentPool {
    pub fn new(config: MultiAgentConfig) -> Self {
        let max = config.max_concurrent_agents;
        Self {
            config,
            instances: HashMap::new(),
            concurrency_semaphore: Semaphore::new(max),
            registry: Arc::new(RwLock::new(AgentRegistry::new())),
            runner: None,
            delegation_tracker: Arc::new(DelegationTracker::new()),
        }
    }

    /// Create a pool with a shared registry.
    pub fn with_registry(config: MultiAgentConfig, registry: Arc<RwLock<AgentRegistry>>) -> Self {
        let max = config.max_concurrent_agents;
        Self {
            config,
            instances: HashMap::new(),
            concurrency_semaphore: Semaphore::new(max),
            registry,
            runner: None,
            delegation_tracker: Arc::new(DelegationTracker::new()),
        }
    }

    /// Inject the agent runner for real LLM execution.
    ///
    /// Must be called at startup before any `delegate()` calls.
    /// Without a runner, `delegate()` returns `DelegationFailed`.
    pub fn set_runner(&mut self, runner: Arc<dyn AgentRunner>) {
        self.runner = Some(runner);
    }

    /// Get a reference to the shared registry.
    pub fn registry(&self) -> &Arc<RwLock<AgentRegistry>> {
        &self.registry
    }

    /// Spawn a new agent instance from a definition.
    ///
    /// Returns the instance ID. Does NOT acquire a concurrency permit yet —
    /// that happens when the instance transitions to `Running`.
    pub fn spawn(
        &mut self,
        definition: AgentDefinition,
        delegation_id: Option<String>,
    ) -> Result<String, MultiAgentError> {
        // Check per-delegation limit if applicable
        if let Some(ref del_id) = delegation_id {
            let count = self
                .instances
                .values()
                .filter(|i| i.delegation_id.as_deref() == Some(del_id) && !i.state.is_terminal())
                .count();
            if count >= self.config.max_agents_per_delegation {
                return Err(MultiAgentError::PoolLimitReached {
                    max: self.config.max_agents_per_delegation,
                });
            }
        }

        let instance = AgentInstance::new(definition, delegation_id);
        let id = instance.instance_id.clone();
        self.instances.insert(id.clone(), instance);
        Ok(id)
    }

    /// Spawn a new agent instance with a specific delegation depth.
    ///
    /// Used when creating sub-agents: the parent's depth - 1 is passed.
    /// Returns an error if `delegation_depth` is 0 (no further delegation allowed).
    pub fn spawn_with_depth(
        &mut self,
        definition: AgentDefinition,
        delegation_id: Option<String>,
        delegation_depth: u32,
    ) -> Result<String, MultiAgentError> {
        if delegation_depth == 0 {
            return Err(MultiAgentError::DelegationDepthExceeded {
                depth: 0,
                max: self.config.max_delegation_depth,
            });
        }

        // Check per-delegation limit if applicable
        if let Some(ref del_id) = delegation_id {
            let count = self
                .instances
                .values()
                .filter(|i| i.delegation_id.as_deref() == Some(del_id) && !i.state.is_terminal())
                .count();
            if count >= self.config.max_agents_per_delegation {
                return Err(MultiAgentError::PoolLimitReached {
                    max: self.config.max_agents_per_delegation,
                });
            }
        }

        let instance = AgentInstance::with_depth(definition, delegation_id, delegation_depth);
        let id = instance.instance_id.clone();
        self.instances.insert(id.clone(), instance);
        Ok(id)
    }

    /// Configured maximum delegation depth.
    pub fn max_delegation_depth(&self) -> usize {
        self.config.max_delegation_depth
    }

    /// Get an instance by ID.
    pub fn get(&self, instance_id: &str) -> Result<&AgentInstance, MultiAgentError> {
        self.instances
            .get(instance_id)
            .ok_or_else(|| MultiAgentError::NotFound {
                id: instance_id.to_string(),
            })
    }

    /// Get a mutable instance by ID.
    pub fn get_mut(&mut self, instance_id: &str) -> Result<&mut AgentInstance, MultiAgentError> {
        self.instances
            .get_mut(instance_id)
            .ok_or_else(|| MultiAgentError::NotFound {
                id: instance_id.to_string(),
            })
    }

    /// List all non-terminal instances.
    pub fn list_active(&self) -> Vec<&AgentInstance> {
        self.instances
            .values()
            .filter(|i| !i.state.is_terminal())
            .collect()
    }

    /// List all instances (including terminal).
    pub fn list_all(&self) -> Vec<&AgentInstance> {
        self.instances.values().collect()
    }

    /// Try to acquire a concurrency permit for running.
    ///
    /// Returns `true` if a permit was acquired. The caller must
    /// transition the instance to `Running` only after acquiring.
    pub fn try_acquire_concurrency(&self) -> bool {
        self.concurrency_semaphore.try_acquire().is_ok()
    }

    /// Number of available concurrency slots.
    pub fn available_slots(&self) -> usize {
        self.concurrency_semaphore.available_permits()
    }

    /// Clean up terminal instances older than the retain window.
    pub fn gc_terminal(&mut self) {
        self.instances.retain(|_, inst| !inst.state.is_terminal());
    }

    /// Total number of instances (including terminal).
    pub fn count(&self) -> usize {
        self.instances.len()
    }

    /// Get the shared delegation tracker for observability.
    pub fn delegation_tracker(&self) -> &Arc<DelegationTracker> {
        &self.delegation_tracker
    }
}

// ---------------------------------------------------------------------------
// AgentDelegator implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl AgentDelegator for AgentPool {
    /// Delegate a task to a named agent.
    ///
    /// 1. Resolve the agent definition from the registry.
    /// 2. Build an `AgentRunConfig` from the definition and input.
    /// 3. Execute via the injected `AgentRunner`.
    /// 4. Map `AgentRunOutput` → `DelegationOutput`.
    async fn delegate(
        &self,
        agent_name: &str,
        input: serde_json::Value,
        _context_strategy: ContextStrategyHint,
    ) -> Result<DelegationOutput, DelegationError> {
        // Step 1: Resolve agent definition from registry (scoped to drop guard before await)
        let definition = {
            let registry = self
                .registry
                .read()
                .map_err(|_| DelegationError::DelegationFailed {
                    message: "registry lock poisoned".to_string(),
                })?;

            registry
                .get(agent_name)
                .ok_or_else(|| DelegationError::AgentNotFound {
                    name: agent_name.to_string(),
                })?
                .clone()
        };

        // Step 2: Build AgentRunConfig from definition
        let runner = self
            .runner
            .as_ref()
            .ok_or_else(|| DelegationError::DelegationFailed {
                message: format!(
                    "no AgentRunner configured -- cannot execute agent '{agent_name}'. \
                 Call AgentPool::set_runner() at startup.",
                ),
            })?;

        let config = AgentRunConfig {
            agent_name: definition.id.clone(),
            system_prompt: definition.system_prompt.clone(),
            input,
            preferred_models: definition.preferred_models.clone(),
            fallback_models: definition.fallback_models.clone(),
            provider_tags: definition.provider_tags.clone(),
            temperature: definition.temperature,
            max_tokens: Some(u32::try_from(definition.max_context_tokens).unwrap_or(u32::MAX)),
            timeout_secs: definition.timeout_secs,
            allowed_tools: definition.allowed_tools.clone(),
            denied_tools: definition.denied_tools.clone(),
            max_iterations: definition.max_iterations,
        };

        // Register for observability before execution.
        let delegation_id = self.delegation_tracker.register(agent_name);

        // Step 3: Execute via runner
        let result = runner.run(config).await;

        // Deregister after completion (success or failure).
        self.delegation_tracker.deregister(&delegation_id);

        // Step 4: Map to DelegationOutput
        let output = result?;
        Ok(DelegationOutput {
            text: output.text,
            tokens_used: output.tokens_used,
            input_tokens: output.input_tokens,
            output_tokens: output.output_tokens,
            model_used: output.model_used,
            duration_ms: output.duration_ms,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::definition::{AgentMode, ContextStrategy};
    use crate::agent::trust::TrustTier;

    fn test_definition(id: &str) -> AgentDefinition {
        AgentDefinition {
            id: id.to_string(),
            name: format!("Agent {id}"),
            description: "test".to_string(),
            mode: AgentMode::General,
            trust_tier: TrustTier::UserDefined,
            capabilities: vec![],
            allowed_tools: vec![],
            denied_tools: vec![],
            system_prompt: String::new(),
            skills: vec![],
            preferred_models: vec![],
            fallback_models: vec![],
            provider_tags: vec![],
            temperature: None,
            top_p: None,
            max_iterations: 20,
            max_tool_calls: 50,
            timeout_secs: 300,
            context_sharing: ContextStrategy::None,
            max_context_tokens: 4096,
        }
    }

    /// T-MA-R2-04: Pool rejects when `max_agents_per_delegation` exceeded.
    #[test]
    fn test_pool_per_delegation_limit() {
        let config = MultiAgentConfig {
            max_agents_per_delegation: 2,
            ..Default::default()
        };
        let mut pool = AgentPool::new(config);
        let del_id = Some("del-1".to_string());

        pool.spawn(test_definition("a1"), del_id.clone()).unwrap();
        pool.spawn(test_definition("a2"), del_id.clone()).unwrap();

        let result = pool.spawn(test_definition("a3"), del_id);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            MultiAgentError::PoolLimitReached { .. }
        ));
    }

    /// T-MA-R2-05: Pool instance lifecycle state machine transitions.
    #[test]
    fn test_pool_instance_state_machine() {
        let mut pool = AgentPool::new(MultiAgentConfig::default());
        let id = pool.spawn(test_definition("a1"), None).unwrap();

        // Creating → Configuring → Running → Completed
        {
            let inst = pool.get_mut(&id).unwrap();
            assert_eq!(inst.state, InstanceState::Creating);
            inst.transition(InstanceState::Configuring).unwrap();
            assert_eq!(inst.state, InstanceState::Configuring);
            inst.transition(InstanceState::Running).unwrap();
            assert_eq!(inst.state, InstanceState::Running);
            inst.transition(InstanceState::Completed).unwrap();
            assert_eq!(inst.state, InstanceState::Completed);
        }

        // Terminal state should not allow further transitions
        let inst = pool.get_mut(&id).unwrap();
        assert!(inst.transition(InstanceState::Running).is_err());
    }

    /// T-MA-R2-06: Pool per-instance resource tracking (iterations, `tool_calls`).
    #[test]
    fn test_pool_resource_tracking() {
        let mut pool = AgentPool::new(MultiAgentConfig::default());
        let id = pool.spawn(test_definition("a1"), None).unwrap();

        let inst = pool.get_mut(&id).unwrap();
        inst.transition(InstanceState::Running).unwrap();
        inst.iterations = 5;
        inst.tool_calls = 12;
        inst.tokens_used = 4096;

        let inst = pool.get(&id).unwrap();
        assert_eq!(inst.iterations, 5);
        assert_eq!(inst.tool_calls, 12);
        assert_eq!(inst.tokens_used, 4096);
    }

    /// Concurrency semaphore limits active instances.
    #[test]
    fn test_pool_concurrency_semaphore() {
        let config = MultiAgentConfig {
            max_concurrent_agents: 2,
            ..Default::default()
        };
        let pool = AgentPool::new(config);

        assert_eq!(pool.available_slots(), 2);

        // Acquire and forget permits (simulating held concurrent instances)
        let p1 = pool.concurrency_semaphore.try_acquire().unwrap();
        p1.forget();
        assert_eq!(pool.available_slots(), 1);

        let p2 = pool.concurrency_semaphore.try_acquire().unwrap();
        p2.forget();
        assert_eq!(pool.available_slots(), 0);

        // Third acquire should fail
        assert!(!pool.try_acquire_concurrency());
    }

    /// `list_active` excludes terminal instances.
    #[test]
    fn test_pool_list_active() {
        let mut pool = AgentPool::new(MultiAgentConfig::default());
        let id1 = pool.spawn(test_definition("a1"), None).unwrap();
        let _id2 = pool.spawn(test_definition("a2"), None).unwrap();

        // Complete instance 1
        {
            let inst = pool.get_mut(&id1).unwrap();
            inst.transition(InstanceState::Running).unwrap();
            inst.transition(InstanceState::Completed).unwrap();
        }

        let active = pool.list_active();
        assert_eq!(active.len(), 1);
    }

    /// `gc_terminal` removes completed instances.
    #[test]
    fn test_pool_gc_terminal() {
        let mut pool = AgentPool::new(MultiAgentConfig::default());
        let id1 = pool.spawn(test_definition("a1"), None).unwrap();
        let _id2 = pool.spawn(test_definition("a2"), None).unwrap();

        {
            let inst = pool.get_mut(&id1).unwrap();
            inst.transition(InstanceState::Running).unwrap();
            inst.transition(InstanceState::Completed).unwrap();
        }

        assert_eq!(pool.count(), 2);
        pool.gc_terminal();
        assert_eq!(pool.count(), 1);
    }

    /// T-MA-P2-05: `AgentPool` implements `AgentDelegator` — delegates to a known agent.
    #[tokio::test]
    async fn test_agent_pool_implements_delegator() {
        use y_core::agent::{AgentRunConfig, AgentRunOutput, AgentRunner};

        struct MockRunner;

        #[async_trait::async_trait]
        impl AgentRunner for MockRunner {
            async fn run(&self, config: AgentRunConfig) -> Result<AgentRunOutput, DelegationError> {
                Ok(AgentRunOutput {
                    text: format!("agent '{}' responded", config.agent_name),
                    tokens_used: 10,
                    input_tokens: 8,
                    output_tokens: 2,
                    model_used: "mock".to_string(),
                    duration_ms: 5,
                })
            }
        }

        let mut pool = AgentPool::new(MultiAgentConfig::default());
        pool.set_runner(Arc::new(MockRunner));

        // The pool creates a default registry with built-in agents (tool-engineer, agent-architect)
        let result = pool
            .delegate(
                "tool-engineer",
                serde_json::json!({"task": "test"}),
                ContextStrategyHint::None,
            )
            .await;

        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(output.text.contains("tool-engineer"));
    }

    /// T-MA-P2-06: Delegation to unknown agent returns `AgentNotFound`.
    #[tokio::test]
    async fn test_delegation_unknown_agent() {
        let pool = AgentPool::new(MultiAgentConfig::default());

        let result = pool
            .delegate(
                "nonexistent-agent",
                serde_json::json!({}),
                ContextStrategyHint::None,
            )
            .await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            DelegationError::AgentNotFound { .. }
        ));
    }

    /// T-MA-P2-07: Delegation with structured JSON input.
    #[tokio::test]
    async fn test_delegation_structured_input() {
        use y_core::agent::{AgentRunConfig, AgentRunOutput, AgentRunner};

        struct MockRunner;

        #[async_trait::async_trait]
        impl AgentRunner for MockRunner {
            async fn run(&self, config: AgentRunConfig) -> Result<AgentRunOutput, DelegationError> {
                Ok(AgentRunOutput {
                    text: format!("processed input: {}", config.input),
                    tokens_used: 15,
                    input_tokens: 10,
                    output_tokens: 5,
                    model_used: "mock".to_string(),
                    duration_ms: 10,
                })
            }
        }

        let mut pool = AgentPool::new(MultiAgentConfig::default());
        pool.set_runner(Arc::new(MockRunner));

        let input = serde_json::json!({
            "messages": [
                {"role": "user", "content": "hello"},
                {"role": "assistant", "content": "hi"}
            ],
            "message_count": 2,
        });

        let result = pool
            .delegate("agent-architect", input, ContextStrategyHint::Summary)
            .await;

        assert!(result.is_ok());
    }

    /// T-MA-P2-08: Delegation without runner returns DelegationFailed.
    #[tokio::test]
    async fn test_pool_delegate_without_runner_errors() {
        let pool = AgentPool::new(MultiAgentConfig::default());

        let result = pool
            .delegate(
                "tool-engineer",
                serde_json::json!({}),
                ContextStrategyHint::None,
            )
            .await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            DelegationError::DelegationFailed { .. }
        ));
    }

    /// T-MA-P3-10: `spawn_with_depth(0)` returns `DelegationDepthExceeded`.
    #[test]
    fn test_pool_delegation_depth_rejection() {
        let mut pool = AgentPool::new(MultiAgentConfig::default());
        let result = pool.spawn_with_depth(test_definition("a1"), None, 0);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            MultiAgentError::DelegationDepthExceeded { .. }
        ));
    }

    /// T-MA-P3-09b: Instance tracks `start_time` and `elapsed_ms`.
    #[test]
    fn test_pool_instance_start_time() {
        let mut pool = AgentPool::new(MultiAgentConfig::default());
        let id = pool.spawn(test_definition("a1"), None).unwrap();

        let inst = pool.get(&id).unwrap();
        // Elapsed should be very small (microsecond range)
        assert!(inst.elapsed_ms() < 1000);
    }

    /// T-MA-P3-10b: `spawn_with_depth` creates instance with correct depth.
    #[test]
    fn test_pool_spawn_with_depth() {
        let mut pool = AgentPool::new(MultiAgentConfig::default());

        // Parent has depth 3
        let id = pool
            .spawn_with_depth(test_definition("a1"), Some("del-1".to_string()), 3)
            .unwrap();

        let inst = pool.get(&id).unwrap();
        assert_eq!(inst.delegation_depth, 3);

        // Child would be spawned with depth - 1 = 2
        let child_id = pool
            .spawn_with_depth(test_definition("a2"), Some("del-1".to_string()), 2)
            .unwrap();
        let child = pool.get(&child_id).unwrap();
        assert_eq!(child.delegation_depth, 2);
    }
}
