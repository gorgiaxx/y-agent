//! Agent CRUD operations extracted from [`ServiceContainer`].
//!
//! Follows the same unit-struct + static-method pattern as
//! [`crate::chat::ChatService`]: all methods take `&ServiceContainer`
//! (or sub-components thereof) as their first argument so the service
//! carries no state of its own.

use std::sync::Arc;

use tokio::sync::RwLock;
use tracing::info;
use y_agent::AgentRegistry;

use crate::container::ServiceContainer;

/// Stateless service encapsulating agent definition management (CRUD,
/// reload, callable-text refresh).
pub struct AgentManagementService;

impl AgentManagementService {
    /// Hot-reload agent definitions from the agents directory.
    ///
    /// Re-scans all `*.toml` files in the user agents directory and
    /// registers (or overrides) definitions. Built-in agents are preserved.
    /// Newly created agent files take effect immediately without restart.
    ///
    /// Returns `(loaded, errored)` counts.
    pub async fn reload_agents(container: &ServiceContainer) -> (usize, usize) {
        let mut registry = container.agent_registry.lock().await;
        let (loaded, errored) = registry.reload_user_agents_from_dir();
        info!(loaded, errored, "Agent definitions hot-reloaded");
        Self::refresh_callable_agents_text(&registry, &container.callable_agents_text).await;
        (loaded, errored)
    }

    /// Register a single agent from raw TOML content at runtime.
    ///
    /// Useful when `agent-architect` creates a new agent definition and
    /// wants it to take effect immediately without a full directory scan.
    ///
    /// Returns the registered agent's ID on success.
    pub async fn register_agent_from_toml(
        container: &ServiceContainer,
        toml_content: &str,
    ) -> Result<String, String> {
        let mut registry = container.agent_registry.lock().await;
        let id = registry.register_agent_from_toml(toml_content)?;
        info!(agent_id = %id, "Agent definition registered at runtime");
        Self::refresh_callable_agents_text(&registry, &container.callable_agents_text).await;
        Ok(id)
    }

    /// Refresh the callable agents text injected into the orchestration prompt.
    ///
    /// Reads all definitions from the registry where `user_callable == true`
    /// and writes a markdown-formatted summary into the shared handle.
    async fn refresh_callable_agents_text(registry: &AgentRegistry, handle: &Arc<RwLock<String>>) {
        let callable: Vec<_> = registry
            .list()
            .into_iter()
            .filter(|d| d.user_callable)
            .collect();

        let text = if callable.is_empty() {
            String::from("### User-Callable Agents\n\n(none currently registered)")
        } else {
            let mut buf = String::from("### User-Callable Agents\n\n");
            for agent in &callable {
                use std::fmt::Write;
                let _ = writeln!(
                    buf,
                    "- **{}**: {} (mode: {:?}, capabilities: [{}])",
                    agent.id,
                    agent.description,
                    agent.mode,
                    agent.capabilities.join(", "),
                );
            }
            buf
        };

        let mut guard = handle.write().await;
        *guard = text;
    }

    /// Populate the callable agents text at startup.
    ///
    /// Called once after construction so the first prompt assembly has the list.
    pub async fn init_callable_agents_text(container: &ServiceContainer) {
        let registry = container.agent_registry.lock().await;
        Self::refresh_callable_agents_text(&registry, &container.callable_agents_text).await;
    }

    /// Save an agent definition from raw TOML content to the agents directory.
    ///
    /// Parses the TOML, writes the file to disk, and registers the definition
    /// in the agent registry with `UserDefined` trust tier.
    pub async fn save_agent(
        container: &ServiceContainer,
        id: &str,
        toml_content: &str,
    ) -> Result<(), String> {
        let mut registry = container.agent_registry.lock().await;

        let expanded_toml = registry.expand_templates(toml_content);
        let mut def = y_agent::agent::definition::AgentDefinition::from_toml(&expanded_toml)
            .map_err(|e| format!("Invalid agent TOML: {e}"))?;

        def.id = id.to_string();

        let dir = registry
            .agents_dir()
            .ok_or_else(|| "no agents directory configured".to_string())?
            .to_path_buf();

        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| format!("failed to create agents directory: {e}"))?;

        let file_path = dir.join(format!("{id}.toml"));
        tokio::fs::write(&file_path, toml_content)
            .await
            .map_err(|e| format!("failed to write agent file: {e}"))?;

        def.trust_tier = y_agent::TrustTier::UserDefined;
        let _ = registry.register_or_override(def);

        Self::refresh_callable_agents_text(&registry, &container.callable_agents_text).await;
        Ok(())
    }

    /// Reset an overridden built-in agent to its original definition.
    ///
    /// Removes the user override file from disk and restores the built-in
    /// definition in the registry.
    pub async fn reset_agent(container: &ServiceContainer, id: &str) -> Result<(), String> {
        let mut registry = container.agent_registry.lock().await;
        registry
            .reset_builtin(id)
            .map_err(|e| format!("failed to reset agent: {e}"))?;

        if let Some(dir) = registry.agents_dir() {
            let file_path = dir.join(format!("{id}.toml"));
            if file_path.exists() {
                tokio::fs::remove_file(&file_path)
                    .await
                    .map_err(|e| format!("failed to remove override file: {e}"))?;
            }
        }

        Self::refresh_callable_agents_text(&registry, &container.callable_agents_text).await;
        Ok(())
    }

    /// Read the raw TOML source for an agent definition.
    ///
    /// Returns `(path, content, is_user_file)`. If a user override file exists
    /// on disk, returns its content; otherwise serializes the in-memory definition.
    pub async fn get_agent_source(
        container: &ServiceContainer,
        id: &str,
    ) -> Result<(String, String, bool), String> {
        let registry = container.agent_registry.lock().await;
        let def = registry
            .get(id)
            .ok_or_else(|| format!("agent not found: {id}"))?;

        let file_path = registry
            .agents_dir()
            .map(|d| d.join(format!("{}.toml", def.id)))
            .unwrap_or_default();

        if file_path.exists() {
            let content = tokio::fs::read_to_string(&file_path)
                .await
                .map_err(|e| format!("failed to read agent file: {e}"))?;
            return Ok((file_path.display().to_string(), content, true));
        }

        let content =
            toml::to_string_pretty(def).map_err(|e| format!("failed to serialize agent: {e}"))?;
        Ok((file_path.display().to_string(), content, false))
    }
}
