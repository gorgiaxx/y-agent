//! Agent registry: unified management of agent definitions.
//!
//! Design reference: multi-agent-design.md §Agent Registry
//!
//! The registry manages three types of agent definitions:
//! - `BuiltIn`: shipped with the framework (tool-engineer, agent-architect)
//! - `UserDefined`: loaded from TOML configuration files
//! - `Dynamic`: created at runtime by other agents
//!
//! **Override support**: User-defined agents with the same `id` as a built-in
//! agent will replace the built-in definition. The original built-in can be
//! restored via [`AgentRegistry::reset_builtin`].

use std::collections::HashMap;
use std::path::Path;

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
    /// Original built-in definitions, preserved for reset support.
    builtin_originals: HashMap<String, AgentDefinition>,
    /// Expanded TOML source content for each built-in agent (keyed by agent ID).
    /// Used to detect unmodified seed copies in the user agents directory.
    builtin_sources: HashMap<String, String>,
    /// Template variables for TOML expansion (e.g. `{{YAGENT_CONFIG_PATH}}`).
    template_vars: Vec<(String, String)>,
    /// Path to the user-defined agents directory, retained for hot-reload.
    agents_dir: Option<std::path::PathBuf>,
}

impl AgentRegistry {
    /// Create a new registry with built-in agent definitions pre-registered.
    ///
    /// If `user_agents_dir` is provided and exists, user-defined agent TOML
    /// files in that directory are loaded and may override built-in agents.
    pub fn new_with_user_agents(user_agents_dir: Option<&Path>) -> Self {
        let config_dir = user_agents_dir.and_then(|p| p.parent());
        let mut registry = Self::new_with_config_dir(config_dir);

        registry.agents_dir = user_agents_dir.map(std::path::Path::to_path_buf);

        if let Some(dir) = user_agents_dir {
            if let Err(errors) = registry.load_user_agents(dir) {
                for (file, err) in errors {
                    tracing::warn!("failed to load user agent from {file}: {err}");
                }
            }
        }

        registry
    }

    /// Create a new registry with built-in agent definitions pre-registered.
    pub fn new() -> Self {
        Self::new_with_config_dir(None)
    }

    /// Create a new registry, establishing the configuration directory used for template expansion
    /// (e.g., matching `{{YAGENT_CONFIG_PATH}}`).
    pub fn new_with_config_dir(config_dir: Option<&Path>) -> Self {
        let mut registry = Self {
            definitions: HashMap::new(),
            builtin_originals: HashMap::new(),
            builtin_sources: HashMap::new(),
            template_vars: Vec::new(),
            agents_dir: None,
        };

        let path_str =
            config_dir.map_or_else(|| ".".to_string(), |p| p.to_string_lossy().into_owned());

        registry
            .template_vars
            .push(("{{YAGENT_CONFIG_PATH}}".to_string(), path_str));

        for (key, val) in y_core::template::RuntimeTemplateVars::static_vars() {
            registry.template_vars.push((key, val));
        }

        registry.register_builtins();
        registry
    }

    /// Create an empty registry (no built-ins). Useful for testing.
    pub fn empty() -> Self {
        Self {
            definitions: HashMap::new(),
            builtin_originals: HashMap::new(),
            builtin_sources: HashMap::new(),
            template_vars: Vec::new(),
            agents_dir: None,
        }
    }

    /// Path to the user-defined agents directory, if configured.
    pub fn agents_dir(&self) -> Option<&Path> {
        self.agents_dir.as_deref()
    }

    /// Add or update a template variable used for expanding agent TOML definitions.
    ///
    /// If a variable with the same `key` already exists, its value is replaced.
    pub fn add_template_var(&mut self, key: String, value: String) {
        if let Some(entry) = self.template_vars.iter_mut().find(|(k, _)| *k == key) {
            entry.1 = value;
        } else {
            self.template_vars.push((key, value));
        }
    }

    /// Expand registered template variables in a TOML string.
    pub fn expand_templates(&self, content: &str) -> String {
        let mut processed = content.to_string();
        for (key, val) in &self.template_vars {
            // Escape backslashes for TOML basic strings (needed for Windows paths like C:\Users)
            let escaped_val = val.replace('\\', "\\\\");
            processed = processed.replace(key, &escaped_val);
        }
        processed
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
    pub fn search_by_mode(
        &self,
        mode: crate::agent::definition::AgentMode,
    ) -> Vec<&AgentDefinition> {
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
    pub fn register_builtin(&mut self, definition: AgentDefinition) -> Result<(), MultiAgentError> {
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
    pub fn register_dynamic(&mut self, definition: AgentDefinition) -> Result<(), MultiAgentError> {
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

    /// Register or override an existing agent definition.
    ///
    /// Unlike [`Self::register`], this replaces any existing definition with the
    /// same ID. This is the primary mechanism for user-defined overrides
    /// of built-in agents.
    pub fn register_or_override(
        &mut self,
        definition: AgentDefinition,
    ) -> Result<(), MultiAgentError> {
        definition.validate()?;
        self.definitions.insert(definition.id.clone(), definition);
        Ok(())
    }

    /// Load user-defined agent definitions from a directory.
    ///
    /// Scans `dir` for `*.toml` files, parses each as an `AgentDefinition`,
    /// and calls [`Self::register_or_override`] to register (or override built-ins).
    ///
    /// Returns `Ok(())` on success, or a list of `(filename, error)` pairs
    /// for any files that failed to parse. Successfully parsed files are
    /// still registered even if some files fail.
    pub fn load_user_agents(&mut self, dir: &Path) -> Result<(), Vec<(String, String)>> {
        if !dir.is_dir() {
            return Ok(());
        }

        let mut errors = Vec::new();

        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) => {
                errors.push((dir.display().to_string(), e.to_string()));
                return Err(errors);
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }

            let filename = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();

            match std::fs::read_to_string(&path) {
                Ok(content) => {
                    let expanded = self.expand_templates(&content);
                    match AgentDefinition::from_toml(&expanded) {
                        Ok(mut def) => {
                            // Check whether this file is an unmodified copy of a
                            // built-in agent. Compare the expanded TOML content
                            // against the stored built-in source; if identical,
                            // skip the override to keep the BuiltIn trust tier.
                            let is_unmodified_builtin = self
                                .builtin_sources
                                .get(&def.id)
                                .is_some_and(|src| src == &expanded);
                            if is_unmodified_builtin {
                                continue;
                            }
                            def.trust_tier = TrustTier::UserDefined;
                            if let Err(e) = self.register_or_override(def) {
                                errors.push((filename, e.to_string()));
                            }
                        }
                        Err(e) => {
                            errors.push((filename, e.to_string()));
                        }
                    }
                }
                Err(e) => {
                    errors.push((filename, e.to_string()));
                }
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Hot-reload user-defined agents from the stored agents directory.
    ///
    /// Clears all `UserDefined` and `Dynamic` agents, then re-reads all
    /// `*.toml` files from the directory provided at construction time.
    /// Built-in agents (and their originals) are preserved.
    ///
    /// Returns `(loaded, errored)` counts.
    pub fn reload_user_agents_from_dir(&mut self) -> (usize, usize) {
        let Some(ref d) = self.agents_dir else {
            tracing::warn!("no agents directory configured; skipping agent reload");
            return (0, 0);
        };
        let dir = d.clone();

        // Remove all non-built-in definitions.
        self.definitions
            .retain(|_, def| def.trust_tier == TrustTier::BuiltIn);

        // Restore built-in originals (in case any were overridden by user TOML).
        for (id, original) in &self.builtin_originals {
            self.definitions
                .entry(id.clone())
                .or_insert_with(|| original.clone());
        }

        // Re-scan the directory.
        match self.load_user_agents(&dir) {
            Ok(()) => {
                let user_count = self
                    .definitions
                    .values()
                    .filter(|d| d.trust_tier == TrustTier::UserDefined)
                    .count();
                (user_count, 0)
            }
            Err(errors) => {
                let err_count = errors.len();
                for (file, err) in &errors {
                    tracing::warn!("failed to load user agent from {file}: {err}");
                }
                let user_count = self
                    .definitions
                    .values()
                    .filter(|d| d.trust_tier == TrustTier::UserDefined)
                    .count();
                (user_count, err_count)
            }
        }
    }

    /// Parse and register a single agent from raw TOML content.
    ///
    /// The agent is always registered as `UserDefined` tier regardless of
    /// what the TOML specifies. Uses `register_or_override` so that
    /// re-registration of the same ID replaces the previous definition.
    ///
    /// Returns the registered agent's ID on success.
    pub fn register_agent_from_toml(&mut self, toml_content: &str) -> Result<String, String> {
        let expanded = self.expand_templates(toml_content);
        let mut def = AgentDefinition::from_toml(&expanded)
            .map_err(|e| format!("failed to parse agent TOML: {e}"))?;
        def.trust_tier = TrustTier::UserDefined;
        let id = def.id.clone();
        self.register_or_override(def)
            .map_err(|e| format!("failed to register agent: {e}"))?;
        Ok(id)
    }

    /// Check if a built-in agent has been overridden by a user-defined agent.
    pub fn is_overridden(&self, id: &str) -> bool {
        if !self.builtin_originals.contains_key(id) {
            return false;
        }
        // Overridden if the current definition's trust tier differs from BuiltIn.
        self.definitions
            .get(id)
            .is_some_and(|def| def.trust_tier != TrustTier::BuiltIn)
    }

    /// Return the IDs of all built-in agents that have been overridden.
    pub fn list_overridden_ids(&self) -> Vec<&str> {
        self.builtin_originals
            .keys()
            .filter(|id| self.is_overridden(id))
            .map(std::string::String::as_str)
            .collect()
    }

    /// Reset an overridden built-in agent to its original definition.
    ///
    /// Returns an error if the agent was never a built-in.
    pub fn reset_builtin(&mut self, id: &str) -> Result<(), MultiAgentError> {
        match self.builtin_originals.get(id) {
            Some(original) => {
                self.definitions.insert(id.to_string(), original.clone());
                Ok(())
            }
            None => Err(MultiAgentError::InvalidDefinition {
                message: format!("agent '{id}' is not a built-in agent"),
            }),
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
    ///
    /// Also saves a copy of each original definition for reset support.
    fn register_builtins(&mut self) {
        for (name, toml_str) in Self::builtin_toml_sources() {
            let expanded = self.expand_templates(toml_str);
            let def = AgentDefinition::from_toml(&expanded)
                .unwrap_or_else(|e| panic!("built-in agent '{name}' should parse: {e}"));
            self.builtin_originals.insert(def.id.clone(), def.clone());
            self.builtin_sources.insert(def.id.clone(), expanded);
            self.definitions.insert(def.id.clone(), def);
        }
    }

    /// Returns (name, TOML content) pairs for all built-in agents.
    ///
    /// TOML files are embedded at compile time via `include_str!`.
    pub fn builtin_toml_sources() -> Vec<(&'static str, &'static str)> {
        vec![
            (
                "compaction-summarizer",
                include_str!("../../../../config/agents/compaction-summarizer.toml"),
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
            (
                "skill-security-check",
                include_str!("../../../../config/agents/skill-security-check.toml"),
            ),
            (
                "pruning-summarizer",
                include_str!("../../../../config/agents/pruning-summarizer.toml"),
            ),
            (
                "complexity-classifier",
                include_str!("../../../../config/agents/complexity-classifier.toml"),
            ),
            (
                "knowledge-metadata",
                include_str!("../../../../config/agents/knowledge-metadata.toml"),
            ),
            (
                "knowledge-summarizer",
                include_str!("../../../../config/agents/knowledge-summarizer.toml"),
            ),
            (
                "translator",
                include_str!("../../../../config/agents/translator.toml"),
            ),
            (
                "plan-writer",
                include_str!("../../../../config/agents/plan-writer.toml"),
            ),
            (
                "plan-phase-executor",
                include_str!("../../../../config/agents/plan-phase-executor.toml"),
            ),
            (
                "task-decomposer",
                include_str!("../../../../config/agents/task-decomposer.toml"),
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
            icon: None,
            working_directory: None,
            toolcall_enabled: None,
            skills_enabled: None,
            knowledge_enabled: None,
            allowed_tools: vec!["FileRead".to_string()],
            system_prompt: "You are a test agent.".to_string(),
            skills: vec![],
            knowledge_collections: vec![],
            prompt_section_ids: vec![],
            provider_id: None,
            preferred_models: vec![],
            fallback_models: vec![],
            provider_tags: vec![],
            temperature: None,
            top_p: None,
            plan_mode: None,
            thinking_effort: None,
            permission_mode: None,
            max_iterations: 20,
            max_tool_calls: 50,
            timeout_secs: 300,
            context_sharing: ContextStrategy::None,
            max_context_tokens: 4096,
            max_completion_tokens: None,
            user_callable: false,
            mcp_mode: None,
            mcp_servers: vec![],
            prune_tool_history: false,
            auto_update: true,
            response_format: None,
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
            icon: None,
            working_directory: None,
            toolcall_enabled: None,
            skills_enabled: None,
            knowledge_enabled: None,
            allowed_tools: vec!["FileRead".to_string()],
            system_prompt: String::new(),
            skills: vec![],
            knowledge_collections: vec![],
            prompt_section_ids: vec![],
            provider_id: None,
            preferred_models: vec![],
            fallback_models: vec![],
            provider_tags: vec![],
            temperature: None,
            top_p: None,
            plan_mode: None,
            thinking_effort: None,
            permission_mode: None,
            max_iterations: 10,
            max_tool_calls: 20,
            timeout_secs: 120,
            context_sharing: ContextStrategy::None,
            max_context_tokens: 2048,
            max_completion_tokens: None,
            user_callable: false,
            mcp_mode: None,
            mcp_servers: vec![],
            prune_tool_history: false,
            auto_update: true,
            response_format: None,
        };
        registry.register(dynamic_def).unwrap();
        assert!(registry.get("dyn-helper").is_some());

        assert_eq!(registry.count(), 19); // 17 built-in + 1 user + 1 dynamic
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
        assert_eq!(aa.mode, AgentMode::Build);
    }

    /// Template variables in agent TOML are expanded during registration.
    #[test]
    fn test_template_expansion_in_builtins() {
        let registry =
            AgentRegistry::new_with_config_dir(Some(Path::new("/home/user/.config/y-agent")));

        let aa = registry.get("agent-architect").unwrap();
        // The system_prompt should contain the expanded path, not the template variable.
        assert!(
            !aa.system_prompt.contains("{{YAGENT_CONFIG_PATH}}"),
            "template variable should be expanded"
        );
        assert!(
            aa.system_prompt
                .contains("/home/user/.config/y-agent/agents/"),
            "expanded path should appear in system_prompt"
        );
    }

    /// Template expansion with no `config_dir` falls back to ".".
    #[test]
    fn test_template_expansion_fallback() {
        let registry = AgentRegistry::new();

        let aa = registry.get("agent-architect").unwrap();
        assert!(
            !aa.system_prompt.contains("{{YAGENT_CONFIG_PATH}}"),
            "template variable should be expanded even with fallback"
        );
        assert!(
            aa.system_prompt.contains("./agents/"),
            "fallback path '.' should produce './agents/' in system_prompt"
        );
    }

    /// Static runtime vars (OS, ARCH) are expanded in agent TOML at load time.
    #[test]
    fn test_registry_expands_os_arch() {
        let registry = AgentRegistry::new();

        let toml_with_os = r#"
id = "test-os-agent"
name = "Test OS Agent"
description = "Agent that uses {{OS}} and {{ARCH}}"
mode = "explore"
trust_tier = "built_in"
system_prompt = "You run on {{OS}} ({{ARCH}})."
"#;
        let expanded = registry.expand_templates(toml_with_os);

        assert!(
            !expanded.contains("{{OS}}"),
            "{{{{OS}}}} should be expanded"
        );
        assert!(
            !expanded.contains("{{ARCH}}"),
            "{{{{ARCH}}}} should be expanded"
        );
        assert!(
            expanded.contains(std::env::consts::OS),
            "expanded content should contain the actual OS"
        );
        assert!(
            expanded.contains(std::env::consts::ARCH),
            "expanded content should contain the actual ARCH"
        );
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
        let results = registry.search("agent definitions");
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
        assert_eq!(builtins.len(), 17);

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

        // Built-in agent-architect and tool-engineer are both build mode
        let build_agents = registry.search_by_mode(AgentMode::Build);
        assert!(build_agents.iter().any(|a| a.id == "agent-architect"));
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
            icon: None,
            working_directory: None,
            toolcall_enabled: None,
            skills_enabled: None,
            knowledge_enabled: None,
            allowed_tools: vec![],
            system_prompt: String::new(),
            skills: vec![],
            knowledge_collections: vec![],
            prompt_section_ids: vec![],
            provider_id: None,
            preferred_models: vec![],
            fallback_models: vec![],
            provider_tags: vec![],
            temperature: None,
            top_p: None,
            plan_mode: None,
            thinking_effort: None,
            permission_mode: None,
            max_iterations: 10,
            max_tool_calls: 20,
            timeout_secs: 120,
            context_sharing: ContextStrategy::None,
            max_context_tokens: 2048,
            max_completion_tokens: None,
            user_callable: false,
            mcp_mode: None,
            mcp_servers: vec![],
            prune_tool_history: false,
            auto_update: true,
            response_format: None,
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
        assert_eq!(builtins.len(), 17);
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
            mode: Some(AgentMode::Build),
            ..Default::default()
        });
        assert!(results.iter().any(|d| d.id == "agent-architect"));
        assert!(results.iter().any(|d| d.id == "tool-engineer"));

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
        assert!(def.allowed_tools.contains(&"FileRead".to_string()));
        assert!(def.allowed_tools.contains(&"ShellExec".to_string()));
    }

    /// T-MA-P4-12: Agent architect excludes `ShellExec` from its allowlist.
    #[test]
    fn test_builtin_agent_agent_architect() {
        let registry = AgentRegistry::new();
        let def = registry.get("agent-architect").unwrap();
        assert_eq!(def.mode, AgentMode::Build);
        assert!(def.allowed_tools.contains(&"FileWrite".to_string()));
        assert!(!def.allowed_tools.contains(&"ShellExec".to_string()));
    }

    /// T-MA-P4-13: Plan phase executor parses from TOML with build settings.
    #[test]
    fn test_builtin_agent_plan_phase_executor() {
        let registry = AgentRegistry::new();
        let def = registry.get("plan-phase-executor").unwrap();
        assert_eq!(def.mode, AgentMode::Build);
        assert_eq!(def.trust_tier, TrustTier::BuiltIn);
        assert_eq!(def.max_iterations, 60);
        assert!(def.allowed_tools.contains(&"FileWrite".to_string()));
        assert!(def.allowed_tools.contains(&"ToolSearch".to_string()));
        assert!(!def.allowed_tools.contains(&"Task".to_string()));
    }

    /// T-MA-P4-14: Registry loads all 17 built-in agents at startup.
    #[test]
    fn test_registry_loads_all_builtin_agents() {
        let registry = AgentRegistry::new();
        let expected_ids = [
            "compaction-summarizer",
            "title-generator",
            "task-intent-analyzer",
            "pattern-extractor",
            "capability-assessor",
            "tool-engineer",
            "agent-architect",
            "skill-ingestion",
            "skill-security-check",
            "pruning-summarizer",
            "complexity-classifier",
            "knowledge-metadata",
            "knowledge-summarizer",
            "translator",
            "plan-writer",
            "plan-phase-executor",
            "task-decomposer",
        ];

        for id in expected_ids {
            let def = registry.get(id);
            assert!(def.is_some(), "built-in agent '{id}' should be registered");
            let def = def.unwrap();
            assert_eq!(def.trust_tier, TrustTier::BuiltIn);
            assert!(!def.description.is_empty());
            assert!(!def.system_prompt.is_empty());
        }

        assert_eq!(registry.list_by_tier(TrustTier::BuiltIn).len(), 17);
    }

    /// Override a built-in agent with `register_or_override`.
    #[test]
    fn test_register_or_override_replaces_builtin() {
        let mut registry = AgentRegistry::new();
        let original = registry.get("tool-engineer").unwrap();
        assert_eq!(original.trust_tier, TrustTier::BuiltIn);

        let mut override_def = user_definition("tool-engineer", "Custom Tool Engineer");
        override_def.system_prompt = "Custom prompt for tool engineer".to_string();
        registry.register_or_override(override_def).unwrap();

        let updated = registry.get("tool-engineer").unwrap();
        assert_eq!(updated.name, "Custom Tool Engineer");
        assert_eq!(updated.system_prompt, "Custom prompt for tool engineer");
        assert_eq!(updated.trust_tier, TrustTier::UserDefined);
    }

    /// Overriding one built-in does not affect others.
    #[test]
    fn test_register_or_override_preserves_non_overridden() {
        let mut registry = AgentRegistry::new();

        let override_def = user_definition("tool-engineer", "Custom Tool Engineer");
        registry.register_or_override(override_def).unwrap();

        // agent-architect should remain unchanged
        let aa = registry.get("agent-architect").unwrap();
        assert_eq!(aa.trust_tier, TrustTier::BuiltIn);
        assert_eq!(aa.mode, AgentMode::Build);
    }

    /// Reset an overridden built-in to its original definition.
    #[test]
    fn test_reset_builtin_restores_original() {
        let mut registry = AgentRegistry::new();

        let original_prompt = registry.get("tool-engineer").unwrap().system_prompt.clone();

        let mut override_def = user_definition("tool-engineer", "Custom");
        override_def.system_prompt = "Override prompt".to_string();
        registry.register_or_override(override_def).unwrap();
        assert_eq!(
            registry.get("tool-engineer").unwrap().system_prompt,
            "Override prompt"
        );

        registry.reset_builtin("tool-engineer").unwrap();

        let restored = registry.get("tool-engineer").unwrap();
        assert_eq!(restored.trust_tier, TrustTier::BuiltIn);
        assert_eq!(restored.system_prompt, original_prompt);
    }

    /// Reset for a non-built-in agent returns an error.
    #[test]
    fn test_reset_builtin_rejects_non_builtin() {
        let mut registry = AgentRegistry::new();
        let result = registry.reset_builtin("nonexistent-agent");
        assert!(result.is_err());
    }

    /// `is_overridden` and `list_overridden_ids` track overrides correctly.
    #[test]
    fn test_is_overridden_tracking() {
        let mut registry = AgentRegistry::new();
        assert!(!registry.is_overridden("tool-engineer"));
        assert!(registry.list_overridden_ids().is_empty());

        let override_def = user_definition("tool-engineer", "Custom");
        registry.register_or_override(override_def).unwrap();

        assert!(registry.is_overridden("tool-engineer"));
        assert!(registry.list_overridden_ids().contains(&"tool-engineer"));

        // Reset clears the override
        registry.reset_builtin("tool-engineer").unwrap();
        assert!(!registry.is_overridden("tool-engineer"));
    }

    /// Load user agents from a directory.
    #[test]
    fn test_load_user_agents_from_directory() {
        let tmp = std::env::temp_dir().join("y-agent-test-user-agents");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        // Write a valid user agent TOML
        let toml_content = r#"
id = "my-custom-agent"
name = "My Custom Agent"
description = "A custom user agent"
mode = "general"
trust_tier = "built_in"
system_prompt = "You are a custom agent."
"#;
        std::fs::write(tmp.join("my-custom-agent.toml"), toml_content).unwrap();

        let mut registry = AgentRegistry::new();
        let result = registry.load_user_agents(&tmp);
        assert!(result.is_ok());

        let loaded = registry.get("my-custom-agent").unwrap();
        assert_eq!(loaded.name, "My Custom Agent");
        // Trust tier is forced to UserDefined regardless of TOML content
        assert_eq!(loaded.trust_tier, TrustTier::UserDefined);

        // Cleanup
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// User agent with same ID as built-in overrides it.
    #[test]
    fn test_load_user_agents_override_builtin() {
        let tmp = std::env::temp_dir().join("y-agent-test-override-builtin");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let toml_content = r#"
id = "tool-engineer"
name = "My Custom Tool Engineer"
description = "Overridden tool engineer"
mode = "build"
trust_tier = "built_in"
system_prompt = "You are my custom tool engineer."
"#;
        std::fs::write(tmp.join("tool-engineer.toml"), toml_content).unwrap();

        let registry = AgentRegistry::new_with_user_agents(Some(&tmp));

        let def = registry.get("tool-engineer").unwrap();
        assert_eq!(def.name, "My Custom Tool Engineer");
        assert_eq!(def.trust_tier, TrustTier::UserDefined);
        assert!(registry.is_overridden("tool-engineer"));

        // Other built-ins remain untouched
        assert_eq!(
            registry.get("agent-architect").unwrap().trust_tier,
            TrustTier::BuiltIn
        );

        // Cleanup
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_template_expansion_in_user_agents() {
        let tmp = std::env::temp_dir().join("y-agent-test-template-expansion");
        let agents_dir = tmp.join("agents");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&agents_dir).unwrap();

        let toml_content = r#"
id = "template-agent"
name = "Template Agent"
description = "Writes to {{YAGENT_CONFIG_PATH}}/agents/test.toml"
mode = "general"
trust_tier = "user_defined"
system_prompt = "Store in {{YAGENT_CONFIG_PATH}}/data"
"#;
        std::fs::write(agents_dir.join("template-agent.toml"), toml_content).unwrap();

        let registry = AgentRegistry::new_with_user_agents(Some(&agents_dir));
        let loaded = registry
            .get("template-agent")
            .expect("Agent should load successfully");

        let expected_path = tmp.to_string_lossy();
        assert!(
            loaded.description.contains(&*expected_path),
            "description should contain expanded path"
        );
        assert!(
            loaded.system_prompt.contains(&*expected_path),
            "system_prompt should contain expanded path"
        );
        assert!(
            !loaded.description.contains("{{YAGENT_CONFIG_PATH}}"),
            "template should be expanded"
        );

        // Cleanup
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Hot-reload picks up newly added agent TOML files.
    #[test]
    fn test_reload_user_agents_from_dir() {
        let tmp = std::env::temp_dir().join("y-agent-test-reload");
        let agents_dir = tmp.join("agents");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&agents_dir).unwrap();

        // Start with one user agent.
        std::fs::write(
            agents_dir.join("first.toml"),
            r#"
id = "first-agent"
name = "First Agent"
description = "The first agent"
mode = "general"
trust_tier = "user_defined"
system_prompt = "first"
"#,
        )
        .unwrap();

        let mut registry = AgentRegistry::new_with_user_agents(Some(&agents_dir));
        assert!(registry.get("first-agent").is_some());
        assert!(registry.get("second-agent").is_none());

        // Simulate agent-architect creating a new file at runtime.
        std::fs::write(
            agents_dir.join("second.toml"),
            r#"
id = "second-agent"
name = "Second Agent"
description = "Dynamically created"
mode = "build"
trust_tier = "user_defined"
system_prompt = "second"
"#,
        )
        .unwrap();

        // Before reload, the new agent is not visible.
        assert!(registry.get("second-agent").is_none());

        // After reload, both agents are present.
        let (loaded, errored) = registry.reload_user_agents_from_dir();
        assert_eq!(errored, 0);
        assert!(loaded >= 2);
        assert!(registry.get("first-agent").is_some());
        assert!(registry.get("second-agent").is_some());

        // Built-in agents are still present.
        assert!(registry.get("tool-engineer").is_some());

        // Cleanup
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Single-agent registration from raw TOML content.
    #[test]
    fn test_register_agent_from_toml() {
        let mut registry = AgentRegistry::new();

        let toml = r#"
id = "runtime-agent"
name = "Runtime Agent"
description = "Registered at runtime"
mode = "explore"
trust_tier = "built_in"
system_prompt = "hello"
"#;

        let id = registry.register_agent_from_toml(toml).unwrap();
        assert_eq!(id, "runtime-agent");

        let def = registry.get("runtime-agent").unwrap();
        assert_eq!(def.name, "Runtime Agent");
        // Trust tier is forced to UserDefined regardless of TOML content.
        assert_eq!(def.trust_tier, TrustTier::UserDefined);
        assert_eq!(def.mode, AgentMode::Explore);
    }

    /// Re-registering the same agent ID via `register_agent_from_toml`
    /// replaces the previous definition.
    #[test]
    fn test_register_agent_from_toml_override() {
        let mut registry = AgentRegistry::new();

        let v1 = r#"
id = "evolving-agent"
name = "Evolving Agent v1"
description = "Version 1"
mode = "general"
trust_tier = "user_defined"
system_prompt = "v1"
"#;
        registry.register_agent_from_toml(v1).unwrap();
        assert_eq!(
            registry.get("evolving-agent").unwrap().description,
            "Version 1"
        );

        let v2 = r#"
id = "evolving-agent"
name = "Evolving Agent v2"
description = "Version 2"
mode = "build"
trust_tier = "user_defined"
system_prompt = "v2"
"#;
        registry.register_agent_from_toml(v2).unwrap();
        assert_eq!(
            registry.get("evolving-agent").unwrap().description,
            "Version 2"
        );
        assert_eq!(
            registry.get("evolving-agent").unwrap().mode,
            AgentMode::Build
        );
    }
}
