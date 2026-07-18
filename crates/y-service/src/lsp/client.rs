use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;
use y_core::types::SessionId;

use crate::lsp::{LspConfig, LspServerConfig};

#[derive(Debug, thiserror::Error)]
pub enum LspClientError {
    #[error("language-server transport error: {0}")]
    Transport(String),
    #[error("language-server protocol error: {0}")]
    Protocol(String),
    #[error("language server returned error {code}: {message}")]
    Server { code: i64, message: String },
    #[error("language-server request cancelled")]
    Cancelled,
    #[error("language-server restart budget exhausted after {attempts} attempts: {last_error}")]
    RestartExhausted { attempts: u32, last_error: String },
}

#[async_trait]
pub trait LspConnection: Send {
    async fn send(&mut self, message: &Value) -> Result<(), LspClientError>;
    async fn receive(&mut self) -> Result<Value, LspClientError>;
    async fn close(&mut self) -> Result<(), LspClientError>;
}

#[async_trait]
pub trait LspConnector: Send + Sync {
    async fn connect(
        &self,
        session_id: &SessionId,
        server: &LspServerConfig,
        project_root: &Path,
        max_message_bytes: usize,
        request_timeout: Duration,
    ) -> Result<Box<dyn LspConnection>, LspClientError>;
}

#[derive(Debug, Clone)]
struct TrackedDocument {
    uri: String,
    language_id: String,
    version: i64,
    text: String,
}

/// One sequential, restartable language-server connection for a session and project.
pub struct LspClient {
    session_id: SessionId,
    server: LspServerConfig,
    project_root: PathBuf,
    connector: Arc<dyn LspConnector>,
    connection: Option<Box<dyn LspConnection>>,
    documents: BTreeMap<PathBuf, TrackedDocument>,
    next_request_id: u64,
    request_timeout: Duration,
    max_message_bytes: usize,
    max_restarts: u32,
    restart_base_delay: Duration,
}

impl LspClient {
    pub fn new(
        session_id: SessionId,
        server: LspServerConfig,
        project_root: PathBuf,
        config: &LspConfig,
        connector: Arc<dyn LspConnector>,
    ) -> Self {
        Self {
            session_id,
            server,
            project_root,
            connector,
            connection: None,
            documents: BTreeMap::new(),
            next_request_id: 1,
            request_timeout: Duration::from_millis(config.request_timeout_ms),
            max_message_bytes: config.max_message_bytes,
            max_restarts: config.max_restarts,
            restart_base_delay: Duration::from_millis(config.restart_base_delay_ms),
        }
    }

    pub async fn open_document(
        &mut self,
        path: &Path,
        text: String,
        version: i64,
    ) -> Result<(), LspClientError> {
        let uri = url::Url::from_file_path(path)
            .map_err(|()| {
                LspClientError::Protocol(format!("invalid file path: {}", path.display()))
            })?
            .to_string();
        let previous_version = self.documents.get(path).map(|document| document.version);
        let existed = previous_version.is_some();
        let version = next_document_version(previous_version, version);
        self.documents.insert(
            path.to_path_buf(),
            TrackedDocument {
                uri,
                language_id: self.server.language_id.clone(),
                version,
                text,
            },
        );

        self.run_with_restarts(
            if existed {
                ClientAction::ChangeDocument(path.to_path_buf())
            } else {
                ClientAction::OpenDocument(path.to_path_buf())
            },
            None,
        )
        .await
        .map(|_| ())
    }

    pub async fn request(&mut self, method: &str, params: Value) -> Result<Value, LspClientError> {
        self.run_with_restarts(
            ClientAction::Request {
                method: method.to_string(),
                params,
            },
            None,
        )
        .await
    }

    pub async fn request_with_cancellation(
        &mut self,
        method: &str,
        params: Value,
        cancellation: &CancellationToken,
    ) -> Result<Value, LspClientError> {
        self.run_with_restarts(
            ClientAction::Request {
                method: method.to_string(),
                params,
            },
            Some(cancellation),
        )
        .await
    }

    /// Gracefully terminate the language-server protocol, then force transport cleanup.
    pub async fn shutdown(&mut self) {
        let request_id = self.allocate_request_id();
        if let Some(connection) = self.connection.as_deref_mut() {
            if send_request(connection, request_id, "shutdown", Value::Null, None)
                .await
                .is_ok()
            {
                let _ = send_notification(connection, "exit", Value::Null).await;
            }
        }
        self.discard_connection().await;
    }

    async fn run_with_restarts(
        &mut self,
        action: ClientAction,
        cancellation: Option<&CancellationToken>,
    ) -> Result<Value, LspClientError> {
        for attempt in 0..=self.max_restarts {
            let connected_before_attempt = self.connection.is_some();
            let outcome = match self.ensure_connected().await {
                Ok(()) => {
                    if !connected_before_attempt
                        && matches!(
                            action,
                            ClientAction::OpenDocument(_) | ClientAction::ChangeDocument(_)
                        )
                    {
                        Ok(Value::Null)
                    } else {
                        self.execute_action(&action, cancellation).await
                    }
                }
                Err(error) => Err(error),
            };
            match outcome {
                Ok(value) => return Ok(value),
                Err(error) if !error.is_restartable() => return Err(error),
                Err(_error) if attempt < self.max_restarts => {
                    self.discard_connection().await;
                    tokio::time::sleep(restart_delay(self.restart_base_delay, attempt)).await;
                }
                Err(error) => {
                    self.discard_connection().await;
                    return Err(LspClientError::RestartExhausted {
                        attempts: attempt + 1,
                        last_error: error.to_string(),
                    });
                }
            }
        }
        unreachable!("restart loop always returns")
    }

    async fn ensure_connected(&mut self) -> Result<(), LspClientError> {
        if self.connection.is_some() {
            return Ok(());
        }
        let mut connection = self
            .connector
            .connect(
                &self.session_id,
                &self.server,
                &self.project_root,
                self.max_message_bytes,
                self.request_timeout,
            )
            .await?;

        let initialize_id = self.allocate_request_id();
        send_request(
            connection.as_mut(),
            initialize_id,
            "initialize",
            json!({
                "processId": Value::Null,
                "rootUri": project_root_uri(&self.project_root)?,
                "capabilities": {},
                "initializationOptions": self.server.initialization_options,
            }),
            None,
        )
        .await?;
        send_notification(connection.as_mut(), "initialized", json!({})).await?;
        for document in self.documents.values() {
            send_did_open(connection.as_mut(), document).await?;
        }
        self.connection = Some(connection);
        Ok(())
    }

    async fn execute_action(
        &mut self,
        action: &ClientAction,
        cancellation: Option<&CancellationToken>,
    ) -> Result<Value, LspClientError> {
        let request_id = if matches!(action, ClientAction::Request { .. }) {
            Some(self.allocate_request_id())
        } else {
            None
        };
        let connection = self
            .connection
            .as_deref_mut()
            .ok_or_else(|| LspClientError::Protocol("connection is not initialized".into()))?;
        match action {
            ClientAction::Request { method, params } => {
                send_request(
                    connection,
                    request_id.expect("request id allocated"),
                    method,
                    params.clone(),
                    cancellation,
                )
                .await
            }
            ClientAction::OpenDocument(path) => {
                let document = self.documents.get(path).ok_or_else(|| {
                    LspClientError::Protocol(format!("document is not tracked: {}", path.display()))
                })?;
                send_did_open(connection, document).await?;
                Ok(Value::Null)
            }
            ClientAction::ChangeDocument(path) => {
                let document = self.documents.get(path).ok_or_else(|| {
                    LspClientError::Protocol(format!("document is not tracked: {}", path.display()))
                })?;
                send_notification(
                    connection,
                    "textDocument/didChange",
                    json!({
                        "textDocument": {"uri": document.uri, "version": document.version},
                        "contentChanges": [{"text": document.text}],
                    }),
                )
                .await?;
                Ok(Value::Null)
            }
        }
    }

    fn allocate_request_id(&mut self) -> u64 {
        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.saturating_add(1);
        request_id
    }

    async fn discard_connection(&mut self) {
        if let Some(mut connection) = self.connection.take() {
            let _ = connection.close().await;
        }
    }
}

impl LspClientError {
    fn is_restartable(&self) -> bool {
        matches!(self, Self::Transport(_) | Self::Protocol(_))
    }
}

enum ClientAction {
    Request { method: String, params: Value },
    OpenDocument(PathBuf),
    ChangeDocument(PathBuf),
}

async fn send_request(
    connection: &mut dyn LspConnection,
    request_id: u64,
    method: &str,
    params: Value,
    cancellation: Option<&CancellationToken>,
) -> Result<Value, LspClientError> {
    connection
        .send(&json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "method": method,
            "params": params,
        }))
        .await?;
    loop {
        let outcome = if let Some(cancellation) = cancellation {
            let receive = connection.receive();
            tokio::select! {
                message = receive => ReceiveOutcome::Message(message),
                () = cancellation.cancelled() => ReceiveOutcome::Cancelled,
            }
        } else {
            ReceiveOutcome::Message(connection.receive().await)
        };
        let message = match outcome {
            ReceiveOutcome::Message(message) => message?,
            ReceiveOutcome::Cancelled => {
                send_notification(connection, "$/cancelRequest", json!({"id": request_id})).await?;
                return Err(LspClientError::Cancelled);
            }
        };
        if message.get("id") != Some(&json!(request_id)) {
            if message.get("id").is_some() && message.get("method").is_some() {
                answer_server_request(connection, &message).await?;
            }
            continue;
        }
        if let Some(error) = message.get("error") {
            return Err(LspClientError::Server {
                code: error["code"].as_i64().unwrap_or(-32_603),
                message: error["message"]
                    .as_str()
                    .unwrap_or("unknown language-server error")
                    .to_string(),
            });
        }
        return Ok(message.get("result").cloned().unwrap_or(Value::Null));
    }
}

enum ReceiveOutcome {
    Message(Result<Value, LspClientError>),
    Cancelled,
}

async fn answer_server_request(
    connection: &mut dyn LspConnection,
    request: &Value,
) -> Result<(), LspClientError> {
    let id = request
        .get("id")
        .cloned()
        .ok_or_else(|| LspClientError::Protocol("server request is missing id".into()))?;
    let method = request
        .get("method")
        .and_then(Value::as_str)
        .ok_or_else(|| LspClientError::Protocol("server request is missing method".into()))?;
    let response = match method {
        "workspace/configuration" => {
            let item_count = request["params"]["items"].as_array().map_or(0, Vec::len);
            json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": vec![Value::Null; item_count],
            })
        }
        "client/registerCapability"
        | "client/unregisterCapability"
        | "workspace/workspaceFolders" => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": Value::Null,
        }),
        _ => json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {"code": -32601, "message": format!("unsupported client method: {method}")},
        }),
    };
    connection.send(&response).await
}

async fn send_notification(
    connection: &mut dyn LspConnection,
    method: &str,
    params: Value,
) -> Result<(), LspClientError> {
    connection
        .send(&json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        }))
        .await
}

async fn send_did_open(
    connection: &mut dyn LspConnection,
    document: &TrackedDocument,
) -> Result<(), LspClientError> {
    send_notification(
        connection,
        "textDocument/didOpen",
        json!({
            "textDocument": {
                "uri": document.uri,
                "languageId": document.language_id,
                "version": document.version,
                "text": document.text,
            }
        }),
    )
    .await
}

fn project_root_uri(path: &Path) -> Result<String, LspClientError> {
    url::Url::from_directory_path(path)
        .map_err(|()| LspClientError::Protocol(format!("invalid project root: {}", path.display())))
        .map(Into::into)
}

pub(crate) fn restart_delay(base: Duration, attempt: u32) -> Duration {
    base.saturating_mul(2_u32.saturating_pow(attempt))
}

fn next_document_version(previous: Option<i64>, requested: i64) -> i64 {
    previous.map_or(requested, |version| {
        requested.max(version.saturating_add(1))
    })
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use async_trait::async_trait;
    use serde_json::{json, Value};
    use tokio_util::sync::CancellationToken;
    use y_core::types::SessionId;

    use super::{restart_delay, LspClient, LspClientError, LspConnection, LspConnector};
    use crate::lsp::{LspConfig, LspServerConfig};

    struct ConnectionPlan {
        responses: VecDeque<Result<Value, String>>,
    }

    struct FakeConnector {
        plans: Mutex<VecDeque<ConnectionPlan>>,
        events: Arc<Mutex<Vec<String>>>,
    }

    struct FakeConnection {
        responses: VecDeque<Result<Value, String>>,
        events: Arc<Mutex<Vec<String>>>,
        last_request_id: Option<Value>,
    }

    struct BlockingConnector {
        events: Arc<Mutex<Vec<String>>>,
    }

    struct BlockingConnection {
        events: Arc<Mutex<Vec<String>>>,
        initialize_id: Option<Value>,
        initialized: bool,
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
            self.events.lock().expect("events").push("connect".into());
            let plan = self
                .plans
                .lock()
                .expect("plans")
                .pop_front()
                .ok_or_else(|| LspClientError::Transport("no connection plan".into()))?;
            Ok(Box::new(FakeConnection {
                responses: plan.responses,
                events: Arc::clone(&self.events),
                last_request_id: None,
            }))
        }
    }

    #[async_trait]
    impl LspConnection for FakeConnection {
        async fn send(&mut self, message: &Value) -> Result<(), LspClientError> {
            let method = message["method"].as_str().unwrap_or("response");
            self.events
                .lock()
                .expect("events")
                .push(format!("send:{method}"));
            self.last_request_id = message.get("id").cloned();
            Ok(())
        }

        async fn receive(&mut self) -> Result<Value, LspClientError> {
            let response = self
                .responses
                .pop_front()
                .ok_or_else(|| LspClientError::Transport("no response".into()))?
                .map_err(LspClientError::Transport)?;
            if response.get("id").is_some() || self.last_request_id.is_none() {
                return Ok(response);
            }
            let mut response = response;
            response["id"] = self.last_request_id.clone().expect("request id");
            Ok(response)
        }

        async fn close(&mut self) -> Result<(), LspClientError> {
            self.events.lock().expect("events").push("close".into());
            Ok(())
        }
    }

    #[async_trait]
    impl LspConnector for BlockingConnector {
        async fn connect(
            &self,
            _session_id: &SessionId,
            _server: &LspServerConfig,
            _project_root: &Path,
            _max_message_bytes: usize,
            _request_timeout: Duration,
        ) -> Result<Box<dyn LspConnection>, LspClientError> {
            Ok(Box::new(BlockingConnection {
                events: Arc::clone(&self.events),
                initialize_id: None,
                initialized: false,
            }))
        }
    }

    #[async_trait]
    impl LspConnection for BlockingConnection {
        async fn send(&mut self, message: &Value) -> Result<(), LspClientError> {
            let method = message["method"].as_str().unwrap_or("response");
            self.events
                .lock()
                .expect("events")
                .push(format!("send:{method}"));
            if method == "initialize" {
                self.initialize_id = message.get("id").cloned();
            }
            Ok(())
        }

        async fn receive(&mut self) -> Result<Value, LspClientError> {
            if !self.initialized {
                self.initialized = true;
                return Ok(json!({
                    "jsonrpc": "2.0",
                    "id": self.initialize_id.take().expect("initialize id"),
                    "result": {},
                }));
            }
            std::future::pending().await
        }

        async fn close(&mut self) -> Result<(), LspClientError> {
            Ok(())
        }
    }

    fn plan(responses: Vec<Result<Value, &str>>) -> ConnectionPlan {
        ConnectionPlan {
            responses: responses
                .into_iter()
                .map(|response| response.map_err(str::to_string))
                .collect(),
        }
    }

    fn client(
        plans: Vec<ConnectionPlan>,
        max_restarts: u32,
    ) -> (LspClient, Arc<Mutex<Vec<String>>>) {
        let events = Arc::new(Mutex::new(Vec::new()));
        let connector = Arc::new(FakeConnector {
            plans: Mutex::new(plans.into()),
            events: Arc::clone(&events),
        });
        let config = LspConfig {
            enabled: true,
            max_restarts,
            restart_base_delay_ms: 0,
            ..LspConfig::default()
        };
        let server = LspServerConfig {
            id: "rust".into(),
            command: "fake-lsp".into(),
            language_id: "rust".into(),
            extensions: vec!["rs".into()],
            ..LspServerConfig::default()
        };
        (
            LspClient::new(
                SessionId::from_string("session-lsp"),
                server,
                PathBuf::from("/workspace"),
                &config,
                connector,
            ),
            events,
        )
    }

    #[tokio::test]
    async fn restart_is_bounded_by_configuration() {
        let initialize_failure = || plan(vec![Err("server exited during initialize")]);
        let (mut client, events) = client(
            vec![
                initialize_failure(),
                initialize_failure(),
                initialize_failure(),
                initialize_failure(),
            ],
            2,
        );

        let error = client
            .request("workspace/symbol", json!({"query": "Thing"}))
            .await
            .expect_err("restart exhaustion");

        assert!(matches!(
            error,
            LspClientError::RestartExhausted { attempts: 3, .. }
        ));
        assert_eq!(
            events
                .lock()
                .expect("events")
                .iter()
                .filter(|event| event.as_str() == "connect")
                .count(),
            3
        );
    }

    #[tokio::test]
    async fn protocol_level_server_errors_do_not_restart_the_process() {
        let (mut client, events) = client(
            vec![plan(vec![
                Ok(json!({"result": {}})),
                Ok(json!({"error": {"code": -32601, "message": "not supported"}})),
            ])],
            3,
        );

        let error = client
            .request("textDocument/diagnostic", json!({}))
            .await
            .expect_err("server error");

        assert!(matches!(error, LspClientError::Server { code: -32601, .. }));
        assert_eq!(
            events
                .lock()
                .expect("events")
                .iter()
                .filter(|event| event.as_str() == "connect")
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn server_requests_are_answered_while_waiting_for_a_response() {
        let (mut client, events) = client(
            vec![plan(vec![
                Ok(json!({"result": {}})),
                Ok(json!({
                    "jsonrpc": "2.0",
                    "id": 99,
                    "method": "workspace/configuration",
                    "params": {"items": [{}, {}]},
                })),
                Ok(json!({"jsonrpc": "2.0", "id": 2, "result": []})),
            ])],
            0,
        );

        client
            .request("workspace/symbol", json!({"query": "Thing"}))
            .await
            .expect("workspace symbols");

        assert!(events
            .lock()
            .expect("events")
            .iter()
            .any(|event| event == "send:response"));
    }

    #[tokio::test]
    async fn cancellation_is_forwarded_to_the_language_server() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let connector = Arc::new(BlockingConnector {
            events: Arc::clone(&events),
        });
        let config = LspConfig {
            enabled: true,
            max_restarts: 0,
            ..LspConfig::default()
        };
        let server = LspServerConfig {
            id: "rust".into(),
            command: "fake-lsp".into(),
            language_id: "rust".into(),
            ..LspServerConfig::default()
        };
        let mut client = LspClient::new(
            SessionId::from_string("session-lsp"),
            server,
            PathBuf::from("/workspace"),
            &config,
            connector,
        );
        let cancellation = CancellationToken::new();
        cancellation.cancel();

        let error = client
            .request_with_cancellation("workspace/symbol", json!({"query": "Thing"}), &cancellation)
            .await
            .expect_err("cancelled request");

        assert!(matches!(error, LspClientError::Cancelled));
        assert!(events
            .lock()
            .expect("events")
            .iter()
            .any(|event| event == "send:$/cancelRequest"));
    }

    #[tokio::test]
    async fn tracked_documents_replay_after_reinitialize() {
        let (mut client, events) = client(
            vec![
                plan(vec![Ok(json!({"result": {}})), Err("server crashed")]),
                plan(vec![
                    Ok(json!({"result": {}})),
                    Ok(json!({"result": [{"uri": "file:///workspace/src/lib.rs"}]})),
                ]),
            ],
            1,
        );
        client
            .open_document(
                Path::new("/workspace/src/lib.rs"),
                "pub struct Thing;".into(),
                1,
            )
            .await
            .expect("open document");

        let result = client
            .request("workspace/symbol", json!({"query": "Thing"}))
            .await
            .expect("request after restart");

        assert_eq!(result[0]["uri"], "file:///workspace/src/lib.rs");
        let events = events.lock().expect("events");
        assert_eq!(
            events
                .iter()
                .filter(|event| event.as_str() == "send:textDocument/didOpen")
                .count(),
            2
        );
        let second_initialize = events
            .iter()
            .rposition(|event| event == "send:initialized")
            .expect("second initialized");
        let replay = events
            .iter()
            .rposition(|event| event == "send:textDocument/didOpen")
            .expect("replayed didOpen");
        let retried_request = events
            .iter()
            .rposition(|event| event == "send:workspace/symbol")
            .expect("retried request");
        assert!(second_initialize < replay && replay < retried_request);
    }

    #[tokio::test]
    async fn shutdown_sends_protocol_messages_before_closing_transport() {
        let (mut client, events) = client(
            vec![plan(vec![
                Ok(json!({"result": {}})),
                Ok(json!({"result": Value::Null})),
            ])],
            0,
        );
        client
            .open_document(
                Path::new("/workspace/src/lib.rs"),
                "pub struct Thing;".into(),
                1,
            )
            .await
            .expect("open document");

        client.shutdown().await;

        let events = events.lock().expect("events");
        let shutdown = events
            .iter()
            .position(|event| event == "send:shutdown")
            .expect("shutdown request");
        let exit = events
            .iter()
            .position(|event| event == "send:exit")
            .expect("exit notification");
        let close = events
            .iter()
            .position(|event| event == "close")
            .expect("transport close");
        assert!(shutdown < exit && exit < close);
    }

    #[test]
    fn restart_backoff_is_exponential() {
        assert_eq!(
            restart_delay(Duration::from_millis(10), 0),
            Duration::from_millis(10)
        );
        assert_eq!(
            restart_delay(Duration::from_millis(10), 1),
            Duration::from_millis(20)
        );
        assert_eq!(
            restart_delay(Duration::from_millis(10), 2),
            Duration::from_millis(40)
        );
    }

    #[test]
    fn changed_documents_use_monotonic_versions() {
        assert_eq!(super::next_document_version(None, 1), 1);
        assert_eq!(super::next_document_version(Some(1), 1), 2);
        assert_eq!(super::next_document_version(Some(3), 8), 8);
    }
}
