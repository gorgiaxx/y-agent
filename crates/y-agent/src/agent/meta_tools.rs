//! Meta-tools for agent lifecycle management.
//!
//! Design reference: multi-agent-design.md §Meta-Tool System
//!
//! These tools allow agents to create, update, deactivate, and search
//! for other agent definitions at runtime.

use serde::{Deserialize, Serialize};

use crate::agent::definition::{AgentMode, ContextStrategy};
use crate::agent::dynamic_agent::{
    make_dynamic_agent, AgentStatus, CreatorPermissionSnapshot, DynamicAgentDefinition,
    DynamicAgentStoreBackend,
};
use crate::agent::trust::TrustTier;

// ---------------------------------------------------------------------------
// Meta-tool parameter types
// ---------------------------------------------------------------------------

/// Parameters for the `agent_create` tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCreateParams {
    /// Name for the new agent.
    pub name: String,
    /// Description of the agent's purpose.
    pub description: String,
    /// Default behavioral mode.
    #[serde(default = "default_mode")]
    pub mode: AgentMode,
    /// Capabilities/tags for search.
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// Tools the agent is allowed to use.
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    /// Tools the agent is explicitly denied.
    #[serde(default)]
    pub denied_tools: Vec<String>,
    /// System prompt for the agent.
    #[serde(default)]
    pub system_prompt: String,
    /// Context sharing strategy.
    #[serde(default)]
    pub context_sharing: ContextStrategy,
}

fn default_mode() -> AgentMode {
    AgentMode::General
}

/// Parameters for the `agent_update` tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentUpdateParams {
    /// ID of the agent to update.
    pub id: String,
    /// Updated description (if provided).
    pub description: Option<String>,
    /// Updated mode (if provided).
    pub mode: Option<AgentMode>,
    /// Updated tool list (if provided).
    pub allowed_tools: Option<Vec<String>>,
    /// Updated system prompt (if provided).
    pub system_prompt: Option<String>,
}

/// Parameters for the `agent_deactivate` tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDeactivateParams {
    /// ID of the agent to deactivate.
    pub id: String,
    /// Reason for deactivation.
    pub reason: String,
}

/// Parameters for the `agent_search` tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSearchParams {
    /// Search query (matches name, description, capabilities).
    pub query: String,
    /// Filter by mode (optional).
    pub mode: Option<AgentMode>,
    /// Filter by trust tier (optional).
    pub trust_tier: Option<TrustTier>,
    /// Filter by status (optional).
    #[serde(default = "default_status_filter")]
    pub status: Option<AgentStatus>,
}

fn default_status_filter() -> Option<AgentStatus> {
    Some(AgentStatus::Active)
}

// ---------------------------------------------------------------------------
// Meta-tool result types
// ---------------------------------------------------------------------------

/// Result from a meta-tool execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetaToolResult {
    /// Whether the operation succeeded.
    pub success: bool,
    /// Result message.
    pub message: String,
    /// Created/updated agent definition (if applicable).
    pub agent: Option<DynamicAgentDefinition>,
    /// Search results (if applicable).
    pub agents: Option<Vec<DynamicAgentDefinition>>,
}

impl MetaToolResult {
    fn success(message: &str, agent: Option<DynamicAgentDefinition>) -> Self {
        Self {
            success: true,
            message: message.to_string(),
            agent,
            agents: None,
        }
    }

    fn search_result(agents: Vec<DynamicAgentDefinition>) -> Self {
        Self {
            success: true,
            message: format!("Found {} agent(s)", agents.len()),
            agent: None,
            agents: Some(agents),
        }
    }

    fn error(message: &str) -> Self {
        Self {
            success: false,
            message: message.to_string(),
            agent: None,
            agents: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Meta-tool execution
// ---------------------------------------------------------------------------

/// Create a new dynamic agent definition.
///
/// Performs the three-stage validation pipeline (Schema → Permission → Safety).
pub fn agent_create(
    store: &dyn DynamicAgentStoreBackend,
    params: AgentCreateParams,
    creator_id: &str,
    creator_snapshot: &CreatorPermissionSnapshot,
) -> MetaToolResult {
    let agent = make_dynamic_agent(
        &params.name,
        &params.description,
        creator_id,
        &params.allowed_tools,
        creator_snapshot,
    );

    // Apply optional fields
    let mut agent = agent;
    agent.definition.mode = params.mode;
    agent.definition.capabilities = params.capabilities;
    agent.definition.denied_tools = params.denied_tools;
    agent.definition.system_prompt = params.system_prompt;
    agent.definition.context_sharing = params.context_sharing;

    // Re-compute effective permissions with the updated denied_tools
    // (the helper already computed them, but denied_tools may have changed)

    match store.create(agent) {
        Ok(created) => MetaToolResult::success(
            &format!("Agent '{}' created successfully", created.definition.name),
            Some(created),
        ),
        Err(e) => MetaToolResult::error(&format!("Failed to create agent: {e}")),
    }
}

/// Update an existing dynamic agent definition.
pub fn agent_update(
    store: &dyn DynamicAgentStoreBackend,
    params: AgentUpdateParams,
) -> MetaToolResult {
    let Some(existing) = store.get(&params.id) else {
        return MetaToolResult::error(&format!("Agent '{}' not found", params.id));
    };

    let mut updated = existing;
    if let Some(desc) = params.description {
        updated.definition.description = desc;
    }
    if let Some(mode) = params.mode {
        updated.definition.mode = mode;
    }
    if let Some(tools) = params.allowed_tools {
        updated.definition.allowed_tools = tools;
    }
    if let Some(prompt) = params.system_prompt {
        updated.definition.system_prompt = prompt;
    }

    match store.update(updated) {
        Ok(result) => MetaToolResult::success(
            &format!("Agent '{}' updated (v{})", result.id, result.version),
            Some(result),
        ),
        Err(e) => MetaToolResult::error(&format!("Failed to update agent: {e}")),
    }
}

/// Deactivate (soft-delete) a dynamic agent.
pub fn agent_deactivate(
    store: &dyn DynamicAgentStoreBackend,
    params: &AgentDeactivateParams,
) -> MetaToolResult {
    match store.deactivate(&params.id, &params.reason) {
        Ok(()) => MetaToolResult::success(
            &format!("Agent '{}' deactivated: {}", params.id, params.reason),
            None,
        ),
        Err(e) => MetaToolResult::error(&format!("Failed to deactivate agent: {e}")),
    }
}

/// Search for agent definitions.
pub fn agent_search(
    store: &dyn DynamicAgentStoreBackend,
    params: &AgentSearchParams,
) -> MetaToolResult {
    let mut results = store.search(&params.query);

    // Apply optional filters
    if let Some(mode) = params.mode {
        results.retain(|a| a.definition.mode == mode);
    }
    if let Some(tier) = params.trust_tier {
        results.retain(|a| a.trust_tier == tier);
    }
    if let Some(status) = params.status {
        results.retain(|a| a.status == status);
    }

    MetaToolResult::search_result(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::dynamic_agent::DynamicAgentStore;

    fn default_creator() -> CreatorPermissionSnapshot {
        CreatorPermissionSnapshot {
            tools_allowed: vec![
                "file_read".to_string(),
                "file_write".to_string(),
                "search_code".to_string(),
            ],
            tools_denied: vec!["shell_exec".to_string()],
            max_iterations: 50,
            max_tool_calls: 100,
            max_tokens: 8192,
            delegation_depth: 3,
        }
    }

    /// T-MA-R4-03: `agent_create` passes three-stage validation.
    #[test]
    fn test_agent_create_valid() {
        let store = DynamicAgentStore::new();
        let result = agent_create(
            &store,
            AgentCreateParams {
                name: "my-helper".to_string(),
                description: "A helpful assistant".to_string(),
                mode: AgentMode::General,
                capabilities: vec!["code_review".to_string()],
                allowed_tools: vec!["file_read".to_string()],
                denied_tools: vec![],
                system_prompt: "Help the user.".to_string(),
                context_sharing: ContextStrategy::None,
            },
            "parent-agent",
            &default_creator(),
        );

        assert!(result.success);
        assert!(result.agent.is_some());
        let agent = result.agent.unwrap();
        assert_eq!(agent.definition.name, "my-helper");
        assert_eq!(agent.trust_tier, TrustTier::Dynamic);
    }

    /// T-MA-R4-04: `agent_create` rejects permission escalation.
    #[test]
    fn test_agent_create_permission_escalation() {
        let store = DynamicAgentStore::new();
        let result = agent_create(
            &store,
            AgentCreateParams {
                name: "escalator".to_string(),
                description: "Tries to escalate".to_string(),
                mode: AgentMode::Build,
                capabilities: vec![],
                allowed_tools: vec!["shell_exec".to_string()], // not in creator's allowed
                denied_tools: vec![],
                system_prompt: String::new(),
                context_sharing: ContextStrategy::None,
            },
            "parent-agent",
            &default_creator(),
        );

        assert!(!result.success);
        assert!(result.message.contains("Failed"));
    }

    /// T-MA-R4-05: `agent_deactivate` soft-deletes with reason.
    #[test]
    fn test_agent_deactivate_with_reason() {
        let store = DynamicAgentStore::new();

        // First create an agent
        let create_result = agent_create(
            &store,
            AgentCreateParams {
                name: "temp-agent".to_string(),
                description: "Temporary agent".to_string(),
                mode: AgentMode::General,
                capabilities: vec![],
                allowed_tools: vec!["file_read".to_string()],
                denied_tools: vec![],
                system_prompt: String::new(),
                context_sharing: ContextStrategy::None,
            },
            "parent-agent",
            &default_creator(),
        );
        assert!(create_result.success);
        let agent_id = create_result.agent.unwrap().id;

        // Deactivate
        let deact_result = agent_deactivate(
            &store,
            &AgentDeactivateParams {
                id: agent_id.clone(),
                reason: "no longer needed".to_string(),
            },
        );
        assert!(deact_result.success);

        // Verify deactivated
        let agent = store.get(&agent_id).unwrap();
        assert_eq!(agent.status, AgentStatus::Deactivated);
        assert_eq!(
            agent.deactivation_reason.as_deref(),
            Some("no longer needed")
        );
    }

    /// T-MA-R4-06: `agent_search` filters by `mode/trust_tier/status`.
    #[test]
    fn test_agent_search_filters() {
        let store = DynamicAgentStore::new();

        // Create agents with different modes
        agent_create(
            &store,
            AgentCreateParams {
                name: "planner".to_string(),
                description: "A planning agent".to_string(),
                mode: AgentMode::Plan,
                capabilities: vec![],
                allowed_tools: vec!["file_read".to_string()],
                denied_tools: vec![],
                system_prompt: String::new(),
                context_sharing: ContextStrategy::None,
            },
            "parent",
            &default_creator(),
        );

        agent_create(
            &store,
            AgentCreateParams {
                name: "builder".to_string(),
                description: "A building agent".to_string(),
                mode: AgentMode::Build,
                capabilities: vec![],
                allowed_tools: vec!["file_read".to_string()],
                denied_tools: vec![],
                system_prompt: String::new(),
                context_sharing: ContextStrategy::None,
            },
            "parent",
            &default_creator(),
        );

        // Search all
        let all = agent_search(
            &store,
            &AgentSearchParams {
                query: "agent".to_string(),
                mode: None,
                trust_tier: None,
                status: None,
            },
        );
        assert_eq!(all.agents.as_ref().unwrap().len(), 2);

        // Filter by mode
        let plan_only = agent_search(
            &store,
            &AgentSearchParams {
                query: "agent".to_string(),
                mode: Some(AgentMode::Plan),
                trust_tier: None,
                status: None,
            },
        );
        assert_eq!(plan_only.agents.as_ref().unwrap().len(), 1);
        assert_eq!(
            plan_only.agents.as_ref().unwrap()[0].definition.name,
            "planner"
        );
    }

    /// `agent_update` modifies and increments version.
    #[test]
    fn test_agent_update() {
        let store = DynamicAgentStore::new();

        let create_result = agent_create(
            &store,
            AgentCreateParams {
                name: "updatable".to_string(),
                description: "Original description".to_string(),
                mode: AgentMode::General,
                capabilities: vec![],
                allowed_tools: vec!["file_read".to_string()],
                denied_tools: vec![],
                system_prompt: String::new(),
                context_sharing: ContextStrategy::None,
            },
            "parent",
            &default_creator(),
        );
        assert!(create_result.success);
        let agent_id = create_result.agent.unwrap().id;

        let update_result = agent_update(
            &store,
            AgentUpdateParams {
                id: agent_id,
                description: Some("Updated description".to_string()),
                mode: Some(AgentMode::Plan),
                allowed_tools: None,
                system_prompt: None,
            },
        );
        assert!(update_result.success);
        let updated = update_result.agent.unwrap();
        assert_eq!(updated.definition.description, "Updated description");
        assert_eq!(updated.definition.mode, AgentMode::Plan);
        assert_eq!(updated.version, 2);
    }
}
