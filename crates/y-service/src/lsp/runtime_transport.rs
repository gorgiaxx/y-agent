use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::Value;
use y_core::runtime::{
    CommandRunner, ExecutionRequest, ProcessCapability, ProcessStatus, RuntimeCapability,
};
use y_core::types::SessionId;
use y_runtime::RuntimeManager;

use crate::lsp::{LspClientError, LspConnection, LspConnector, LspFrameDecoder, LspServerConfig};

pub struct RuntimeLspConnector {
    runtime: Arc<RuntimeManager>,
}

impl RuntimeLspConnector {
    pub fn new(runtime: Arc<RuntimeManager>) -> Self {
        Self { runtime }
    }
}

#[async_trait]
impl LspConnector for RuntimeLspConnector {
    async fn connect(
        &self,
        session_id: &SessionId,
        server: &LspServerConfig,
        project_root: &Path,
        max_message_bytes: usize,
        request_timeout: Duration,
    ) -> Result<Box<dyn LspConnection>, LspClientError> {
        let handle = self
            .runtime
            .spawn_managed_native(lsp_execution_request(session_id, server, project_root))
            .await
            .map_err(|error| runtime_error(&error))?;
        Ok(Box::new(RuntimeLspConnection {
            runtime: Arc::clone(&self.runtime),
            session_id: session_id.clone(),
            process_id: handle.id,
            decoder: LspFrameDecoder::new(max_message_bytes),
            messages: VecDeque::new(),
            request_timeout,
            max_output_bytes: max_message_bytes.saturating_add(8 * 1024),
        }))
    }
}

struct RuntimeLspConnection {
    runtime: Arc<RuntimeManager>,
    session_id: SessionId,
    process_id: String,
    decoder: LspFrameDecoder,
    messages: VecDeque<Value>,
    request_timeout: Duration,
    max_output_bytes: usize,
}

#[async_trait]
impl LspConnection for RuntimeLspConnection {
    async fn send(&mut self, message: &Value) -> Result<(), LspClientError> {
        let frame = LspFrameDecoder::encode(message)
            .map_err(|error| LspClientError::Protocol(error.to_string()))?;
        let snapshot = self
            .runtime
            .write_process(
                &self.session_id,
                &self.process_id,
                &frame,
                Duration::ZERO,
                self.max_output_bytes,
            )
            .await
            .map_err(|error| runtime_error(&error))?;
        self.ingest(&snapshot.stdout)?;
        self.ensure_process_available(&snapshot.status, &snapshot.stderr)
    }

    async fn receive(&mut self) -> Result<Value, LspClientError> {
        if let Some(message) = self.messages.pop_front() {
            return Ok(message);
        }
        let deadline = Instant::now() + self.request_timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(LspClientError::Transport(format!(
                    "timed out waiting for language-server response after {:?}",
                    self.request_timeout
                )));
            }
            let snapshot = self
                .runtime
                .read_process(
                    &self.session_id,
                    &self.process_id,
                    remaining.min(Duration::from_millis(50)),
                    self.max_output_bytes,
                )
                .await
                .map_err(|error| runtime_error(&error))?;
            self.ingest(&snapshot.stdout)?;
            if let Some(message) = self.messages.pop_front() {
                return Ok(message);
            }
            self.ensure_process_available(&snapshot.status, &snapshot.stderr)?;
        }
    }

    async fn close(&mut self) -> Result<(), LspClientError> {
        self.runtime
            .kill_process(
                &self.session_id,
                &self.process_id,
                Duration::from_millis(50),
                self.max_output_bytes,
            )
            .await
            .map(|_| ())
            .map_err(|error| runtime_error(&error))
    }
}

impl RuntimeLspConnection {
    fn ingest(&mut self, bytes: &[u8]) -> Result<(), LspClientError> {
        let messages = self
            .decoder
            .push(bytes)
            .map_err(|error| LspClientError::Protocol(error.to_string()))?;
        self.messages.extend(messages);
        Ok(())
    }

    fn ensure_process_available(
        &self,
        status: &ProcessStatus,
        stderr: &[u8],
    ) -> Result<(), LspClientError> {
        if matches!(status, ProcessStatus::Running) || !self.messages.is_empty() {
            return Ok(());
        }
        let detail = String::from_utf8_lossy(stderr).trim().to_string();
        let suffix = if detail.is_empty() {
            String::new()
        } else {
            format!(": {detail}")
        };
        Err(LspClientError::Transport(format!(
            "language-server process stopped with status {status:?}{suffix}"
        )))
    }
}

fn lsp_execution_request(
    session_id: &SessionId,
    server: &LspServerConfig,
    project_root: &Path,
) -> ExecutionRequest {
    ExecutionRequest {
        command: server.command.clone(),
        args: server.args.clone(),
        working_dir: Some(project_root.to_string_lossy().into_owned()),
        env: HashMap::new(),
        stdin: None,
        owner_session_id: Some(session_id.clone()),
        event_tool_name: Some("LspServer".to_string()),
        capabilities: RuntimeCapability {
            process: ProcessCapability {
                background: true,
                ..ProcessCapability::default()
            },
            ..RuntimeCapability::default()
        },
        image: None,
    }
}

fn runtime_error(error: &y_core::runtime::RuntimeError) -> LspClientError {
    LspClientError::Transport(error.to_string())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use y_core::types::SessionId;

    use super::lsp_execution_request;
    use crate::lsp::LspServerConfig;

    #[test]
    fn lsp_process_request_preserves_runtime_security_and_event_identity() {
        let session_id = SessionId::from_string("session-lsp");
        let server = LspServerConfig {
            id: "rust".into(),
            command: "rust-analyzer".into(),
            args: vec!["--stdio".into()],
            ..LspServerConfig::default()
        };

        let request = lsp_execution_request(&session_id, &server, Path::new("/workspace"));

        assert_eq!(request.command, "rust-analyzer");
        assert_eq!(request.args, vec!["--stdio"]);
        assert_eq!(request.working_dir.as_deref(), Some("/workspace"));
        assert_eq!(request.owner_session_id.as_ref(), Some(&session_id));
        assert_eq!(request.event_tool_name.as_deref(), Some("LspServer"));
        assert!(request.capabilities.process.background);
        assert_eq!(request.image, None);
    }
}
