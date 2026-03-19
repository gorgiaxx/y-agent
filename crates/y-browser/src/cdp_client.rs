//! CDP (Chrome DevTools Protocol) WebSocket client.
//!
//! Provides low-level JSON-RPC communication with Chrome over WebSocket.
//! Inspired by openclaw's direct CDP approach.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, oneshot};
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, warn};

/// Error type for CDP operations.
#[derive(Debug, thiserror::Error)]
pub enum CdpError {
    #[error("not connected to CDP endpoint")]
    NotConnected,

    #[error("connection failed: {0}")]
    ConnectionFailed(String),

    #[error("CDP request timed out after {0}ms")]
    Timeout(u64),

    #[error("CDP error (code {code}): {message}")]
    ProtocolError { code: i64, message: String },

    #[error("WebSocket error: {0}")]
    WebSocket(String),

    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("CDP endpoint discovery failed: {0}")]
    DiscoveryFailed(String),
}

/// CDP JSON-RPC request.
#[derive(Debug, Serialize)]
struct CdpRequest {
    id: u64,
    method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<serde_json::Value>,
}

/// CDP JSON-RPC response.
#[derive(Debug, Deserialize)]
struct CdpResponse {
    id: Option<u64>,
    result: Option<serde_json::Value>,
    error: Option<CdpProtocolError>,
    // Events have `method` + `params` but no `id`.
    #[allow(dead_code)]
    method: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CdpProtocolError {
    code: i64,
    message: String,
}

/// Chrome version info from `/json/version`.
#[derive(Debug, Deserialize)]
struct VersionInfo {
    #[serde(rename = "webSocketDebuggerUrl")]
    web_socket_debugger_url: Option<String>,
    #[serde(rename = "Browser")]
    #[allow(dead_code)]
    browser: Option<String>,
}

/// Target info from `/json/list`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CdpTarget {
    pub id: String,
    #[serde(rename = "type")]
    pub target_type: String,
    pub title: String,
    pub url: String,
    #[serde(rename = "webSocketDebuggerUrl")]
    pub ws_url: Option<String>,
}

type PendingMap = HashMap<u64, oneshot::Sender<Result<serde_json::Value, CdpError>>>;

/// CDP WebSocket client.
///
/// Connects to Chrome via the DevTools Protocol WebSocket and sends
/// JSON-RPC commands. Thread-safe and shareable via `Arc`.
pub struct CdpClient {
    /// The base CDP URL (http:// or ws://).
    cdp_url: String,
    /// Default timeout for requests.
    default_timeout: Duration,
    /// Auto-incrementing message ID.
    next_id: AtomicU64,
    /// WebSocket writer half (protected by mutex).
    writer: Mutex<Option<WriterHalf>>,
    /// Pending request map: id → response sender.
    pending: Arc<Mutex<PendingMap>>,
    /// Handle to the reader task.
    reader_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

type WriterHalf = futures_util::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    Message,
>;

impl CdpClient {
    /// Create a new CDP client (not yet connected).
    pub fn new(cdp_url: String, default_timeout: Duration) -> Self {
        Self {
            cdp_url,
            default_timeout,
            next_id: AtomicU64::new(1),
            writer: Mutex::new(None),
            pending: Arc::new(Mutex::new(HashMap::new())),
            reader_handle: Mutex::new(None),
        }
    }

    /// Connect to the CDP endpoint.
    ///
    /// For HTTP(S) URLs, discovers the WebSocket URL via `/json/version`.
    /// For WS(S) URLs, connects directly.
    pub async fn connect(&self) -> Result<(), CdpError> {
        let ws_url = self.resolve_ws_url().await?;
        debug!(ws_url = %ws_url, "connecting to CDP WebSocket");

        let (ws_stream, _) = tokio_tungstenite::connect_async(&ws_url)
            .await
            .map_err(|e| CdpError::ConnectionFailed(e.to_string()))?;

        let (writer, reader) = ws_stream.split();

        // Store writer.
        *self.writer.lock().await = Some(writer);

        // Spawn reader task to dispatch responses to pending senders.
        let pending = Arc::clone(&self.pending);
        let handle = tokio::spawn(async move {
            Self::reader_loop(reader, pending).await;
        });

        *self.reader_handle.lock().await = Some(handle);

        debug!("CDP WebSocket connected");
        Ok(())
    }

    /// Disconnect from the CDP endpoint.
    pub async fn disconnect(&self) {
        // Close the writer (sends close frame).
        if let Some(mut writer) = self.writer.lock().await.take() {
            let _ = writer.close().await;
        }
        // Abort reader task.
        if let Some(handle) = self.reader_handle.lock().await.take() {
            handle.abort();
        }
        // Drain pending.
        let mut pending = self.pending.lock().await;
        for (_, sender) in pending.drain() {
            let _ = sender.send(Err(CdpError::NotConnected));
        }
    }

    /// Check if the client is connected.
    pub async fn is_connected(&self) -> bool {
        self.writer.lock().await.is_some()
    }

    /// Send a CDP command and wait for the response.
    pub async fn send(
        &self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> Result<serde_json::Value, CdpError> {
        self.send_with_timeout(method, params, self.default_timeout)
            .await
    }

    /// Send a CDP command with a custom timeout.
    pub async fn send_with_timeout(
        &self,
        method: &str,
        params: Option<serde_json::Value>,
        request_timeout: Duration,
    ) -> Result<serde_json::Value, CdpError> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let request = CdpRequest {
            id,
            method: method.into(),
            params,
        };

        let json = serde_json::to_string(&request)?;
        debug!(id, method, "CDP send");

        // Register pending response.
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);

        // Send over WebSocket.
        {
            let mut guard = self.writer.lock().await;
            let writer = guard.as_mut().ok_or(CdpError::NotConnected)?;
            writer
                .send(Message::Text(json.into()))
                .await
                .map_err(|e| CdpError::WebSocket(e.to_string()))?;
        }

        // Wait for response with timeout.
        let timeout_ms = request_timeout.as_millis() as u64;
        match timeout(request_timeout, rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(CdpError::NotConnected), // sender dropped
            Err(_) => {
                // Remove from pending on timeout.
                self.pending.lock().await.remove(&id);
                Err(CdpError::Timeout(timeout_ms))
            }
        }
    }

    /// List available targets (pages/tabs).
    pub async fn list_targets(&self) -> Result<Vec<CdpTarget>, CdpError> {
        let url = format!("{}/json/list", self.http_base_url());
        let resp = reqwest::get(&url)
            .await
            .map_err(|e| CdpError::DiscoveryFailed(e.to_string()))?;
        let targets: Vec<CdpTarget> = resp
            .json()
            .await
            .map_err(|e| CdpError::DiscoveryFailed(e.to_string()))?;
        Ok(targets)
    }

    /// Resolve the WebSocket URL from the configured CDP URL.
    async fn resolve_ws_url(&self) -> Result<String, CdpError> {
        let url = self.cdp_url.trim();

        // Direct WebSocket URL.
        if url.starts_with("ws://") || url.starts_with("wss://") {
            return Ok(url.to_string());
        }

        // HTTP(S) — discover via /json/version.
        let version_url = format!("{}/json/version", url.trim_end_matches('/'));
        let resp = reqwest::get(&version_url)
            .await
            .map_err(|e| CdpError::DiscoveryFailed(format!("GET {version_url}: {e}")))?;

        let info: VersionInfo = resp
            .json()
            .await
            .map_err(|e| CdpError::DiscoveryFailed(format!("parse /json/version: {e}")))?;

        let ws_url = info
            .web_socket_debugger_url
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                CdpError::DiscoveryFailed(
                    "/json/version missing webSocketDebuggerUrl".into(),
                )
            })?;

        // Normalize: rewrite loopback if CDP URL host differs.
        Ok(normalize_ws_url(&ws_url, url))
    }

    /// Get HTTP base URL for /json/* endpoints.
    fn http_base_url(&self) -> String {
        let url = self.cdp_url.trim().trim_end_matches('/');
        if url.starts_with("ws://") {
            url.replacen("ws://", "http://", 1)
        } else if url.starts_with("wss://") {
            url.replacen("wss://", "https://", 1)
        } else {
            url.to_string()
        }
    }

    /// Reader loop: receives WebSocket messages and dispatches responses.
    async fn reader_loop(
        mut reader: futures_util::stream::SplitStream<
            tokio_tungstenite::WebSocketStream<
                tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
            >,
        >,
        pending: Arc<Mutex<PendingMap>>,
    ) {
        while let Some(msg) = reader.next().await {
            let text = match msg {
                Ok(Message::Text(t)) => t.to_string(),
                Ok(Message::Close(_)) => break,
                Ok(_) => continue, // binary, ping, pong
                Err(e) => {
                    warn!(error = %e, "CDP WebSocket read error");
                    break;
                }
            };

            let resp: CdpResponse = match serde_json::from_str(&text) {
                Ok(r) => r,
                Err(e) => {
                    warn!(error = %e, "CDP invalid JSON response");
                    continue;
                }
            };

            // Events (no id) are ignored for now.
            let Some(id) = resp.id else {
                continue;
            };

            let result = if let Some(err) = resp.error {
                Err(CdpError::ProtocolError {
                    code: err.code,
                    message: err.message,
                })
            } else {
                Ok(resp.result.unwrap_or(serde_json::Value::Null))
            };

            let mut pending = pending.lock().await;
            if let Some(sender) = pending.remove(&id) {
                let _ = sender.send(result);
            }
        }

        // Connection lost: close all pending.
        let mut pending = pending.lock().await;
        for (_, sender) in pending.drain() {
            let _ = sender.send(Err(CdpError::NotConnected));
        }
    }
}

impl Drop for CdpClient {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.reader_handle.try_lock() {
            if let Some(handle) = guard.take() {
                handle.abort();
            }
        }
    }
}

/// Normalize WebSocket URL reported by Chrome: rewrite loopback if the
/// CDP URL points to a different host (e.g., Docker container).
fn normalize_ws_url(ws_url: &str, cdp_url: &str) -> String {
    let Ok(mut ws) = url::Url::parse(ws_url) else {
        return ws_url.to_string();
    };
    let Ok(cdp) = url::Url::parse(cdp_url) else {
        return ws_url.to_string();
    };

    let ws_host = ws.host_str().unwrap_or_default();
    let cdp_host = cdp.host_str().unwrap_or_default();

    let is_ws_loopback = ws_host == "127.0.0.1"
        || ws_host == "localhost"
        || ws_host == "0.0.0.0"
        || ws_host == "[::]";
    let is_cdp_loopback =
        cdp_host == "127.0.0.1" || cdp_host == "localhost";

    if is_ws_loopback && !is_cdp_loopback {
        let _ = ws.set_host(Some(cdp_host));
        if let Some(port) = cdp.port() {
            let _ = ws.set_port(Some(port));
        }
    }

    ws.to_string()
}
