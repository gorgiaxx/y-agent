//! MCP Connection Manager: lifecycle management for multiple MCP servers.
//!
//! Provides concurrent startup, status tracking, aggregated tool listing,
//! tool call routing, and `ToolListChanged` notification handling.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::auth::McpAuthStore;
use crate::client::{McpClient, McpToolInfo};
use crate::error::McpError;
use crate::transport::{HttpTransport, McpTransport, NotificationHandler, StdioTransport};

/// Status of an individual MCP server connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpServerStatus {
    /// Successfully connected and initialized.
    Connected,
    /// Connection attempt in progress.
    Connecting,
    /// Not connected (never started or cleanly disconnected).
    Disconnected,
    /// Connection or initialization failed.
    Failed { error: String },
    /// Server is configured but disabled.
    Disabled,
}

impl std::fmt::Display for McpServerStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Connected => write!(f, "connected"),
            Self::Connecting => write!(f, "connecting"),
            Self::Disconnected => write!(f, "disconnected"),
            Self::Failed { error } => write!(f, "failed: {error}"),
            Self::Disabled => write!(f, "disabled"),
        }
    }
}

/// Internal state for a single MCP server.
struct ServerState {
    client: Arc<McpClient>,
    status: McpServerStatus,
    tools: Vec<McpToolInfo>,
    #[allow(dead_code)] // kept for reconnection support
    config: McpServerConfigRef,
}

/// Minimal config reference stored alongside each server state.
///
/// Avoids a dependency on the full `McpServerConfig` from y-tools.
#[derive(Debug, Clone)]
pub struct McpServerConfigRef {
    pub name: String,
    pub transport: String,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub url: Option<String>,
    pub env: HashMap<String, String>,
    pub headers: HashMap<String, String>,
    pub startup_timeout_secs: u64,
    pub tool_timeout_secs: u64,
    pub cwd: Option<String>,
    pub bearer_token: Option<String>,
}

/// Central orchestrator for multiple MCP server connections.
///
/// Manages connection lifecycle, tool caching, and call routing
/// across all configured MCP servers.
pub struct McpConnectionManager {
    servers: RwLock<HashMap<String, ServerState>>,
    auth_store: Option<McpAuthStore>,
}

impl McpConnectionManager {
    /// Create a new empty connection manager.
    pub fn new(auth_store: Option<McpAuthStore>) -> Self {
        Self {
            servers: RwLock::new(HashMap::new()),
            auth_store,
        }
    }

    /// Connect to multiple servers concurrently.
    ///
    /// Each server is started in a separate task. Failures are logged
    /// and reflected in the server status; they do not abort other connections.
    pub async fn connect_all(&self, configs: Vec<McpServerConfigRef>) {
        let mut set = tokio::task::JoinSet::new();

        for config in configs {
            let name = config.name.clone();
            let auth_store = self
                .auth_store
                .as_ref()
                .map(|s| McpAuthStore::new(s.path().to_path_buf()));
            let startup_timeout = Duration::from_secs(config.startup_timeout_secs);

            set.spawn(async move {
                let result = tokio::time::timeout(
                    startup_timeout,
                    connect_single_server(&config, auth_store.as_ref()),
                )
                .await;

                match result {
                    Ok(Ok((client, tools))) => (name, config, Ok((client, tools))),
                    Ok(Err(e)) => (name, config, Err(e.to_string())),
                    Err(_) => (
                        name,
                        config,
                        Err(format!(
                            "startup timed out after {}s",
                            startup_timeout.as_secs()
                        )),
                    ),
                }
            });
        }

        while let Some(result) = set.join_next().await {
            match result {
                Ok((name, config, Ok((client, tools)))) => {
                    info!(
                        server = %name,
                        tool_count = tools.len(),
                        "MCP server connected"
                    );
                    let mut servers = self.servers.write().await;
                    servers.insert(
                        name,
                        ServerState {
                            client,
                            status: McpServerStatus::Connected,
                            tools,
                            config,
                        },
                    );
                }
                Ok((name, config, Err(error))) => {
                    warn!(server = %name, %error, "MCP server connection failed");
                    let client_placeholder =
                        Arc::new(McpClient::new(Arc::new(NullTransport), &name));
                    let mut servers = self.servers.write().await;
                    servers.insert(
                        name,
                        ServerState {
                            client: client_placeholder,
                            status: McpServerStatus::Failed {
                                error: error.clone(),
                            },
                            tools: Vec::new(),
                            config,
                        },
                    );
                }
                Err(e) => {
                    warn!(error = %e, "MCP server connection task panicked");
                }
            }
        }
    }

    /// Get the status of all servers.
    pub async fn status(&self) -> HashMap<String, McpServerStatus> {
        let servers = self.servers.read().await;
        servers
            .iter()
            .map(|(name, state)| (name.clone(), state.status.clone()))
            .collect()
    }

    /// Get the status of a single server.
    pub async fn server_status(&self, name: &str) -> Option<McpServerStatus> {
        let servers = self.servers.read().await;
        servers.get(name).map(|s| s.status.clone())
    }

    /// List all tools across all connected servers.
    ///
    /// Returns `(server_name, tool_info)` pairs.
    pub async fn list_all_tools(&self) -> Vec<(String, McpToolInfo)> {
        let servers = self.servers.read().await;
        let mut result = Vec::new();
        for (name, state) in servers.iter() {
            if state.status == McpServerStatus::Connected {
                for tool in &state.tools {
                    result.push((name.clone(), tool.clone()));
                }
            }
        }
        result
    }

    /// Call a tool on a specific server.
    pub async fn call_tool(
        &self,
        server_name: &str,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, McpError> {
        let servers = self.servers.read().await;
        let state = servers.get(server_name).ok_or_else(|| McpError::Other {
            message: format!("unknown MCP server: {server_name}"),
        })?;

        if state.status != McpServerStatus::Connected {
            return Err(McpError::ConnectionFailed {
                message: format!("server '{server_name}' is not connected: {}", state.status),
            });
        }

        state.client.call_tool(tool_name, arguments).await
    }

    /// Refresh the tool list for a specific server.
    ///
    /// Called when the server sends a `notifications/tools/list_changed`
    /// notification.
    pub async fn refresh_tools(&self, server_name: &str) -> Result<usize, McpError> {
        let mut servers = self.servers.write().await;
        let state = servers
            .get_mut(server_name)
            .ok_or_else(|| McpError::Other {
                message: format!("unknown MCP server: {server_name}"),
            })?;

        if state.status != McpServerStatus::Connected {
            return Err(McpError::ConnectionFailed {
                message: format!("cannot refresh tools: server '{server_name}' is not connected"),
            });
        }

        let tools = state.client.list_tools().await?;
        let count = tools.len();
        info!(
            server = %server_name,
            tool_count = count,
            "refreshed MCP tool list"
        );
        state.tools = tools;
        Ok(count)
    }

    /// Disconnect a specific server.
    pub async fn disconnect(&self, server_name: &str) -> Result<(), McpError> {
        let mut servers = self.servers.write().await;
        if let Some(state) = servers.get_mut(server_name) {
            state.client.close().await?;
            state.status = McpServerStatus::Disconnected;
            state.tools.clear();
            info!(server = %server_name, "MCP server disconnected");
        }
        Ok(())
    }

    /// Gracefully close all server connections.
    pub async fn close_all(&self) {
        let mut servers = self.servers.write().await;
        for (name, state) in servers.iter_mut() {
            if state.status == McpServerStatus::Connected {
                if let Err(e) = state.client.close().await {
                    warn!(server = %name, error = %e, "error closing MCP server");
                }
                state.status = McpServerStatus::Disconnected;
                state.tools.clear();
            }
        }
        info!("all MCP servers closed");
    }

    /// Get the number of connected servers.
    pub async fn connected_count(&self) -> usize {
        let servers = self.servers.read().await;
        servers
            .values()
            .filter(|s| s.status == McpServerStatus::Connected)
            .count()
    }
}

/// Connect to a single MCP server and return the client + discovered tools.
async fn connect_single_server(
    config: &McpServerConfigRef,
    auth_store: Option<&McpAuthStore>,
) -> Result<(Arc<McpClient>, Vec<McpToolInfo>), McpError> {
    let transport: Arc<dyn McpTransport> = match config.transport.as_str() {
        "stdio" => {
            let command = config.command.as_deref().ok_or_else(|| McpError::Other {
                message: "stdio transport requires a 'command' field".into(),
            })?;
            Arc::new(StdioTransport::spawn(
                command,
                &config.args,
                &config.env,
                config.cwd.as_deref(),
            )?)
        }
        "http" => {
            let url = config.url.as_deref().unwrap_or("http://localhost:3000");
            let mut builder = HttpTransport::builder(url)
                .server_name(&config.name)
                .headers(config.headers.clone())
                .timeout(Duration::from_secs(config.tool_timeout_secs));

            let token = crate::auth::resolve_bearer_token(
                &config.name,
                config.bearer_token.as_deref(),
                auth_store,
            );
            if let Some(t) = token {
                builder = builder.bearer_token(t);
            }

            Arc::new(builder.build()?)
        }
        other => {
            return Err(McpError::Other {
                message: format!("unsupported transport: {other}"),
            });
        }
    };

    let client = Arc::new(McpClient::new(transport, &config.name));
    client.initialize().await?;

    let tools = client.list_tools().await?;
    debug!(
        server = %config.name,
        tool_count = tools.len(),
        "discovered tools from MCP server"
    );

    Ok((client, tools))
}

/// Null transport used as a placeholder for failed connections.
struct NullTransport;

#[async_trait::async_trait]
impl McpTransport for NullTransport {
    async fn send(
        &self,
        _request: crate::transport::JsonRpcRequest,
    ) -> Result<crate::transport::JsonRpcResponse, McpError> {
        Err(McpError::ConnectionFailed {
            message: "server is not connected".into(),
        })
    }

    async fn send_notification(
        &self,
        _notification: crate::transport::JsonRpcNotification,
    ) -> Result<(), McpError> {
        Err(McpError::ConnectionFailed {
            message: "server is not connected".into(),
        })
    }

    async fn close(&self) -> Result<(), McpError> {
        Ok(())
    }

    fn transport_type(&self) -> &'static str {
        "null"
    }
}

/// Create a [`NotificationHandler`] that triggers tool refresh on the given
/// connection manager when `notifications/tools/list_changed` is received.
pub fn tool_list_changed_handler(
    manager: Arc<McpConnectionManager>,
    server_name: String,
) -> NotificationHandler {
    Arc::new(move |method: &str, _params: Option<serde_json::Value>| {
        if method == "notifications/tools/list_changed" {
            let mgr = Arc::clone(&manager);
            let name = server_name.clone();
            tokio::spawn(async move {
                match mgr.refresh_tools(&name).await {
                    Ok(count) => {
                        info!(
                            server = %name,
                            tool_count = count,
                            "tool list refreshed after ToolListChanged notification"
                        );
                    }
                    Err(e) => {
                        warn!(
                            server = %name,
                            error = %e,
                            "failed to refresh tools after ToolListChanged notification"
                        );
                    }
                }
            });
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_server_status_display() {
        assert_eq!(McpServerStatus::Connected.to_string(), "connected");
        assert_eq!(McpServerStatus::Connecting.to_string(), "connecting");
        assert_eq!(McpServerStatus::Disconnected.to_string(), "disconnected");
        assert_eq!(McpServerStatus::Disabled.to_string(), "disabled");
        assert_eq!(
            McpServerStatus::Failed {
                error: "timeout".into()
            }
            .to_string(),
            "failed: timeout"
        );
    }

    #[test]
    fn test_config_ref_clone() {
        let config = McpServerConfigRef {
            name: "test".into(),
            transport: "stdio".into(),
            command: Some("echo".into()),
            args: vec!["hello".into()],
            url: None,
            env: HashMap::new(),
            headers: HashMap::new(),
            startup_timeout_secs: 30,
            tool_timeout_secs: 120,
            cwd: None,
            bearer_token: None,
        };
        let cloned = config.clone();
        assert_eq!(cloned.name, "test");
        assert_eq!(cloned.startup_timeout_secs, 30);
    }

    #[tokio::test]
    async fn test_empty_manager() {
        let manager = McpConnectionManager::new(None);
        assert_eq!(manager.connected_count().await, 0);
        assert!(manager.status().await.is_empty());
        assert!(manager.list_all_tools().await.is_empty());
    }

    #[tokio::test]
    async fn test_call_tool_unknown_server() {
        let manager = McpConnectionManager::new(None);
        let result = manager
            .call_tool("nonexistent", "tool", serde_json::json!({}))
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_disconnect_unknown_server() {
        let manager = McpConnectionManager::new(None);
        // Disconnecting an unknown server is a no-op.
        assert!(manager.disconnect("nonexistent").await.is_ok());
    }

    #[tokio::test]
    async fn test_close_all_empty() {
        let manager = McpConnectionManager::new(None);
        manager.close_all().await;
        assert_eq!(manager.connected_count().await, 0);
    }

    #[tokio::test]
    async fn test_connect_all_with_bad_command() {
        let manager = McpConnectionManager::new(None);
        let config = McpServerConfigRef {
            name: "bad-server".into(),
            transport: "stdio".into(),
            command: Some("__nonexistent_cmd__".into()),
            args: vec![],
            url: None,
            env: HashMap::new(),
            headers: HashMap::new(),
            startup_timeout_secs: 5,
            tool_timeout_secs: 30,
            cwd: None,
            bearer_token: None,
        };

        manager.connect_all(vec![config]).await;

        let status = manager.server_status("bad-server").await;
        assert!(matches!(status, Some(McpServerStatus::Failed { .. })));
        assert_eq!(manager.connected_count().await, 0);
    }
}
