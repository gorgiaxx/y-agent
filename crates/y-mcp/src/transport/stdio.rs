//! Stdio transport: spawns a child process and communicates via stdin/stdout.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use std::time::Duration;

use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin};
use tokio::sync::{oneshot, Mutex};
use tokio::task::JoinHandle;
use tracing::{debug, warn};

use super::{
    JsonRpcNotification, JsonRpcRequest, JsonRpcResponse, McpTransport, NotificationHandler,
    RawJsonRpcMessage,
};
use crate::error::McpError;

/// MCP transport over subprocess stdin/stdout.
///
/// Spawns a child process and exchanges newline-delimited JSON-RPC messages
/// over its standard I/O streams.
pub struct StdioTransport {
    stdin: Arc<Mutex<ChildStdin>>,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<JsonRpcResponse>>>>,
    notification_handler: Arc<Mutex<Option<NotificationHandler>>>,
    reader_handle: Mutex<Option<JoinHandle<()>>>,
    stderr_handle: Mutex<Option<JoinHandle<()>>>,
    child: Mutex<Option<Child>>,
    closed: AtomicBool,
}

impl std::fmt::Debug for StdioTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StdioTransport")
            .field("closed", &self.closed.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

impl StdioTransport {
    /// Spawn a child process and create a stdio transport.
    ///
    /// The child process must speak newline-delimited JSON-RPC on stdin/stdout.
    /// An optional working directory can be specified for the child process.
    pub fn spawn(
        command: &str,
        args: &[String],
        env: &HashMap<String, String>,
        cwd: Option<&str>,
    ) -> Result<Self, McpError> {
        let mut cmd = tokio::process::Command::new(command);
        cmd.args(args)
            .envs(env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }

        let mut child = cmd.spawn().map_err(|e| McpError::ConnectionFailed {
            message: format!("failed to spawn '{command}': {e}"),
        })?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| McpError::ConnectionFailed {
                message: "failed to capture child stdin".into(),
            })?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| McpError::ConnectionFailed {
                message: "failed to capture child stdout".into(),
            })?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| McpError::ConnectionFailed {
                message: "failed to capture child stderr".into(),
            })?;

        let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<JsonRpcResponse>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        let notification_handler: Arc<Mutex<Option<NotificationHandler>>> =
            Arc::new(Mutex::new(None));

        // Background task: read stdout lines and route responses/notifications.
        let reader_pending = Arc::clone(&pending);
        let reader_notif = Arc::clone(&notification_handler);
        let reader_handle = tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                // Peek at the message to decide: response (has id) or notification
                // (has method, no id).
                let raw: RawJsonRpcMessage = match serde_json::from_str(&line) {
                    Ok(r) => r,
                    Err(e) => {
                        debug!(
                            error = %e,
                            line = %line,
                            "failed to parse JSON-RPC message from stdout, skipping"
                        );
                        continue;
                    }
                };

                if let Some(id) = raw.id {
                    // This is a response.
                    let resp: JsonRpcResponse = match serde_json::from_str(&line) {
                        Ok(r) => r,
                        Err(e) => {
                            debug!(
                                error = %e,
                                "failed to parse JSON-RPC response, skipping"
                            );
                            continue;
                        }
                    };
                    let mut map = reader_pending.lock().await;
                    if let Some(tx) = map.remove(&id) {
                        let _ = tx.send(resp);
                    } else {
                        debug!(id, "received response for unknown request ID");
                    }
                } else if let Some(method) = raw.method {
                    // This is a server notification.
                    let params: Option<serde_json::Value> =
                        serde_json::from_str::<serde_json::Value>(&line)
                            .ok()
                            .and_then(|v| v.get("params").cloned());
                    debug!(method = %method, "received server notification");
                    if let Some(handler) = reader_notif.lock().await.as_ref() {
                        handler(&method, params);
                    }
                } else {
                    debug!(line = %line, "received unrecognized JSON-RPC message");
                }
            }
            // stdout closed -- clear all pending requests so they get RecvError.
            let mut map = reader_pending.lock().await;
            map.clear();
        });

        // Background task: log stderr lines.
        let stderr_handle = tokio::spawn(async move {
            let reader = BufReader::new(stderr);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                debug!(target: "mcp_server_stderr", "{}", line);
            }
        });

        Ok(Self {
            stdin: Arc::new(Mutex::new(stdin)),
            pending,
            notification_handler,
            reader_handle: Mutex::new(Some(reader_handle)),
            stderr_handle: Mutex::new(Some(stderr_handle)),
            child: Mutex::new(Some(child)),
            closed: AtomicBool::new(false),
        })
    }

    /// Write a serialized message (+ newline) to stdin.
    async fn write_message(&self, data: &[u8]) -> Result<(), McpError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(McpError::TransportError {
                message: "transport is closed".into(),
            });
        }
        let mut stdin = self.stdin.lock().await;
        stdin
            .write_all(data)
            .await
            .map_err(|e| McpError::TransportError {
                message: format!("failed to write to stdin: {e}"),
            })?;
        stdin
            .write_all(b"\n")
            .await
            .map_err(|e| McpError::TransportError {
                message: format!("failed to write newline: {e}"),
            })?;
        stdin.flush().await.map_err(|e| McpError::TransportError {
            message: format!("failed to flush stdin: {e}"),
        })?;
        Ok(())
    }
}

#[async_trait]
impl McpTransport for StdioTransport {
    async fn send(&self, request: JsonRpcRequest) -> Result<JsonRpcResponse, McpError> {
        let id = request.id;
        let (tx, rx) = oneshot::channel();

        // Register the pending response channel.
        {
            let mut map = self.pending.lock().await;
            map.insert(id, tx);
        }

        // Serialize and write.
        let data = serde_json::to_vec(&request).map_err(McpError::SerializationError)?;
        if let Err(e) = self.write_message(&data).await {
            // Clean up pending entry on write failure.
            let mut map = self.pending.lock().await;
            map.remove(&id);
            return Err(e);
        }

        // Wait for the response with a timeout.
        match tokio::time::timeout(std::time::Duration::from_secs(30), rx).await {
            Ok(Ok(resp)) => Ok(resp),
            Ok(Err(_)) => Err(McpError::TransportError {
                message: "server process exited before responding".into(),
            }),
            Err(_) => {
                // Timeout -- remove pending entry.
                let mut map = self.pending.lock().await;
                map.remove(&id);
                Err(McpError::Timeout {
                    message: format!("request {id} timed out after 30s"),
                })
            }
        }
    }

    async fn send_notification(&self, notification: JsonRpcNotification) -> Result<(), McpError> {
        let data = serde_json::to_vec(&notification).map_err(McpError::SerializationError)?;
        self.write_message(&data).await
    }

    async fn close(&self) -> Result<(), McpError> {
        self.closed.store(true, Ordering::Release);

        // Graceful shutdown: SIGTERM -> grace period -> SIGKILL.
        if let Some(mut child) = self.child.lock().await.take() {
            graceful_shutdown(&mut child).await;
        }

        // Abort background tasks.
        if let Some(handle) = self.reader_handle.lock().await.take() {
            handle.abort();
        }
        if let Some(handle) = self.stderr_handle.lock().await.take() {
            handle.abort();
        }

        // Clear pending requests.
        self.pending.lock().await.clear();

        Ok(())
    }

    fn transport_type(&self) -> &'static str {
        "stdio"
    }

    fn set_notification_handler(&self, handler: NotificationHandler) {
        // Use try_lock to avoid blocking in sync context. The handler is
        // typically set once during setup before any messages arrive.
        if let Ok(mut guard) = self.notification_handler.try_lock() {
            *guard = Some(handler);
        }
    }
}

/// Grace period before escalating from SIGTERM to SIGKILL.
const SHUTDOWN_GRACE_PERIOD: Duration = Duration::from_secs(2);

/// Gracefully shut down a child process.
///
/// On Unix: sends SIGTERM, waits up to 2 seconds, then SIGKILL if still alive.
/// On other platforms: falls back to immediate kill.
async fn graceful_shutdown(child: &mut Child) {
    #[cfg(unix)]
    {
        use nix::sys::signal::{self, Signal};
        use nix::unistd::Pid;

        if let Some(pid) = child.id().and_then(|p| i32::try_from(p).ok()) {
            let pid = Pid::from_raw(pid);
            if signal::kill(pid, Signal::SIGTERM).is_ok() {
                debug!(%pid, "sent SIGTERM to MCP server process");
                match tokio::time::timeout(SHUTDOWN_GRACE_PERIOD, child.wait()).await {
                    Ok(Ok(status)) => {
                        debug!(%pid, %status, "MCP server process exited after SIGTERM");
                        return;
                    }
                    Ok(Err(e)) => {
                        warn!(%pid, error = %e, "error waiting for MCP server process");
                    }
                    Err(_) => {
                        debug!(%pid, "SIGTERM grace period expired, sending SIGKILL");
                    }
                }
            }
        }
    }

    // Fallback: force kill.
    let _ = child.kill().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_spawn_nonexistent_command() {
        let result = StdioTransport::spawn("__nonexistent_mcp_cmd__", &[], &HashMap::new(), None);
        assert!(
            matches!(result, Err(McpError::ConnectionFailed { .. })),
            "expected ConnectionFailed, got: {result:?}"
        );
    }

    #[tokio::test]
    async fn test_send_after_close() {
        // Use 'cat' as a simple echo process.
        let transport =
            StdioTransport::spawn("cat", &[], &HashMap::new(), None).expect("failed to spawn cat");
        transport.close().await.unwrap();

        let req = JsonRpcRequest::new(1, "test", None);
        let result = transport.send(req).await;
        assert!(
            matches!(result, Err(McpError::TransportError { .. })),
            "expected TransportError after close, got: {result:?}"
        );
    }

    #[tokio::test]
    async fn test_roundtrip_echo() {
        // 'cat' echoes stdin to stdout, so a valid JSON-RPC response written
        // as a request will be echoed back and parsed as a response.
        // We craft a JSON object that is valid as both request serialization
        // output and response deserialization input.
        let transport =
            StdioTransport::spawn("cat", &[], &HashMap::new(), None).expect("failed to spawn cat");

        // Send a request -- cat will echo it back. The echoed JSON has
        // the same `id` so the pending map will match it.
        // Note: cat echoes the serialized JsonRpcRequest, which has
        // `jsonrpc`, `id`, `method` fields. JsonRpcResponse expects
        // `jsonrpc`, `id`, `result?`, `error?` -- missing result/error
        // deserialize as None, which is fine for this test.
        let req = JsonRpcRequest::new(42, "test/echo", None);
        let resp = transport.send(req).await.expect("send failed");
        assert_eq!(resp.id, 42);

        transport.close().await.unwrap();
    }
}
