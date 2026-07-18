//! MCP server lifecycle and tool registration extracted from [`ServiceContainer`].
//!
//! Some operations (`init_mcp_connections`, `register_mcp_tools`,
//! `start_mcp_event_consumer`, `refresh_mcp_server_tools`) need an
//! `Arc<ServiceContainer>` because they spawn background tasks or call
//! other methods that require `Arc`. `deactivate_mcp_server_tools` only
//! needs a shared reference.

use std::sync::Arc;

use tracing::{info, warn};

use crate::container::ServiceContainer;

/// Stateless service encapsulating MCP connection management, tool
/// registration, and lifecycle event handling.
pub struct McpService;

impl McpService {
    /// Connect to configured MCP servers via the connection manager.
    ///
    /// Converts `McpServerConfig` entries from the tool config into
    /// `McpServerConfigRef` and starts concurrent connections. Disabled
    /// servers are skipped. Called as part of `start_background_services`.
    pub async fn init_mcp_connections(container: &Arc<ServiceContainer>) {
        let mcp_configs = &container.tool_registry.config().mcp_servers;
        if mcp_configs.is_empty() {
            return;
        }

        let configs: Vec<y_mcp::McpServerConfigRef> = mcp_configs
            .iter()
            .filter(|c| c.enabled)
            .map(mcp_config_ref)
            .collect();

        if configs.is_empty() {
            return;
        }

        let count = configs.len();
        container.mcp_manager.connect_all(configs).await;
        let connected = container.mcp_manager.connected_count().await;
        info!(
            total = count,
            connected = connected,
            "MCP server connections initialized"
        );
    }

    /// Register MCP tools discovered from connected servers into the tool
    /// registry and taxonomy so they are discoverable via `ToolSearch` and
    /// executable through the standard tool dispatch pipeline.
    pub async fn register_mcp_tools(container: &Arc<ServiceContainer>) {
        let all_tools = container.mcp_manager.list_all_tools().await;
        if all_tools.is_empty() {
            return;
        }

        let mcp_configs = container.tool_registry.config().mcp_servers;
        let mut registered_names: Vec<String> = Vec::new();

        for (server_name, tool_info) in &all_tools {
            if let Some(cfg) = mcp_configs.iter().find(|c| c.name == *server_name) {
                if let Some(ref whitelist) = cfg.enabled_tools {
                    if !whitelist.contains(&tool_info.name) {
                        continue;
                    }
                }
                if let Some(ref blacklist) = cfg.disabled_tools {
                    if blacklist.contains(&tool_info.name) {
                        continue;
                    }
                }
            }

            let prefixed = format!("mcp_{}_{}", server_name, tool_info.name);
            let adapter = y_mcp::McpManagedToolAdapter::new(
                Arc::clone(&container.mcp_manager),
                server_name,
                &tool_info.name,
                &prefixed,
                tool_info.description.as_deref().unwrap_or(""),
                tool_info
                    .input_schema
                    .clone()
                    .unwrap_or(serde_json::json!({})),
            );
            let def = adapter.definition().clone();
            match container
                .tool_registry
                .register_tool(Arc::new(adapter), def)
                .await
            {
                Ok(()) => registered_names.push(prefixed),
                Err(e) => {
                    warn!(tool = %prefixed, error = %e, "failed to register MCP tool");
                }
            }
        }

        if !registered_names.is_empty() {
            let mut taxonomy = container.tool_taxonomy.write().await;
            taxonomy.add_dynamic_category(
                "mcp",
                "MCP tools from external servers",
                registered_names.clone(),
            );
        }

        info!(
            count = registered_names.len(),
            "MCP tools registered in tool registry"
        );

        Self::refresh_mcp_instructions(container).await;
    }

    /// Spawn a background task that listens for MCP lifecycle events
    /// (reconnect / disconnect) and updates the tool registry accordingly.
    pub async fn start_mcp_event_consumer(container: &Arc<ServiceContainer>) {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        container.mcp_manager.set_event_sender(tx).await;

        let container = Arc::clone(container);
        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                match event {
                    y_mcp::McpEvent::ServerReconnected { server_name } => {
                        Self::refresh_mcp_server_tools(&container, &server_name).await;
                    }
                    y_mcp::McpEvent::ServerDisconnected { server_name } => {
                        Self::deactivate_mcp_server_tools(&container, &server_name).await;
                    }
                }
            }
        });
    }

    /// Re-discover and re-register tools for a single MCP server after reconnection.
    pub async fn refresh_mcp_server_tools(container: &Arc<ServiceContainer>, server_name: &str) {
        let prefix = format!("mcp_{server_name}_");

        // Remove old tools for this server from the registry.
        let all_defs = container.tool_registry.get_all_definitions().await;
        {
            let mut set = container.tool_activation_set.write().await;
            for def in &all_defs {
                if def.name.as_str().starts_with(&prefix) {
                    container.tool_registry.unregister_tool(&def.name).await;
                    set.deactivate(&def.name);
                }
            }
        }

        // Re-discover tools from the reconnected server.
        let all_tools = container.mcp_manager.list_all_tools().await;
        let server_tools: Vec<_> = all_tools
            .into_iter()
            .filter(|(name, _)| name == server_name)
            .collect();

        let mcp_configs = container.tool_registry.config().mcp_servers;
        let selected_config = mcp_configs
            .iter()
            .find(|config| config.name == server_name)
            .cloned();
        #[cfg(feature = "capability_packs")]
        if selected_config.is_none() {
            selected_config = container
                .active_capability_pack_mcp
                .read()
                .await
                .get(server_name)
                .cloned();
        }
        let mut registered_names = Vec::new();

        for (sname, tool_info) in &server_tools {
            if let Some(cfg) = selected_config.as_ref() {
                if let Some(ref wl) = cfg.enabled_tools {
                    if !wl.contains(&tool_info.name) {
                        continue;
                    }
                }
                if let Some(ref bl) = cfg.disabled_tools {
                    if bl.contains(&tool_info.name) {
                        continue;
                    }
                }
            }

            let prefixed = format!("mcp_{}_{}", sname, tool_info.name);
            let adapter = y_mcp::McpManagedToolAdapter::new(
                Arc::clone(&container.mcp_manager),
                sname,
                &tool_info.name,
                &prefixed,
                tool_info.description.as_deref().unwrap_or(""),
                tool_info
                    .input_schema
                    .clone()
                    .unwrap_or(serde_json::json!({})),
            );
            let def = adapter.definition().clone();
            if container
                .tool_registry
                .register_tool(Arc::new(adapter), def)
                .await
                .is_ok()
            {
                registered_names.push(prefixed);
            }
        }

        if !registered_names.is_empty() {
            let mut taxonomy = container.tool_taxonomy.write().await;
            taxonomy.add_dynamic_category(
                "mcp",
                "MCP tools from external servers",
                registered_names.clone(),
            );
        }

        info!(
            server = %server_name,
            count = registered_names.len(),
            "MCP server tools refreshed after reconnection"
        );
        Self::reconcile_mcp_taxonomy(container).await;
        Self::refresh_mcp_instructions(container).await;
    }

    /// Deactivate tools from a disconnected MCP server.
    pub async fn deactivate_mcp_server_tools(container: &ServiceContainer, server_name: &str) {
        let prefix = format!("mcp_{server_name}_");
        let all_defs = container.tool_registry.get_all_definitions().await;
        let mut removed = 0usize;
        {
            let mut set = container.tool_activation_set.write().await;
            for def in &all_defs {
                if def.name.as_str().starts_with(&prefix) {
                    container.tool_registry.unregister_tool(&def.name).await;
                    set.deactivate(&def.name);
                    removed += 1;
                }
            }
        }
        if removed > 0 {
            info!(
                server = %server_name,
                removed,
                "MCP server tools deactivated after disconnect"
            );
        }
        Self::reconcile_mcp_taxonomy(container).await;
        Self::refresh_mcp_instructions(container).await;
    }

    #[cfg(feature = "capability_packs")]
    pub(crate) async fn activate_capability_pack_server(
        container: &Arc<ServiceContainer>,
        config: y_tools::McpServerConfig,
    ) -> Result<(), String> {
        if !config.enabled {
            return Err(format!(
                "capability-pack MCP declaration is disabled: {}",
                config.name
            ));
        }
        if container
            .tool_registry
            .config()
            .mcp_servers
            .iter()
            .any(|configured| configured.name == config.name)
        {
            return Err(format!(
                "capability-pack MCP server conflicts with user configuration: {}",
                config.name
            ));
        }

        let already_owned = container
            .active_capability_pack_mcp
            .read()
            .await
            .contains_key(&config.name);
        if !already_owned
            && matches!(
                container.mcp_manager.server_status(&config.name).await,
                Some(y_mcp::McpServerStatus::Connected)
            )
        {
            return Err(format!(
                "MCP server is already connected outside capability-pack ownership: {}",
                config.name
            ));
        }

        if !matches!(
            container.mcp_manager.server_status(&config.name).await,
            Some(y_mcp::McpServerStatus::Connected)
        ) {
            container
                .mcp_manager
                .connect_all(vec![mcp_config_ref(&config)])
                .await;
        }
        match container.mcp_manager.server_status(&config.name).await {
            Some(y_mcp::McpServerStatus::Connected) => {}
            Some(status) => {
                return Err(format!(
                    "capability-pack MCP server failed to connect: {} ({status})",
                    config.name
                ));
            }
            None => {
                return Err(format!(
                    "capability-pack MCP server produced no connection state: {}",
                    config.name
                ));
            }
        }

        container
            .active_capability_pack_mcp
            .write()
            .await
            .insert(config.name.clone(), config.clone());
        if let Err(error) = Self::register_capability_pack_server_tools(container, &config).await {
            let _ = container.mcp_manager.disconnect(&config.name).await;
            container
                .active_capability_pack_mcp
                .write()
                .await
                .remove(&config.name);
            return Err(error);
        }
        Self::refresh_mcp_instructions(container).await;
        Ok(())
    }

    #[cfg(feature = "capability_packs")]
    pub(crate) async fn deactivate_capability_pack_server(
        container: &ServiceContainer,
        server_name: &str,
    ) -> Result<bool, String> {
        if !container
            .active_capability_pack_mcp
            .read()
            .await
            .contains_key(server_name)
        {
            return Ok(false);
        }
        Self::deactivate_mcp_server_tools(container, server_name).await;
        container
            .mcp_manager
            .disconnect(server_name)
            .await
            .map_err(|error| format!("failed to disconnect MCP server {server_name}: {error}"))?;
        container
            .active_capability_pack_mcp
            .write()
            .await
            .remove(server_name);
        Self::refresh_mcp_instructions(container).await;
        Ok(true)
    }

    #[cfg(feature = "capability_packs")]
    async fn register_capability_pack_server_tools(
        container: &Arc<ServiceContainer>,
        config: &y_tools::McpServerConfig,
    ) -> Result<(), String> {
        Self::deactivate_mcp_server_tools(container, &config.name).await;
        let server_tools = container
            .mcp_manager
            .list_all_tools()
            .await
            .into_iter()
            .filter(|(server_name, _)| server_name == &config.name)
            .collect::<Vec<_>>();
        let mut registered_names = Vec::new();
        for (server_name, tool_info) in server_tools {
            if config
                .enabled_tools
                .as_ref()
                .is_some_and(|enabled| !enabled.contains(&tool_info.name))
                || config
                    .disabled_tools
                    .as_ref()
                    .is_some_and(|disabled| disabled.contains(&tool_info.name))
            {
                continue;
            }
            let prefixed = format!("mcp_{}_{}", server_name, tool_info.name);
            let adapter = y_mcp::McpManagedToolAdapter::new(
                Arc::clone(&container.mcp_manager),
                &server_name,
                &tool_info.name,
                &prefixed,
                tool_info.description.as_deref().unwrap_or(""),
                tool_info
                    .input_schema
                    .unwrap_or_else(|| serde_json::json!({})),
            );
            let definition = adapter.definition().clone();
            if let Err(error) = container
                .tool_registry
                .register_tool(Arc::new(adapter), definition)
                .await
            {
                Self::deactivate_mcp_server_tools(container, &config.name).await;
                return Err(format!(
                    "failed to register capability-pack MCP tool {prefixed}: {error}"
                ));
            }
            registered_names.push(prefixed);
        }
        if !registered_names.is_empty() {
            container.tool_taxonomy.write().await.add_dynamic_category(
                "mcp",
                "MCP tools from external servers",
                registered_names,
            );
        }
        Self::reconcile_mcp_taxonomy(container).await;
        Ok(())
    }

    async fn reconcile_mcp_taxonomy(container: &ServiceContainer) {
        let live_tools = container
            .tool_registry
            .get_all_definitions()
            .await
            .into_iter()
            .filter_map(|definition| {
                definition
                    .name
                    .as_str()
                    .starts_with("mcp_")
                    .then(|| definition.name.as_str().to_string())
            })
            .collect::<std::collections::HashSet<_>>();
        container
            .tool_taxonomy
            .write()
            .await
            .retain_category_tools("mcp", &live_tools);
    }

    async fn refresh_mcp_instructions(container: &ServiceContainer) {
        use std::fmt::Write;

        let instructions = container.mcp_manager.collect_server_instructions().await;
        let text = if instructions.is_empty() {
            None
        } else {
            let mut text = String::from("## MCP Server Instructions\n");
            for (server_name, instruction) in &instructions {
                let _ = write!(text, "\n### {server_name}\n{instruction}\n");
            }
            Some(text)
        };
        container
            .prompt_context
            .write()
            .await
            .mcp_server_instructions = text;
    }
}

fn mcp_config_ref(config: &y_tools::McpServerConfig) -> y_mcp::McpServerConfigRef {
    y_mcp::McpServerConfigRef {
        name: config.name.clone(),
        transport: config.transport.clone(),
        command: config.command.clone(),
        args: config.args.clone(),
        url: config.url.clone(),
        env: config.env.clone(),
        headers: config.headers.clone(),
        startup_timeout_secs: config.startup_timeout_secs,
        tool_timeout_secs: config.tool_timeout_secs,
        cwd: config.cwd.clone(),
        bearer_token: config.bearer_token.clone(),
        auto_reconnect: config.auto_reconnect,
        max_reconnect_attempts: config.max_reconnect_attempts,
    }
}
