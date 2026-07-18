use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::RwLock as StdRwLock;

use serde_json::{json, Value};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use y_core::tool::{ToolError, ToolOutput};
use y_core::types::SessionId;

use crate::lsp::{LspClient, LspClientError, LspConfig, LspConnector, LspServerConfig};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ClientKey {
    session_id: SessionId,
    server_id: String,
    project_root: PathBuf,
}

#[derive(Debug, Clone, Copy)]
enum Operation {
    Definition,
    References,
    Hover,
    DocumentSymbols,
    WorkspaceSymbols,
    Diagnostics,
}

impl Operation {
    fn from_tool_name(tool_name: &str) -> Result<Self, ToolError> {
        match tool_name {
            "LspDefinition" => Ok(Self::Definition),
            "LspReferences" => Ok(Self::References),
            "LspHover" => Ok(Self::Hover),
            "LspDocumentSymbols" => Ok(Self::DocumentSymbols),
            "LspWorkspaceSymbols" => Ok(Self::WorkspaceSymbols),
            "LspDiagnostics" => Ok(Self::Diagnostics),
            _ => Err(ToolError::NotFound {
                name: tool_name.to_string(),
            }),
        }
    }

    fn method(self) -> &'static str {
        match self {
            Self::Definition => "textDocument/definition",
            Self::References => "textDocument/references",
            Self::Hover => "textDocument/hover",
            Self::DocumentSymbols => "textDocument/documentSymbol",
            Self::WorkspaceSymbols => "workspace/symbol",
            Self::Diagnostics => "textDocument/diagnostic",
        }
    }

    fn requires_document(self) -> bool {
        !matches!(self, Self::WorkspaceSymbols)
    }
}

/// Service-owned LSP client pool and tool dispatcher.
pub struct LspManager {
    config: LspConfig,
    dynamic_servers: StdRwLock<HashMap<String, LspServerConfig>>,
    connector: Arc<dyn LspConnector>,
    clients: Mutex<HashMap<ClientKey, Arc<Mutex<LspClient>>>>,
}

impl LspManager {
    pub fn new(config: LspConfig, connector: Arc<dyn LspConnector>) -> Self {
        Self {
            config,
            dynamic_servers: StdRwLock::new(HashMap::new()),
            connector,
            clients: Mutex::new(HashMap::new()),
        }
    }

    pub async fn execute(
        &self,
        tool_name: &str,
        arguments: &Value,
        session_id: &SessionId,
        working_dir: Option<&str>,
        additional_read_dirs: &[String],
    ) -> Result<ToolOutput, ToolError> {
        self.execute_with_cancellation(
            tool_name,
            arguments,
            session_id,
            working_dir,
            additional_read_dirs,
            None,
        )
        .await
    }

    pub async fn execute_with_cancellation(
        &self,
        tool_name: &str,
        arguments: &Value,
        session_id: &SessionId,
        working_dir: Option<&str>,
        additional_read_dirs: &[String],
        cancellation: Option<&CancellationToken>,
    ) -> Result<ToolOutput, ToolError> {
        if !self.config.enabled {
            return Err(ToolError::RuntimeError {
                name: tool_name.to_string(),
                message: "LSP support is disabled by service configuration".to_string(),
            });
        }
        let operation = Operation::from_tool_name(tool_name)?;
        let prepared = if operation.requires_document() {
            self.prepare_document_operation(
                operation,
                arguments,
                session_id,
                working_dir,
                additional_read_dirs,
            )
            .await?
        } else {
            self.prepare_workspace_operation(
                arguments,
                session_id,
                working_dir,
                additional_read_dirs,
            )
            .await?
        };
        let mut client = prepared.client.lock().await;
        if let Some(document) = prepared.document {
            client
                .open_document(&document.path, document.text, 1)
                .await
                .map_err(|error| client_error(tool_name, &error))?;
        }
        let result = if let Some(cancellation) = cancellation {
            client
                .request_with_cancellation(operation.method(), prepared.params, cancellation)
                .await
        } else {
            client.request(operation.method(), prepared.params).await
        }
        .map_err(|error| client_error(tool_name, &error))?;
        Ok(ToolOutput {
            success: true,
            content: result,
            warnings: Vec::new(),
            metadata: json!({
                "server_id": prepared.server_id,
                "project_root": prepared.project_root,
                "method": operation.method(),
            }),
        })
    }

    async fn prepare_document_operation(
        &self,
        operation: Operation,
        arguments: &Value,
        session_id: &SessionId,
        working_dir: Option<&str>,
        additional_read_dirs: &[String],
    ) -> Result<PreparedOperation, ToolError> {
        let path = required_string(arguments, "path")?;
        let (path, trusted_root) =
            resolve_trusted_path(path, working_dir, additional_read_dirs, false)?;
        let server = self
            .server_for_path(&path)
            .ok_or_else(|| ToolError::ValidationError {
                message: format!("no configured language server matches {}", path.display()),
            })?;
        let project_root = server
            .project_root_within(&path, &trusted_root)
            .ok_or_else(|| ToolError::PermissionDenied {
                name: operation.method().to_string(),
                reason: "project root would escape the trusted workspace".to_string(),
            })?;
        let text =
            tokio::fs::read_to_string(&path)
                .await
                .map_err(|error| ToolError::RuntimeError {
                    name: operation.method().to_string(),
                    message: format!("failed to read {}: {error}", path.display()),
                })?;
        let uri = file_uri(&path)?;
        let params = document_params(operation, arguments, &uri)?;
        let client = self.client_for(session_id, &server, &project_root).await;
        Ok(PreparedOperation {
            client,
            server_id: server.id,
            project_root,
            params,
            document: Some(PreparedDocument { path, text }),
        })
    }

    async fn prepare_workspace_operation(
        &self,
        arguments: &Value,
        session_id: &SessionId,
        working_dir: Option<&str>,
        additional_read_dirs: &[String],
    ) -> Result<PreparedOperation, ToolError> {
        let requested_dir = arguments
            .get("working_directory")
            .and_then(Value::as_str)
            .or(working_dir)
            .ok_or_else(|| ToolError::ValidationError {
                message: "working_directory is required for LspWorkspaceSymbols".to_string(),
            })?;
        let (directory, trusted_root) =
            resolve_trusted_path(requested_dir, working_dir, additional_read_dirs, true)?;
        let server = self.select_workspace_server(arguments, &directory)?;
        let project_root = server
            .project_root_within(&directory, &trusted_root)
            .ok_or_else(|| ToolError::PermissionDenied {
                name: "LspWorkspaceSymbols".to_string(),
                reason: "project root would escape the trusted workspace".to_string(),
            })?;
        let query = required_string(arguments, "query")?;
        let client = self.client_for(session_id, &server, &project_root).await;
        Ok(PreparedOperation {
            client,
            server_id: server.id,
            project_root,
            params: json!({"query": query}),
            document: None,
        })
    }

    async fn client_for(
        &self,
        session_id: &SessionId,
        server: &LspServerConfig,
        project_root: &Path,
    ) -> Arc<Mutex<LspClient>> {
        let key = ClientKey {
            session_id: session_id.clone(),
            server_id: server.id.clone(),
            project_root: project_root.to_path_buf(),
        };
        let mut clients = self.clients.lock().await;
        Arc::clone(clients.entry(key).or_insert_with(|| {
            Arc::new(Mutex::new(LspClient::new(
                session_id.clone(),
                server.clone(),
                project_root.to_path_buf(),
                &self.config,
                Arc::clone(&self.connector),
            )))
        }))
    }

    pub async fn cleanup_session(&self, session_id: &SessionId) {
        let clients = {
            let mut clients = self.clients.lock().await;
            let keys = clients
                .keys()
                .filter(|key| &key.session_id == session_id)
                .cloned()
                .collect::<Vec<_>>();
            keys.into_iter()
                .filter_map(|key| clients.remove(&key))
                .collect::<Vec<_>>()
        };
        for client in clients {
            client.lock().await.shutdown().await;
        }
    }

    pub fn register_dynamic_server(&self, server: LspServerConfig) -> Result<bool, String> {
        if self
            .config
            .servers
            .iter()
            .any(|existing| existing.id == server.id)
        {
            return Err(format!(
                "dynamic LSP server conflicts with user configuration: {}",
                server.id
            ));
        }
        if server.command.trim().is_empty() {
            return Err(format!(
                "dynamic LSP server requires command: {}",
                server.id
            ));
        }
        let mut servers = self
            .dynamic_servers
            .write()
            .map_err(|_| "dynamic LSP server lock is poisoned".to_string())?;
        if servers.contains_key(&server.id) {
            return Ok(false);
        }
        servers.insert(server.id.clone(), server);
        Ok(true)
    }

    pub async fn unregister_dynamic_server(&self, server_id: &str) -> Result<bool, String> {
        let removed = self
            .dynamic_servers
            .write()
            .map_err(|_| "dynamic LSP server lock is poisoned".to_string())?
            .remove(server_id)
            .is_some();
        if !removed {
            return Ok(false);
        }
        let clients = {
            let mut clients = self.clients.lock().await;
            let keys = clients
                .keys()
                .filter(|key| key.server_id == server_id)
                .cloned()
                .collect::<Vec<_>>();
            keys.into_iter()
                .filter_map(|key| clients.remove(&key))
                .collect::<Vec<_>>()
        };
        for client in clients {
            client.lock().await.shutdown().await;
        }
        Ok(true)
    }

    pub fn has_dynamic_server(&self, server_id: &str) -> bool {
        self.dynamic_servers
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains_key(server_id)
    }

    pub fn has_configured_server(&self, server_id: &str) -> bool {
        self.config
            .servers
            .iter()
            .any(|server| server.id == server_id)
    }

    fn server_for_path(&self, path: &Path) -> Option<LspServerConfig> {
        if let Some(server) = self.config.server_for_path(path) {
            return Some(server.clone());
        }
        let dynamic = self.dynamic_config();
        dynamic.server_for_path(path).cloned()
    }

    fn select_workspace_server(
        &self,
        arguments: &Value,
        directory: &Path,
    ) -> Result<LspServerConfig, ToolError> {
        if let Some(language) = arguments.get("language").and_then(Value::as_str) {
            if let Some(server) = self
                .config
                .servers
                .iter()
                .find(|server| server.id == language || server.language_id == language)
            {
                return Ok(server.clone());
            }
            return self
                .dynamic_servers_sorted()
                .into_iter()
                .find(|server| server.id == language || server.language_id == language)
                .ok_or_else(|| ToolError::ValidationError {
                    message: format!("no configured language server matches '{language}'"),
                });
        }
        if let Some(server) = self.config.servers.iter().find(|server| {
            server
                .root_markers
                .iter()
                .any(|marker| directory.join(marker).exists())
        }) {
            return Ok(server.clone());
        }
        let dynamic = self.dynamic_servers_sorted();
        if let Some(server) = dynamic.iter().find(|server| {
            server
                .root_markers
                .iter()
                .any(|marker| directory.join(marker).exists())
        }) {
            return Ok(server.clone());
        }
        self.config
            .servers
            .first()
            .cloned()
            .or_else(|| dynamic.into_iter().next())
            .ok_or_else(|| ToolError::ValidationError {
                message: "no language servers are configured".to_string(),
            })
    }

    fn dynamic_config(&self) -> LspConfig {
        LspConfig {
            enabled: self.config.enabled,
            servers: self.dynamic_servers_sorted(),
            ..self.config.clone()
        }
    }

    fn dynamic_servers_sorted(&self) -> Vec<LspServerConfig> {
        let mut servers = self
            .dynamic_servers
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .cloned()
            .collect::<Vec<_>>();
        servers.sort_by(|left, right| left.id.cmp(&right.id));
        servers
    }
}

struct PreparedOperation {
    client: Arc<Mutex<LspClient>>,
    server_id: String,
    project_root: PathBuf,
    params: Value,
    document: Option<PreparedDocument>,
}

struct PreparedDocument {
    path: PathBuf,
    text: String,
}

fn document_params(operation: Operation, arguments: &Value, uri: &str) -> Result<Value, ToolError> {
    let text_document = json!({"uri": uri});
    match operation {
        Operation::Definition | Operation::Hover => Ok(json!({
            "textDocument": text_document,
            "position": required_position(arguments)?,
        })),
        Operation::References => Ok(json!({
            "textDocument": text_document,
            "position": required_position(arguments)?,
            "context": {
                "includeDeclaration": arguments
                    .get("include_declaration")
                    .and_then(Value::as_bool)
                    .unwrap_or(true),
            },
        })),
        Operation::DocumentSymbols | Operation::Diagnostics => {
            Ok(json!({"textDocument": text_document}))
        }
        Operation::WorkspaceSymbols => Err(ToolError::ValidationError {
            message: "workspace symbols do not accept a document path".to_string(),
        }),
    }
}

fn required_position(arguments: &Value) -> Result<Value, ToolError> {
    let line = required_u64(arguments, "line")?;
    let character = required_u64(arguments, "character")?;
    Ok(json!({"line": line, "character": character}))
}

fn required_string<'a>(arguments: &'a Value, field: &str) -> Result<&'a str, ToolError> {
    arguments
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| ToolError::ValidationError {
            message: format!("'{field}' must be a non-empty string"),
        })
}

fn required_u64(arguments: &Value, field: &str) -> Result<u64, ToolError> {
    arguments
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| ToolError::ValidationError {
            message: format!("'{field}' must be a non-negative integer"),
        })
}

fn resolve_trusted_path(
    requested: &str,
    working_dir: Option<&str>,
    additional_read_dirs: &[String],
    require_directory: bool,
) -> Result<(PathBuf, PathBuf), ToolError> {
    let working_dir = working_dir.ok_or_else(|| ToolError::PermissionDenied {
        name: "LSP".to_string(),
        reason: "a trusted working directory is required".to_string(),
    })?;
    let base = Path::new(working_dir);
    let candidate = if Path::new(requested).is_absolute() {
        PathBuf::from(requested)
    } else {
        base.join(requested)
    };
    let canonical = std::fs::canonicalize(&candidate).map_err(|_| ToolError::FileNotFound {
        path: candidate.display().to_string(),
    })?;
    if require_directory && !canonical.is_dir() {
        return Err(ToolError::ValidationError {
            message: format!("{} is not a directory", canonical.display()),
        });
    }
    if !require_directory && !canonical.is_file() {
        return Err(ToolError::ValidationError {
            message: format!("{} is not a file", canonical.display()),
        });
    }

    let mut roots = Vec::with_capacity(additional_read_dirs.len() + 1);
    roots.push(PathBuf::from(working_dir));
    roots.extend(additional_read_dirs.iter().map(PathBuf::from));
    let trusted_root = roots
        .into_iter()
        .filter_map(|root| std::fs::canonicalize(root).ok())
        .filter(|root| canonical.starts_with(root))
        .max_by_key(|root| root.components().count())
        .ok_or_else(|| ToolError::PermissionDenied {
            name: "LSP".to_string(),
            reason: format!("{} is outside trusted read roots", canonical.display()),
        })?;
    Ok((canonical, trusted_root))
}

fn file_uri(path: &Path) -> Result<String, ToolError> {
    url::Url::from_file_path(path)
        .map_err(|()| ToolError::ValidationError {
            message: format!("invalid source path: {}", path.display()),
        })
        .map(|url| url.to_string())
}

fn client_error(tool_name: &str, error: &LspClientError) -> ToolError {
    if matches!(error, LspClientError::Cancelled) {
        return ToolError::Cancelled;
    }
    ToolError::ExternalServiceError {
        name: tool_name.to_string(),
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use async_trait::async_trait;
    use serde_json::{json, Value};
    use y_core::tool::ToolError;
    use y_core::types::SessionId;

    use super::LspManager;
    use crate::lsp::{LspClientError, LspConfig, LspConnection, LspConnector, LspServerConfig};

    struct FakeConnector {
        connections: AtomicUsize,
        methods: Arc<Mutex<Vec<String>>>,
    }

    struct FakeConnection {
        methods: Arc<Mutex<Vec<String>>>,
        last_request: Option<(Value, String)>,
    }

    #[async_trait]
    impl LspConnector for FakeConnector {
        async fn connect(
            &self,
            _session_id: &SessionId,
            _server: &LspServerConfig,
            _project_root: &Path,
            _max_message_bytes: usize,
            _request_timeout: Duration,
        ) -> Result<Box<dyn LspConnection>, LspClientError> {
            self.connections.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(FakeConnection {
                methods: Arc::clone(&self.methods),
                last_request: None,
            }))
        }
    }

    #[async_trait]
    impl LspConnection for FakeConnection {
        async fn send(&mut self, message: &Value) -> Result<(), LspClientError> {
            let method = message["method"].as_str().unwrap_or("response").to_string();
            self.methods.lock().expect("methods").push(method.clone());
            if let Some(id) = message.get("id") {
                self.last_request = Some((id.clone(), method));
            }
            Ok(())
        }

        async fn receive(&mut self) -> Result<Value, LspClientError> {
            let (id, method) = self
                .last_request
                .take()
                .ok_or_else(|| LspClientError::Protocol("missing request".into()))?;
            let result = if method == "initialize" {
                json!({})
            } else {
                json!([{"uri": "file:///workspace/src/lib.rs", "range": {}}])
            };
            Ok(json!({"jsonrpc": "2.0", "id": id, "result": result}))
        }

        async fn close(&mut self) -> Result<(), LspClientError> {
            Ok(())
        }
    }

    fn manager() -> (LspManager, Arc<FakeConnector>) {
        let connector = Arc::new(FakeConnector {
            connections: AtomicUsize::new(0),
            methods: Arc::new(Mutex::new(Vec::new())),
        });
        let config = LspConfig {
            enabled: true,
            restart_base_delay_ms: 0,
            servers: vec![LspServerConfig {
                id: "rust".into(),
                command: "fake-rust-analyzer".into(),
                language_id: "rust".into(),
                extensions: vec!["rs".into()],
                root_markers: vec!["Cargo.toml".into()],
                ..LspServerConfig::default()
            }],
            ..LspConfig::default()
        };
        (LspManager::new(config, connector.clone()), connector)
    }

    #[test]
    fn dynamic_servers_extend_but_do_not_replace_user_server_selection() {
        let (manager, _) = manager();
        manager
            .register_dynamic_server(LspServerConfig {
                id: "pack-language".into(),
                command: "pack-language-server".into(),
                language_id: "pack-language".into(),
                extensions: vec!["pack".into(), "rs".into()],
                ..LspServerConfig::default()
            })
            .expect("register dynamic server");

        assert_eq!(
            manager
                .server_for_path(Path::new("/workspace/src/lib.rs"))
                .expect("user rust server")
                .id,
            "rust"
        );
        assert_eq!(
            manager
                .server_for_path(Path::new("/workspace/src/file.pack"))
                .expect("pack server")
                .id,
            "pack-language"
        );
        assert!(manager.has_dynamic_server("pack-language"));
    }

    #[tokio::test]
    async fn definition_executes_through_the_project_client() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let workspace = temp.path().join("workspace");
        let source = workspace.join("src");
        std::fs::create_dir_all(&source).expect("source dirs");
        std::fs::write(workspace.join("Cargo.toml"), "[package]").expect("marker");
        let file = source.join("lib.rs");
        std::fs::write(&file, "pub struct Thing;").expect("source");
        let (manager, connector) = manager();

        let output = manager
            .execute(
                "LspDefinition",
                &json!({"path": file, "line": 0, "character": 11}),
                &SessionId::from_string("session-lsp"),
                Some(&workspace.to_string_lossy()),
                &[],
            )
            .await
            .expect("definition output");

        assert!(output.success);
        assert_eq!(output.content[0]["uri"], "file:///workspace/src/lib.rs");
        assert_eq!(output.metadata["server_id"], "rust");
        assert_eq!(connector.connections.load(Ordering::SeqCst), 1);
        assert!(connector
            .methods
            .lock()
            .expect("methods")
            .iter()
            .any(|method| method == "textDocument/definition"));
    }

    #[tokio::test]
    async fn source_paths_outside_trusted_roots_are_rejected_before_connect() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let workspace = temp.path().join("workspace");
        let outside = temp.path().join("outside.rs");
        std::fs::create_dir_all(&workspace).expect("workspace");
        std::fs::write(&outside, "pub struct Outside;").expect("outside source");
        let (manager, connector) = manager();

        let error = manager
            .execute(
                "LspDefinition",
                &json!({"path": outside, "line": 0, "character": 11}),
                &SessionId::from_string("session-lsp"),
                Some(&workspace.to_string_lossy()),
                &[],
            )
            .await
            .expect_err("untrusted path");

        assert!(matches!(error, ToolError::PermissionDenied { .. }));
        assert_eq!(connector.connections.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn cancelled_client_errors_remain_tool_cancellations() {
        let error = super::client_error("LspDefinition", &LspClientError::Cancelled);

        assert!(matches!(error, ToolError::Cancelled));
    }
}
