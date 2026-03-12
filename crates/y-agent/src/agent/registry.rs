//! Agent registry: unified management of agent definitions.
//!
//! Design reference: multi-agent-design.md §Agent Registry
//!
//! The registry manages three types of agent definitions:
//! - `BuiltIn`: shipped with the framework (tool-engineer, agent-architect)
//! - `UserDefined`: loaded from TOML configuration files
//! - `Dynamic`: created at runtime by other agents

use std::collections::HashMap;

use crate::agent::definition::{AgentDefinition, AgentMode};
use crate::agent::error::MultiAgentError;
use crate::agent::trust::TrustTier;

/// A search result tagged with the agent's source trust tier.
#[derive(Debug)]
pub struct TieredSearchResult<'a> {
    pub definition: &'a AgentDefinition,
    pub source_tier: TrustTier,
}

/// Multi-criterion search filter for [`AgentRegistry::search_advanced`].
///
/// All provided fields are combined with AND logic; `None` means no filter on that dimension.
#[derive(Debug, Default)]
pub struct SearchCriteria {
    /// Partial match on name or description.
    pub name_query: Option<String>,
    /// Exact mode match.
    pub mode: Option<AgentMode>,
    /// Exact trust tier match.
    pub trust_tier: Option<TrustTier>,
    /// All listed capabilities must be present (partial match per tag).
    pub capabilities: Option<Vec<String>>,
}

/// Unified registry for all agent definitions.
///
/// Separates the concern of *registering definitions* from *managing runtime instances*
/// (which is the responsibility of `AgentPool`).
#[derive(Debug)]
pub struct AgentRegistry {
    definitions: HashMap<String, AgentDefinition>,
}

impl AgentRegistry {
    /// Create a new registry with built-in agent definitions pre-registered.
    pub fn new() -> Self {
        let mut registry = Self {
            definitions: HashMap::new(),
        };
        registry.register_builtins();
        registry
    }

    /// Create an empty registry (no built-ins). Useful for testing.
    pub fn empty() -> Self {
        Self {
            definitions: HashMap::new(),
        }
    }

    /// Register a new agent definition.
    ///
    /// Returns an error if an agent with the same ID is already registered.
    pub fn register(&mut self, definition: AgentDefinition) -> Result<(), MultiAgentError> {
        definition.validate()?;

        if self.definitions.contains_key(&definition.id) {
            return Err(MultiAgentError::InvalidDefinition {
                message: format!("agent '{}' is already registered", definition.id),
            });
        }

        self.definitions.insert(definition.id.clone(), definition);
        Ok(())
    }

    /// Get an agent definition by ID.
    pub fn get(&self, id: &str) -> Option<&AgentDefinition> {
        self.definitions.get(id)
    }

    /// List all registered definitions.
    pub fn list(&self) -> Vec<&AgentDefinition> {
        self.definitions.values().collect()
    }

    /// Search definitions by name, description, or capabilities.
    pub fn search(&self, query: &str) -> Vec<&AgentDefinition> {
        let query_lower = query.to_lowercase();
        self.definitions
            .values()
            .filter(|def| {
                def.name.to_lowercase().contains(&query_lower)
                    || def.description.to_lowercase().contains(&query_lower)
                    || def
                        .capabilities
                        .iter()
                        .any(|c| c.to_lowercase().contains(&query_lower))
            })
            .collect()
    }

    /// Search definitions filtered by agent mode.
    pub fn search_by_mode(&self, mode: crate::agent::definition::AgentMode) -> Vec<&AgentDefinition> {
        self.definitions
            .values()
            .filter(|def| def.mode == mode)
            .collect()
    }

    /// Search definitions with tiered results (tagged by source tier).
    ///
    /// Results are returned as `TieredSearchResult` entries, each tagged
    /// with the agent's `TrustTier` for downstream display/filtering.
    pub fn search_tiered(&self, query: &str) -> Vec<TieredSearchResult<'_>> {
        let query_lower = query.to_lowercase();
        self.definitions
            .values()
            .filter(|def| {
                def.name.to_lowercase().contains(&query_lower)
                    || def.description.to_lowercase().contains(&query_lower)
                    || def
                        .capabilities
                        .iter()
                        .any(|c| c.to_lowercase().contains(&query_lower))
            })
            .map(|def| TieredSearchResult {
                definition: def,
                source_tier: def.trust_tier,
            })
            .collect()
    }

    /// Advanced search with multiple filter criteria.
    ///
    /// All provided criteria are combined with AND logic. `None` means "don't filter on this".
    pub fn search_advanced(&self, criteria: &SearchCriteria) -> Vec<&AgentDefinition> {
        self.definitions
            .values()
            .filter(|def| {
                if let Some(ref q) = criteria.name_query {
                    let q_lower = q.to_lowercase();
                    if !def.name.to_lowercase().contains(&q_lower)
                        && !def.description.to_lowercase().contains(&q_lower)
                    {
                        return false;
                    }
                }
                if let Some(mode) = criteria.mode {
                    if def.mode != mode {
                        return false;
                    }
                }
                if let Some(tier) = criteria.trust_tier {
                    if def.trust_tier != tier {
                        return false;
                    }
                }
                if let Some(ref tags) = criteria.capabilities {
                    let has_all = tags.iter().all(|tag| {
                        def.capabilities
                            .iter()
                            .any(|c| c.to_lowercase().contains(&tag.to_lowercase()))
                    });
                    if !has_all {
                        return false;
                    }
                }
                true
            })
            .collect()
    }

    /// Register a built-in agent definition.
    ///
    /// Asserts the definition's `trust_tier` is `BuiltIn`.
    pub fn register_builtin(
        &mut self,
        definition: AgentDefinition,
    ) -> Result<(), MultiAgentError> {
        if definition.trust_tier != TrustTier::BuiltIn {
            return Err(MultiAgentError::InvalidDefinition {
                message: format!(
                    "register_builtin requires trust_tier=BuiltIn, got {:?}",
                    definition.trust_tier
                ),
            });
        }
        self.register(definition)
    }

    /// Register a user-defined agent definition.
    ///
    /// Asserts the definition's `trust_tier` is `UserDefined`.
    pub fn register_user_defined(
        &mut self,
        definition: AgentDefinition,
    ) -> Result<(), MultiAgentError> {
        if definition.trust_tier != TrustTier::UserDefined {
            return Err(MultiAgentError::InvalidDefinition {
                message: format!(
                    "register_user_defined requires trust_tier=UserDefined, got {:?}",
                    definition.trust_tier
                ),
            });
        }
        self.register(definition)
    }

    /// Register a dynamic agent definition.
    ///
    /// Asserts the definition's `trust_tier` is `Dynamic`.
    pub fn register_dynamic(
        &mut self,
        definition: AgentDefinition,
    ) -> Result<(), MultiAgentError> {
        if definition.trust_tier != TrustTier::Dynamic {
            return Err(MultiAgentError::InvalidDefinition {
                message: format!(
                    "register_dynamic requires trust_tier=Dynamic, got {:?}",
                    definition.trust_tier
                ),
            });
        }
        self.register(definition)
    }

    /// Unregister a definition by ID.
    ///
    /// Returns an error if the agent is a built-in (`BuiltIn` tier cannot be removed).
    ///
    /// # Panics
    ///
    /// Panics if the internal state is inconsistent (key present in `get` but missing in `remove`).
    /// This should never happen in practice.
    pub fn unregister(&mut self, id: &str) -> Result<AgentDefinition, MultiAgentError> {
        match self.definitions.get(id) {
            Some(def) if def.trust_tier == TrustTier::BuiltIn => {
                Err(MultiAgentError::InvalidDefinition {
                    message: format!("cannot unregister built-in agent '{id}'"),
                })
            }
            Some(_) => {
                // Safe: we just confirmed the key exists via `get`.
                Ok(self.definitions.remove(id).expect("key confirmed present"))
            }
            None => Err(MultiAgentError::NotFound { id: id.to_string() }),
        }
    }

    /// List definitions filtered by trust tier.
    pub fn list_by_tier(&self, tier: TrustTier) -> Vec<&AgentDefinition> {
        self.definitions
            .values()
            .filter(|def| def.trust_tier == tier)
            .collect()
    }

    /// Total number of registered definitions.
    pub fn count(&self) -> usize {
        self.definitions.len()
    }

    /// Register the built-in agent definitions from embedded TOML files.
    fn register_builtins(&mut self) {
        for (name, toml_str) in Self::builtin_toml_sources() {
            let def = AgentDefinition::from_toml(toml_str)
                .unwrap_or_else(|e| panic!("built-in agent '{name}' should parse: {e}"));
            self.definitions.insert(def.id.clone(), def);
        }
    }

    /// Returns (name, TOML content) pairs for all built-in agents.
    ///
    /// TOML files are embedded at compile time via `include_str!`.
    fn builtin_toml_sources() -> Vec<(&'static str, &'static str)> {
        vec![
            (
                "compaction-summarizer",
                include_str!("../../../../config/agents/compaction-summarizer.toml"),
            ),
            (
                "context-summarizer",
                include_str!("../../../../config/agents/context-summarizer.toml"),
            ),
            (
                "title-generator",
                include_str!("../../../../config/agents/title-generator.toml"),
            ),
            (
                "task-intent-analyzer",
                include_str!("../../../../config/agents/task-intent-analyzer.toml"),
            ),
            (
                "pattern-extractor",
                include_str!("../../../../config/agents/pattern-extractor.toml"),
            ),
            (
                "capability-assessor",
                include_str!("../../../../config/agents/capability-assessor.toml"),
            ),
            (
                "tool-engineer",
                include_str!("../../../../config/agents/tool-engineer.toml"),
            ),
            (
                "agent-architect",
                include_str!("../../../../config/agents/agent-architect.toml"),
            ),
            (
                "skill-ingestion",
                include_str!("../../../../config/agents/skill-ingestion.toml"),
            ),
        ]
    }
}

impl Default for AgentRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::definition::{AgentMode, ContextStrategy};

    fn user_definition(id: &str, name: &str) -> AgentDefinition {
        AgentDefinition {
            id: id.to_string(),
            name: name.to_string(),
            description: "A test user-defined agent".to_string(),
            mode: AgentMode::General,
            trust_tier: TrustTier::UserDefined,
            capabilities: vec!["code_review".to_string()],
            allowed_tools: vec!["file_read".to_string()],
            denied_tools: vec![],
            system_prompt: "You are a test agent.".to_string(),
            skills: vec![],
            preferred_models: vec![],
            fallback_models: vec![],
            temperature: None,
            top_p: None,
            max_iterations: 20,
            max_tool_calls: 50,
            timeout_secs: 300,
            context_sharing: ContextStrategy::None,
            max_context_tokens: 4096,
        }
    }

    /// T-MA-R2-01: Registry registers/queries all three definition types.
    #[test]
    fn test_registry_three_types() {
        let mut registry = AgentRegistry::new();

        // Built-ins are already registered
        assert!(registry.get("tool-engineer").is_some());
        assert!(registry.get("agent-architect").is_some());

        // Register a user-defined agent
        let user_def = user_definition("reviewer-1", "Code Reviewer");
        registry.register(user_def).unwrap();
        assert!(registry.get("reviewer-1").is_some());

        // Register a dynamic agent
        let dynamic_def = AgentDefinition {
            id: "dyn-helper".to_string(),
            name: "Dynamic Helper".to_string(),
            description: "A dynamically created agent".to_string(),
            mode: AgentMode::Explore,
            trust_tier: TrustTier::Dynamic,
            capabilities: vec![],
            allowed_tools: vec!["file_read".to_string()],
            denied_tools: vec![],
            system_prompt: String::new(),
            skills: vec![],
            preferred_models: vec![],
            fallback_models: vec![],
            temperature: None,
            top_p: None,
            max_iterations: 10,
            max_tool_calls: 20,
            timeout_secs: 120,
            context_sharing: ContextStrategy::None,
            max_context_tokens: 2048,
        };
        registry.register(dynamic_def).unwrap();
        assert!(registry.get("dyn-helper").is_some());

        assert_eq!(registry.count(), 11); // 9 built-in + 1 user + 1 dynamic
    }

    /// T-MA-R2-02: Registry ships built-in tool-engineer and agent-architect.
    #[test]
    fn test_registry_builtins() {
        let registry = AgentRegistry::new();

        let te = registry.get("tool-engineer").unwrap();
        assert_eq!(te.name, "tool-engineer");
        assert_eq!(te.trust_tier, TrustTier::BuiltIn);
        assert_eq!(te.mode, AgentMode::Build);

        let aa = registry.get("agent-architect").unwrap();
        assert_eq!(aa.name, "agent-architect");
        assert_eq!(aa.trust_tier, TrustTier::BuiltIn);
        assert_eq!(aa.mode, AgentMode::Plan);
    }

    /// T-MA-R2-03: Registry `search()` filters by name/capabilities.
    #[test]
    fn test_registry_search() {
        let mut registry = AgentRegistry::new();
        registry
            .register(user_definition("reviewer-1", "Code Reviewer"))
            .unwrap();

        // Search by name
        let results = registry.search("engineer");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "tool-engineer");

        // Search by capability
        let results = registry.search("code_review");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "reviewer-1");

        // Search by description
        let results = registry.search("capability gaps");
        assert!(!results.is_empty());
        assert!(results.iter().any(|d| d.id == "agent-architect"));

        // Case-insensitive
        let results = registry.search("TOOL");
        assert!(!results.is_empty());
    }

    /// Duplicate registration is rejected.
    #[test]
    fn test_registry_duplicate_rejected() {
        let mut registry = AgentRegistry::new();
        registry
            .register(user_definition("agent-1", "Agent 1"))
            .unwrap();

        let result = registry.register(user_definition("agent-1", "Agent 1 Copy"));
        assert!(result.is_err());
    }

    /// Cannot unregister built-in agents.
    #[test]
    fn test_registry_cannot_unregister_builtin() {
        let mut registry = AgentRegistry::new();
        let result = registry.unregister("tool-engineer");
        assert!(result.is_err());
    }

    /// Unregister user-defined agents.
    #[test]
    fn test_registry_unregister_user() {
        let mut registry = AgentRegistry::new();
        registry
            .register(user_definition("temp", "Temp Agent"))
            .unwrap();
        assert!(registry.get("temp").is_some());

        let removed = registry.unregister("temp").unwrap();
        assert_eq!(removed.id, "temp");
        assert!(registry.get("temp").is_none());
    }

    /// List by tier filters correctly.
    #[test]
    fn test_registry_list_by_tier() {
        let mut registry = AgentRegistry::new();
        registry
            .register(user_definition("user-1", "User Agent"))
            .unwrap();

        let builtins = registry.list_by_tier(TrustTier::BuiltIn);
        assert_eq!(builtins.len(), 9);

        let user_defs = registry.list_by_tier(TrustTier::UserDefined);
        assert_eq!(user_defs.len(), 1);

        let dynamics = registry.list_by_tier(TrustTier::Dynamic);
        assert_eq!(dynamics.len(), 0);
    }

    /// T-MA-P3-03: Search by mode returns only agents with that mode.
    #[test]
    fn test_registry_search_by_mode() {
        let mut registry = AgentRegistry::new();
        registry
            .register(user_definition("user-1", "User Agent"))
            .unwrap();

        // Built-in agent-architect is plan mode, tool-engineer is build mode
        let plan_agents = registry.search_by_mode(AgentMode::Plan);
        assert!(plan_agents.iter().any(|a| a.id == "agent-architect"));

        let build_agents = registry.search_by_mode(AgentMode::Build);
        assert!(build_agents.iter().any(|a| a.id == "tool-engineer"));

        // User agent is General mode
        let general_agents = registry.search_by_mode(AgentMode::General);
        assert!(general_agents.iter().any(|a| a.id == "user-1"));
    }

    /// T-MA-P3-01: Typed registration enforces correct trust tier.
    #[test]
    fn test_registry_typed_registration() {
        let mut registry = AgentRegistry::empty();

        // register_builtin rejects non-BuiltIn
        let user_def = user_definition("bad", "Bad");
        assert!(registry.register_builtin(user_def).is_err());

        // register_user_defined accepts UserDefined
        let user_def = user_definition("good-user", "Good User");
        registry.register_user_defined(user_def).unwrap();

        // register_dynamic rejects non-Dynamic
        let user_def = user_definition("bad2", "Bad2");
        assert!(registry.register_dynamic(user_def).is_err());

        // register_dynamic accepts Dynamic
        let dyn_def = AgentDefinition {
            id: "dyn-test".to_string(),
            name: "Dynamic Test".to_string(),
            description: "A dynamic agent".to_string(),
            mode: AgentMode::Explore,
            trust_tier: TrustTier::Dynamic,
            capabilities: vec![],
            allowed_tools: vec![],
            denied_tools: vec![],
            system_prompt: String::new(),
            skills: vec![],
            preferred_models: vec![],
            fallback_models: vec![],
            temperature: None,
            top_p: None,
            max_iterations: 10,
            max_tool_calls: 20,
            timeout_secs: 120,
            context_sharing: ContextStrategy::None,
            max_context_tokens: 2048,
        };
        registry.register_dynamic(dyn_def).unwrap();
        assert_eq!(registry.count(), 2);
    }

    /// T-MA-P3-04: Search by trust tier returns only matching agents.
    #[test]
    fn test_registry_search_by_trust_tier() {
        let mut registry = AgentRegistry::new();
        registry
            .register(user_definition("user-1", "User Agent"))
            .unwrap();

        let builtins = registry.list_by_tier(TrustTier::BuiltIn);
        assert_eq!(builtins.len(), 9);
        assert!(builtins.iter().all(|d| d.trust_tier == TrustTier::BuiltIn));

        let user_defs = registry.list_by_tier(TrustTier::UserDefined);
        assert_eq!(user_defs.len(), 1);
        assert!(user_defs
            .iter()
            .all(|d| d.trust_tier == TrustTier::UserDefined));

        let dynamics = registry.list_by_tier(TrustTier::Dynamic);
        assert!(dynamics.is_empty());
    }

    /// T-MA-P3-05: Tiered results tag each result with its source tier.
    #[test]
    fn test_registry_tiered_results() {
        let mut registry = AgentRegistry::new();
        registry
            .register(user_definition("user-1", "User Agent"))
            .unwrap();

        // Search for something that matches across tiers
        let results = registry.search_tiered("agent");
        assert!(results.len() >= 2); // at least user-1 and agent-architect

        // Verify each result's source_tier matches its definition
        for r in &results {
            assert_eq!(r.source_tier, r.definition.trust_tier);
        }

        // Verify built-in and user-defined tiers are both present
        assert!(results.iter().any(|r| r.source_tier == TrustTier::BuiltIn));
        assert!(results
            .iter()
            .any(|r| r.source_tier == TrustTier::UserDefined));
    }

    /// Advanced search with multiple criteria.
    #[test]
    fn test_registry_search_advanced() {
        let mut registry = AgentRegistry::new();
        registry
            .register(user_definition("user-1", "User Agent"))
            .unwrap();

        // Search by mode only
        let results = registry.search_advanced(&SearchCriteria {
            mode: Some(AgentMode::Plan),
            ..Default::default()
        });
        assert!(results.iter().any(|d| d.id == "agent-architect"));
        assert!(!results.iter().any(|d| d.id == "tool-engineer"));

        // Search by tier + mode
        let results = registry.search_advanced(&SearchCriteria {
            mode: Some(AgentMode::General),
            trust_tier: Some(TrustTier::UserDefined),
            ..Default::default()
        });
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "user-1");

        let results = registry.search_advanced(&SearchCriteria {
            name_query: Some("engineer".to_string()),
            trust_tier: Some(TrustTier::UserDefined),
            ..Default::default()
        });
        assert!(results.is_empty());
    }

    /// T-MA-P4-10: Compaction summarizer parses from TOML with correct config.
    #[test]
    fn test_builtin_agent_compaction_summarizer() {
        let registry = AgentRegistry::new();
        let def = registry.get("compaction-summarizer").unwrap();
        assert_eq!(def.mode, AgentMode::Explore);
        assert_eq!(def.trust_tier, TrustTier::BuiltIn);
        assert!(def.allowed_tools.is_empty());
        assert!(def.system_prompt.contains("Compaction Summarizer"));
    }

    /// T-MA-P4-11: Tool engineer parses from TOML with correct config.
    #[test]
    fn test_builtin_agent_tool_engineer() {
        let registry = AgentRegistry::new();
        let def = registry.get("tool-engineer").unwrap();
        assert_eq!(def.mode, AgentMode::Build);
        assert_eq!(def.trust_tier, TrustTier::BuiltIn);
        assert!(def.allowed_tools.contains(&"file_read".to_string()));
        assert!(def.allowed_tools.contains(&"shell_exec".to_string()));
    }

    /// T-MA-P4-12: Agent architect parses from TOML with `shell_exec` denied.
    #[test]
    fn test_builtin_agent_agent_architect() {
        let registry = AgentRegistry::new();
        let def = registry.get("agent-architect").unwrap();
        assert_eq!(def.mode, AgentMode::Plan);
        assert!(def.denied_tools.contains(&"shell_exec".to_string()));
    }

    /// T-MA-P4-13: Registry loads all 8 built-in agents at startup.
    #[test]
    fn test_registry_loads_all_builtin_agents() {
        let registry = AgentRegistry::new();
        let expected_ids = [
            "compaction-summarizer",
            "context-summarizer",
            "title-generator",
            "task-intent-analyzer",
            "pattern-extractor",
            "capability-assessor",
            "tool-engineer",
            "agent-architect",
            "skill-ingestion",
        ];

        for id in expected_ids {
            let def = registry.get(id);
            assert!(def.is_some(), "built-in agent '{id}' should be registered");
            let def = def.unwrap();
            assert_eq!(def.trust_tier, TrustTier::BuiltIn);
            assert!(!def.description.is_empty());
            assert!(!def.system_prompt.is_empty());
        }

        assert_eq!(registry.list_by_tier(TrustTier::BuiltIn).len(), 9);
    }
}
