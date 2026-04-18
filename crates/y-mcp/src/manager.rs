//! MCP Connection Manager: lifecycle management for multiple MCP servers.
//!
//! Provides concurrent startup, status tracking, aggregated tool/resource/prompt
//! listing, call routing, notification dispatch, and automatic reconnection
//! with exponential backoff.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, RwLock};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use crate::auth::McpAuthStore;
use crate::client::{
    GetPromptResult, McpClient, McpPrompt, McpResource, McpToolInfo, ReadResourceResult,
};
use crate::error::McpError;
use crate::transport::{HttpTransport, McpTransport, NotificationHandler, StdioTransport};

/// Status of an individual MCP server connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpServerStatus {
    /// Successfully connected and initialized.
    Connected,
    /// Connection attempt in progress.
    Connecting,
    /// Reconnection in progress after an unexpected disconnect.
    Reconnecting { attempt: u32, next_delay_ms: u64 },
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
            Self::Reconnecting {
                attempt,
                next_delay_ms,
            } => write!(
                f,
                "reconnecting (attempt {attempt}, next in {next_delay_ms}ms)"
            ),
            Self::Disconnected => write!(f, "disconnected"),
            Self::Failed { error } => write!(f, "failed: {error}"),
            Self::Disabled => write!(f, "disabled"),
        }
    }
}

/// Reconnection policy: exponential backoff with bounded attempts.
#[derive(Debug, Clone)]
pub struct ReconnectPolicy {
    pub enabled: bool,
    pub max_retries: u32,
    pub initial_delay: Duration,
    pub max_delay: Duration,
    pub multiplier: f64,
}

impl Default for ReconnectPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            max_retries: 5,
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(30),
            multiplier: 2.0,
        }
    }
}

impl ReconnectPolicy {
    /// Compute the delay before the given attempt number (1-indexed).
    pub fn backoff_delay(&self, attempt: u32) -> Duration {
        let attempt = attempt.saturating_sub(1);
        let millis = (self.initial_delay.as_millis() as f64)
            * self
                .multiplier
                .powi(i32::try_from(attempt).unwrap_or(i32::MAX));
        let capped = millis.min(self.max_delay.as_millis() as f64);
        Duration::from_millis(capped as u64)
    }
}

/// Internal state for a single MCP server.
struct ServerState {
    client: Arc<McpClient>,
    status: McpServerStatus,
    tools: Vec<McpToolInfo>,
    resources: Vec<McpResource>,
    prompts: Vec<McpPrompt>,
    instructions: Option<String>,
    config: McpServerConfigRef,
    reconnect_attempt: u32,
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
    /// Whether to auto-reconnect on unexpected disconnect.
    pub auto_reconnect: bool,
    /// Maximum reconnection attempts before giving up.
    pub max_reconnect_attempts: u32,
}

impl Default for McpServerConfigRef {
    fn default() -> Self {
        Self {
            name: String::new(),
            transport: String::new(),
            command: None,
            args: Vec::new(),
            url: None,
            env: HashMap::new(),
            headers: HashMap::new(),
            startup_timeout_secs: 30,
            tool_timeout_secs: 120,
            cwd: None,
            bearer_token: None,
            auto_reconnect: true,
            max_reconnect_attempts: 5,
        }
    }
}

/// Lifecycle events emitted by the connection manager.
#[derive(Debug, Clone)]
pub enum McpEvent {
    /// A server reconnected after an unexpected disconnect.
    ServerReconnected { server_name: String },
    /// A server disconnected and will not auto-reconnect (or exhausted retries).
    ServerDisconnected { server_name: String },
}

/// Central orchestrator for multiple MCP server connections.
pub struct McpConnectionManager {
    servers: RwLock<HashMap<String, ServerState>>,
    auth_store: Option<McpAuthStore>,
    supervisors: RwLock<Vec<JoinHandle<()>>>,
    reconnect_policy: ReconnectPolicy,
    event_tx: RwLock<Option<mpsc::UnboundedSender<McpEvent>>>,
}

impl McpConnectionManager {
    /// Create a new empty connection manager.
    pub fn new(auth_store: Option<McpAuthStore>) -> Self {
        Self {
            servers: RwLock::new(HashMap::new()),
            auth_store,
            supervisors: RwLock::new(Vec::new()),
            reconnect_policy: ReconnectPolicy::default(),
            event_tx: RwLock::new(None),
        }
    }

    /// Set the global reconnect policy.
    #[must_use]
    pub fn with_reconnect_policy(mut self, policy: ReconnectPolicy) -> Self {
        self.reconnect_policy = policy;
        self
    }

    /// Register an event sender for lifecycle notifications.
    pub async fn set_event_sender(&self, tx: mpsc::UnboundedSender<McpEvent>) {
        *self.event_tx.write().await = Some(tx);
    }

    /// Emit a lifecycle event (best-effort, never blocks).
    async fn emit_event(&self, event: McpEvent) {
        if let Some(ref tx) = *self.event_tx.read().await {
            let _ = tx.send(event);
        }
    }

    /// Connect to multiple servers concurrently.
    pub async fn connect_all(self: &Arc<Self>, configs: Vec<McpServerConfigRef>) {
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
                    Ok(Ok(built)) => (name, config, Ok(built)),
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
                Ok((name, config, Ok(built))) => {
                    info!(
                        server = %name,
                        tool_count = built.tools.len(),
                        resource_count = built.resources.len(),
                        prompt_count = built.prompts.len(),
                        "MCP server connected"
                    );
                    self.install_server(name, config, built).await;
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
                            resources: Vec::new(),
                            prompts: Vec::new(),
                            instructions: None,
                            config,
                            reconnect_attempt: 0,
                        },
                    );
                }
                Err(e) => {
                    warn!(error = %e, "MCP server connection task panicked");
                }
            }
        }
    }

    /// Install a freshly-connected server into the state map and wire up its
    /// notification handler and disconnect supervisor.
    ///
    /// Returns a boxed future to break recursive async cycles with
    /// `reconnect -> install_server`.
    fn install_server(
        self: &Arc<Self>,
        name: String,
        config: McpServerConfigRef,
        built: ConnectedServer,
    ) -> futures::future::BoxFuture<'_, ()> {
        Box::pin(async move {
            let ConnectedServer {
                client,
                tools,
                resources,
                prompts,
                instructions,
                disconnect_rx,
            } = built;

            // Register the unified notification dispatcher.
            client
                .set_notification_handler(notification_dispatcher(Arc::clone(self), name.clone()));

            // Spawn supervisor task if disconnect detection is available.
            if let Some(rx) = disconnect_rx {
                let mgr = Arc::clone(self);
                let supervised_name = name.clone();
                let handle = tokio::spawn(async move {
                    let mut rx = rx;
                    if rx.recv().await.is_some() {
                        warn!(server = %supervised_name, "MCP transport disconnected");
                        mgr.handle_disconnect(&supervised_name).await;
                    }
                });
                self.supervisors.write().await.push(handle);
            }

            let mut servers = self.servers.write().await;
            servers.insert(
                name,
                ServerState {
                    client,
                    status: McpServerStatus::Connected,
                    tools,
                    resources,
                    prompts,
                    instructions,
                    config,
                    reconnect_attempt: 0,
                },
            );
        })
    }

    /// Called by the supervisor when a transport unexpectedly disconnects.
    async fn handle_disconnect(self: &Arc<Self>, server_name: &str) {
        let should_reconnect = {
            let servers = self.servers.read().await;
            servers
                .get(server_name)
                .is_some_and(|s| s.config.auto_reconnect && self.reconnect_policy.enabled)
        };

        if !should_reconnect {
            let mut servers = self.servers.write().await;
            if let Some(state) = servers.get_mut(server_name) {
                state.status = McpServerStatus::Disconnected;
                state.tools.clear();
                state.resources.clear();
                state.prompts.clear();
            }
            self.emit_event(McpEvent::ServerDisconnected {
                server_name: server_name.to_string(),
            })
            .await;
            return;
        }

        self.reconnect(server_name).await;
    }

    /// Attempt to reconnect a server, applying the configured backoff policy.
    pub async fn reconnect(self: &Arc<Self>, server_name: &str) {
        let config = {
            let servers = self.servers.read().await;
            match servers.get(server_name) {
                Some(s) => s.config.clone(),
                None => return,
            }
        };

        let max_retries = config
            .max_reconnect_attempts
            .min(self.reconnect_policy.max_retries);
        let auth_store = self
            .auth_store
            .as_ref()
            .map(|s| McpAuthStore::new(s.path().to_path_buf()));

        for attempt in 1..=max_retries {
            let delay = self.reconnect_policy.backoff_delay(attempt);
            {
                let mut servers = self.servers.write().await;
                if let Some(state) = servers.get_mut(server_name) {
                    state.status = McpServerStatus::Reconnecting {
                        attempt,
                        next_delay_ms: delay.as_millis() as u64,
                    };
                    state.reconnect_attempt = attempt;
                }
            }

            info!(
                server = %server_name,
                attempt,
                delay_ms = delay.as_millis() as u64,
                "scheduling reconnect"
            );
            tokio::time::sleep(delay).await;

            match connect_single_server(&config, auth_store.as_ref()).await {
                Ok(built) => {
                    info!(server = %server_name, attempt, "MCP server reconnected");
                    self.install_server(server_name.to_string(), config.clone(), built)
                        .await;
                    self.emit_event(McpEvent::ServerReconnected {
                        server_name: server_name.to_string(),
                    })
                    .await;
                    return;
                }
                Err(e) => {
                    warn!(
                        server = %server_name,
                        attempt,
                        error = %e,
                        "reconnect attempt failed"
                    );
                }
            }
        }

        let mut servers = self.servers.write().await;
        if let Some(state) = servers.get_mut(server_name) {
            state.status = McpServerStatus::Failed {
                error: format!("reconnect exhausted after {max_retries} attempts"),
            };
            state.tools.clear();
            state.resources.clear();
            state.prompts.clear();
        }
        drop(servers);
        self.emit_event(McpEvent::ServerDisconnected {
            server_name: server_name.to_string(),
        })
        .await;
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
    pub async fn list_all_tools(&self) -> Vec<(String, McpToolInfo)> {
        self.collect_from_connected(|s| &s.tools).await
    }

    /// List all resources across all connected servers.
    pub async fn list_all_resources(&self) -> Vec<(String, McpResource)> {
        self.collect_from_connected(|s| &s.resources).await
    }

    /// List all prompts across all connected servers.
    pub async fn list_all_prompts(&self) -> Vec<(String, McpPrompt)> {
        self.collect_from_connected(|s| &s.prompts).await
    }

    /// Collect server instructions from all connected servers that provide them.
    pub async fn collect_server_instructions(&self) -> Vec<(String, String)> {
        let servers = self.servers.read().await;
        let mut result = Vec::new();
        for (name, state) in servers.iter() {
            if state.status == McpServerStatus::Connected {
                if let Some(ref instructions) = state.instructions {
                    result.push((name.clone(), instructions.clone()));
                }
            }
        }
        result
    }

    async fn collect_from_connected<T: Clone>(
        &self,
        field: impl Fn(&ServerState) -> &Vec<T>,
    ) -> Vec<(String, T)> {
        let servers = self.servers.read().await;
        let mut result = Vec::new();
        for (name, state) in servers.iter() {
            if state.status == McpServerStatus::Connected {
                for item in field(state) {
                    result.push((name.clone(), item.clone()));
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
        let client = self.connected_client(server_name).await?;
        client.call_tool(tool_name, arguments).await
    }

    /// Read a resource from a specific server.
    pub async fn read_resource(
        &self,
        server_name: &str,
        uri: &str,
    ) -> Result<ReadResourceResult, McpError> {
        let client = self.connected_client(server_name).await?;
        client.read_resource(uri).await
    }

    /// Retrieve a prompt from a specific server.
    pub async fn get_prompt(
        &self,
        server_name: &str,
        prompt_name: &str,
        arguments: HashMap<String, String>,
    ) -> Result<GetPromptResult, McpError> {
        let client = self.connected_client(server_name).await?;
        client.get_prompt(prompt_name, arguments).await
    }

    async fn connected_client(&self, server_name: &str) -> Result<Arc<McpClient>, McpError> {
        let servers = self.servers.read().await;
        let state = servers.get(server_name).ok_or_else(|| McpError::Other {
            message: format!("unknown MCP server: {server_name}"),
        })?;
        if state.status != McpServerStatus::Connected {
            return Err(McpError::ConnectionFailed {
                message: format!("server '{server_name}' is not connected: {}", state.status),
            });
        }
        Ok(Arc::clone(&state.client))
    }

    /// Refresh the tool list for a specific server.
    pub async fn refresh_tools(&self, server_name: &str) -> Result<usize, McpError> {
        let client = self.connected_client(server_name).await?;
        let tools = client.list_tools().await?;
        let count = tools.len();
        info!(server = %server_name, tool_count = count, "refreshed MCP tool list");
        let mut servers = self.servers.write().await;
        if let Some(state) = servers.get_mut(server_name) {
            state.tools = tools;
        }
        Ok(count)
    }

    /// Refresh the resource list for a specific server.
    pub async fn refresh_resources(&self, server_name: &str) -> Result<usize, McpError> {
        let client = self.connected_client(server_name).await?;
        let resources = client.list_resources().await?;
        let count = resources.len();
        info!(server = %server_name, resource_count = count, "refreshed MCP resource list");
        let mut servers = self.servers.write().await;
        if let Some(state) = servers.get_mut(server_name) {
            state.resources = resources;
        }
        Ok(count)
    }

    /// Refresh the prompt list for a specific server.
    pub async fn refresh_prompts(&self, server_name: &str) -> Result<usize, McpError> {
        let client = self.connected_client(server_name).await?;
        let prompts = client.list_prompts().await?;
        let count = prompts.len();
        info!(server = %server_name, prompt_count = count, "refreshed MCP prompt list");
        let mut servers = self.servers.write().await;
        if let Some(state) = servers.get_mut(server_name) {
            state.prompts = prompts;
        }
        Ok(count)
    }

    /// Disconnect a specific server.
    pub async fn disconnect(&self, server_name: &str) -> Result<(), McpError> {
        let mut servers = self.servers.write().await;
        if let Some(state) = servers.get_mut(server_name) {
            state.client.close().await?;
            state.status = McpServerStatus::Disconnected;
            state.tools.clear();
            state.resources.clear();
            state.prompts.clear();
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
                state.resources.clear();
                state.prompts.clear();
            }
        }
        // Abort supervisors.
        let mut sups = self.supervisors.write().await;
        for h in sups.drain(..) {
            h.abort();
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

/// Bundle of artifacts returned by `connect_single_server`.
struct ConnectedServer {
    client: Arc<McpClient>,
    tools: Vec<McpToolInfo>,
    resources: Vec<McpResource>,
    prompts: Vec<McpPrompt>,
    instructions: Option<String>,
    disconnect_rx: Option<mpsc::UnboundedReceiver<()>>,
}

/// Connect to a single MCP server and probe its advertised capabilities.
async fn connect_single_server(
    config: &McpServerConfigRef,
    auth_store: Option<&McpAuthStore>,
) -> Result<ConnectedServer, McpError> {
    let mut disconnect_rx: Option<mpsc::UnboundedReceiver<()>> = None;

    let transport: Arc<dyn McpTransport> = match config.transport.as_str() {
        "stdio" => {
            let command = config.command.as_deref().ok_or_else(|| McpError::Other {
                message: "stdio transport requires a 'command' field".into(),
            })?;
            let stdio =
                StdioTransport::spawn(command, &config.args, &config.env, config.cwd.as_deref())?;
            // Wire up disconnect signal channel for supervisor.
            let (tx, rx) = mpsc::unbounded_channel();
            stdio.set_disconnect_signal(tx);
            disconnect_rx = Some(rx);
            Arc::new(stdio)
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
    let caps = client.initialize().await?;

    let tools = if caps.supports_tools() {
        client.list_tools().await.unwrap_or_else(|e| {
            warn!(server = %config.name, error = %e, "failed to list tools");
            Vec::new()
        })
    } else {
        Vec::new()
    };

    let resources = if caps.supports_resources() {
        client.list_resources().await.unwrap_or_else(|e| {
            warn!(server = %config.name, error = %e, "failed to list resources");
            Vec::new()
        })
    } else {
        Vec::new()
    };

    let prompts = if caps.supports_prompts() {
        client.list_prompts().await.unwrap_or_else(|e| {
            warn!(server = %config.name, error = %e, "failed to list prompts");
            Vec::new()
        })
    } else {
        Vec::new()
    };

    let instructions = caps.instructions.clone();

    debug!(
        server = %config.name,
        tools = tools.len(),
        resources = resources.len(),
        prompts = prompts.len(),
        has_instructions = instructions.is_some(),
        "discovered MCP capabilities"
    );

    // Silence the unused-value warning when tools-only servers are used.
    let _ = caps;

    Ok(ConnectedServer {
        client,
        tools,
        resources,
        prompts,
        instructions,
        disconnect_rx,
    })
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

/// Unified notification dispatcher: routes `list_changed` notifications to the
/// appropriate refresh method on the manager.
fn notification_dispatcher(
    manager: Arc<McpConnectionManager>,
    server_name: String,
) -> NotificationHandler {
    Arc::new(move |method: &str, _params: Option<serde_json::Value>| {
        let mgr = Arc::clone(&manager);
        let name = server_name.clone();
        let method = method.to_string();
        tokio::spawn(async move {
            let outcome: Result<(), McpError> = match method.as_str() {
                "notifications/tools/list_changed" => mgr.refresh_tools(&name).await.map(|_| ()),
                "notifications/resources/list_changed" => {
                    mgr.refresh_resources(&name).await.map(|_| ())
                }
                "notifications/prompts/list_changed" => {
                    mgr.refresh_prompts(&name).await.map(|_| ())
                }
                _ => {
                    debug!(server = %name, method = %method, "ignoring MCP notification");
                    return;
                }
            };
            if let Err(e) = outcome {
                warn!(
                    server = %name,
                    method = %method,
                    error = %e,
                    "failed to handle MCP notification"
                );
            }
        });
    })
}

/// Backward-compatible wrapper for callers that only need tool refresh.
pub fn tool_list_changed_handler(
    manager: Arc<McpConnectionManager>,
    server_name: String,
) -> NotificationHandler {
    notification_dispatcher(manager, server_name)
}

/// Probe the manager-public helper so external callers can derive one.
pub fn build_notification_dispatcher(
    manager: Arc<McpConnectionManager>,
    server_name: String,
) -> NotificationHandler {
    notification_dispatcher(manager, server_name)
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
        assert_eq!(
            McpServerStatus::Reconnecting {
                attempt: 2,
                next_delay_ms: 4000
            }
            .to_string(),
            "reconnecting (attempt 2, next in 4000ms)"
        );
    }

    #[test]
    fn test_reconnect_policy_backoff() {
        let policy = ReconnectPolicy {
            enabled: true,
            max_retries: 5,
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_millis(1000),
            multiplier: 2.0,
        };
        assert_eq!(policy.backoff_delay(1), Duration::from_millis(100));
        assert_eq!(policy.backoff_delay(2), Duration::from_millis(200));
        assert_eq!(policy.backoff_delay(3), Duration::from_millis(400));
        assert_eq!(policy.backoff_delay(4), Duration::from_millis(800));
        // Capped at max_delay.
        assert_eq!(policy.backoff_delay(5), Duration::from_millis(1000));
        assert_eq!(policy.backoff_delay(99), Duration::from_millis(1000));
    }

    #[test]
    fn test_config_ref_default() {
        let config = McpServerConfigRef::default();
        assert!(config.auto_reconnect);
        assert_eq!(config.max_reconnect_attempts, 5);
    }

    #[tokio::test]
    async fn test_empty_manager() {
        let manager = McpConnectionManager::new(None);
        assert_eq!(manager.connected_count().await, 0);
        assert!(manager.status().await.is_empty());
        assert!(manager.list_all_tools().await.is_empty());
        assert!(manager.list_all_resources().await.is_empty());
        assert!(manager.list_all_prompts().await.is_empty());
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
        let manager = Arc::new(McpConnectionManager::new(None));
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
            auto_reconnect: false,
            max_reconnect_attempts: 5,
        };

        manager.connect_all(vec![config]).await;

        let status = manager.server_status("bad-server").await;
        assert!(matches!(status, Some(McpServerStatus::Failed { .. })));
        assert_eq!(manager.connected_count().await, 0);
    }
}
