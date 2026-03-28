//! CDP (Chrome `DevTools` Protocol) WebSocket client.
//!
//! Provides low-level JSON-RPC communication with Chrome over WebSocket.
//! Inspired by openclaw's direct CDP approach.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, oneshot, Mutex};
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, trace, warn};

use crate::timeouts;

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

/// A CDP event received from the browser.
#[derive(Debug, Clone)]
pub struct CdpEvent {
    /// The event method name (e.g. "Runtime.consoleAPICalled").
    pub method: String,
    /// Event parameters.
    pub params: serde_json::Value,
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
    /// Events have `method` + `params` but no `id`.
    method: Option<String>,
    /// Event params (present when method is set).
    params: Option<serde_json::Value>,
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
/// Connects to Chrome via the `DevTools` Protocol WebSocket and sends
/// JSON-RPC commands. Thread-safe and shareable via `Arc`.
pub struct CdpClient {
    /// The base CDP URL (http:// or ws://). Wrapped in `RwLock` to allow
    /// updating after launcher picks a different port.
    cdp_url: RwLock<String>,
    /// Default timeout for requests.
    default_timeout: Duration,
    /// Auto-incrementing message ID.
    next_id: AtomicU64,
    /// WebSocket writer half (protected by mutex).
    writer: Arc<Mutex<Option<WriterHalf>>>,
    /// Pending request map: id -> response sender.
    pending: Arc<Mutex<PendingMap>>,
    /// Handle to the reader task.
    reader_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// Broadcast sender for CDP events.
    event_tx: broadcast::Sender<CdpEvent>,
    /// Whether the connection is alive. Set to false by the reader loop when
    /// the WebSocket closes (e.g. browser crash). This ensures `is_connected`
    /// returns false immediately without waiting to try a write.
    alive: Arc<AtomicBool>,
}

type WriterHalf = futures_util::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    Message,
>;

impl CdpClient {
    /// Create a new CDP client (not yet connected).
    pub fn new(cdp_url: String, default_timeout: Duration) -> Self {
        let (event_tx, _) = broadcast::channel(256);
        Self {
            cdp_url: RwLock::new(cdp_url),
            default_timeout,
            next_id: AtomicU64::new(1),
            writer: Arc::new(Mutex::new(None)),
            pending: Arc::new(Mutex::new(HashMap::new())),
            reader_handle: Mutex::new(None),
            event_tx,
            alive: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Update the CDP URL (e.g. when the launcher picks a different port).
    ///
    /// # Panics
    ///
    /// Panics if the internal lock is poisoned.
    pub fn set_cdp_url(&self, url: String) {
        *self.cdp_url.write().unwrap() = url;
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
        self.alive.store(true, Ordering::SeqCst);

        // Spawn reader task to dispatch responses to pending senders.
        // Share the writer handle and alive flag so the reader can clear
        // them when the WebSocket closes unexpectedly.
        let pending = Arc::clone(&self.pending);
        let event_tx = self.event_tx.clone();
        let writer_handle = Arc::clone(&self.writer);
        let alive_flag = Arc::clone(&self.alive);
        let handle = tokio::spawn(async move {
            Self::reader_loop(reader, pending, event_tx, writer_handle, alive_flag).await;
        });

        *self.reader_handle.lock().await = Some(handle);

        debug!("CDP WebSocket connected");
        Ok(())
    }

    /// Disconnect from the CDP endpoint.
    pub async fn disconnect(&self) {
        self.alive.store(false, Ordering::SeqCst);
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
    ///
    /// Returns true only if the WebSocket writer exists **and** the reader
    /// loop has not detected a connection loss.
    pub async fn is_connected(&self) -> bool {
        self.alive.load(Ordering::SeqCst) && self.writer.lock().await.is_some()
    }

    /// Subscribe to CDP events.
    ///
    /// Returns a receiver that will receive all CDP events dispatched by
    /// the reader loop. The caller should spawn a task to drain the receiver.
    pub fn subscribe_events(&self) -> broadcast::Receiver<CdpEvent> {
        self.event_tx.subscribe()
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
        let timeout_ms = u64::try_from(request_timeout.as_millis()).unwrap_or(u64::MAX);
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
    ///
    /// Prefers connecting to a **page-level** target (which supports `Page.*`,
    /// `Runtime.*`, etc.) rather than the browser-level endpoint.
    ///
    /// Retries `/json/list` multiple times (Chrome may need a moment after
    /// launching to register its default tab), creating a new page target
    /// only as a last resort to avoid producing duplicate windows/tabs.
    async fn resolve_ws_url(&self) -> Result<String, CdpError> {
        let cdp_url_owned = self.cdp_url.read().unwrap().clone();
        let url = cdp_url_owned.trim();

        // Direct WebSocket URL — use as-is.
        if url.starts_with("ws://") || url.starts_with("wss://") {
            return Ok(url.to_string());
        }

        // HTTP(S) — try to find an existing page target via /json/list.
        // Page targets support the full CDP domain set (Page, Runtime, DOM, etc.)
        // while the browser endpoint from /json/version only supports
        // Target.* and Browser.* commands.
        //
        // Retry several times because Chrome may not have registered its
        // default tab yet right after launching.
        let base = url.trim_end_matches('/');
        let list_url = format!("{base}/json/list");

        let max_retries = 10;
        let retry_delay = Duration::from_millis(200);

        for attempt in 0..max_retries {
            if let Ok(resp) = reqwest::get(&list_url).await {
                if let Ok(targets) = resp.json::<Vec<CdpTarget>>().await {
                    // Pick the first "page" target that has a WebSocket URL.
                    if let Some(target) = targets
                        .iter()
                        .find(|t| t.target_type == "page" && t.ws_url.is_some())
                    {
                        let ws_url = target.ws_url.as_ref().unwrap();
                        debug!(ws_url, target_url = %target.url, attempt, "using existing page target");
                        return Ok(normalize_ws_url(ws_url, url));
                    }
                }
            }

            if attempt < max_retries - 1 {
                debug!(attempt, "no page target yet, retrying...");
                tokio::time::sleep(retry_delay).await;
            }
        }

        // No page target found after retries — get the browser endpoint from
        // /json/version and create a new page target via Target.createTarget.
        debug!("no page target found after {max_retries} retries, creating one via Target.createTarget");
        let version_url = format!("{base}/json/version");
        let resp = reqwest::get(&version_url)
            .await
            .map_err(|e| CdpError::DiscoveryFailed(format!("GET {version_url}: {e}")))?;

        let info: VersionInfo = resp
            .json()
            .await
            .map_err(|e| CdpError::DiscoveryFailed(format!("parse /json/version: {e}")))?;

        let browser_ws_url = info
            .web_socket_debugger_url
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                CdpError::DiscoveryFailed("/json/version missing webSocketDebuggerUrl".into())
            })?;

        let browser_ws = normalize_ws_url(&browser_ws_url, url);

        // Try to create a new page target via the browser-level endpoint.
        // This gives us a page-level WebSocket that supports Page.*, Runtime.*, etc.
        match self.create_page_target(&browser_ws, base).await {
            Ok(page_ws_url) => Ok(page_ws_url),
            Err(e) => {
                // If creation fails for any reason, fall back to browser endpoint.
                // This is a degraded mode — Page.* commands will fail.
                warn!(error = %e, "failed to create page target, falling back to browser endpoint");
                Ok(browser_ws)
            }
        }
    }

    /// Create a new page target using `Target.createTarget` over a temporary
    /// browser-level WebSocket connection, then return the page-level WS URL.
    async fn create_page_target(
        &self,
        browser_ws_url: &str,
        http_base: &str,
    ) -> Result<String, CdpError> {
        // Open a temporary WebSocket to the browser endpoint.
        let (ws_stream, _) = tokio_tungstenite::connect_async(browser_ws_url)
            .await
            .map_err(|e| CdpError::ConnectionFailed(e.to_string()))?;

        let (mut writer, mut reader) = ws_stream.split();

        // Send Target.createTarget to create a blank page.
        let request = CdpRequest {
            id: 1,
            method: "Target.createTarget".into(),
            params: Some(serde_json::json!({ "url": "about:blank" })),
        };
        let json = serde_json::to_string(&request)?;
        writer
            .send(Message::Text(json.into()))
            .await
            .map_err(|e| CdpError::WebSocket(e.to_string()))?;

        // Wait for the response with the new target ID.
        let response_timeout = Duration::from_secs(10);
        let target_id = match timeout(response_timeout, async {
            while let Some(msg) = reader.next().await {
                let text = match msg {
                    Ok(Message::Text(t)) => t.to_string(),
                    Ok(Message::Close(_)) => break,
                    Ok(_) => continue,
                    Err(e) => return Err(CdpError::WebSocket(e.to_string())),
                };
                let resp: CdpResponse = serde_json::from_str(&text)?;
                if resp.id == Some(1) {
                    if let Some(err) = resp.error {
                        return Err(CdpError::ProtocolError {
                            code: err.code,
                            message: err.message,
                        });
                    }
                    let target_id = resp
                        .result
                        .as_ref()
                        .and_then(|r| r.get("targetId"))
                        .and_then(|v| v.as_str())
                        .map(String::from)
                        .ok_or_else(|| {
                            CdpError::DiscoveryFailed(
                                "Target.createTarget returned no targetId".into(),
                            )
                        })?;
                    return Ok(target_id);
                }
            }
            Err(CdpError::DiscoveryFailed(
                "WebSocket closed before receiving Target.createTarget response".into(),
            ))
        })
        .await
        {
            Ok(result) => result?,
            Err(_) => {
                return Err(CdpError::Timeout(
                    u64::try_from(response_timeout.as_millis()).unwrap_or(u64::MAX),
                ))
            }
        };

        // Close the temporary connection.
        let _ = writer.close().await;

        debug!(target_id = %target_id, "created new page target");

        // Now look up the new page target's WS URL from /json/list.
        let list_url = format!("{http_base}/json/list");
        // Small delay to let Chrome register the new target.
        tokio::time::sleep(Duration::from_millis(200)).await;

        let resp = reqwest::get(&list_url)
            .await
            .map_err(|e| CdpError::DiscoveryFailed(format!("GET {list_url}: {e}")))?;
        let targets: Vec<CdpTarget> = resp
            .json()
            .await
            .map_err(|e| CdpError::DiscoveryFailed(format!("parse /json/list: {e}")))?;

        // Find our newly created target.
        let target = targets
            .iter()
            .find(|t| t.id == target_id && t.ws_url.is_some())
            .or_else(|| {
                // Fallback: any page target with a WS URL.
                targets
                    .iter()
                    .find(|t| t.target_type == "page" && t.ws_url.is_some())
            })
            .ok_or_else(|| {
                CdpError::DiscoveryFailed(format!(
                    "created target {target_id} but not found in /json/list"
                ))
            })?;

        let ws_url = target.ws_url.as_ref().unwrap();
        let cdp_url_owned2 = self.cdp_url.read().unwrap().clone();
        let http_url = cdp_url_owned2.trim();
        debug!(ws_url, "using newly created page target");
        Ok(normalize_ws_url(ws_url, http_url))
    }

    /// Get HTTP base URL for /json/* endpoints.
    fn http_base_url(&self) -> String {
        let cdp_url_owned3 = self.cdp_url.read().unwrap().clone();
        let url = cdp_url_owned3.trim().trim_end_matches('/');
        if url.starts_with("ws://") {
            url.replacen("ws://", "http://", 1)
        } else if url.starts_with("wss://") {
            url.replacen("wss://", "https://", 1)
        } else {
            url.to_string()
        }
    }

    /// Reader loop: receives WebSocket messages and dispatches responses.
    ///
    /// When the loop exits (connection lost, browser closed, etc.), it:
    /// 1. Marks the connection as dead via the `alive` flag.
    /// 2. Clears the writer so `is_connected()` returns false.
    /// 3. Drains all pending requests with `NotConnected` errors.
    async fn reader_loop(
        mut reader: futures_util::stream::SplitStream<
            tokio_tungstenite::WebSocketStream<
                tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
            >,
        >,
        pending: Arc<Mutex<PendingMap>>,
        event_tx: broadcast::Sender<CdpEvent>,
        writer: Arc<Mutex<Option<WriterHalf>>>,
        alive: Arc<AtomicBool>,
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

            // Events (no id) -- dispatch to subscribers.
            if resp.id.is_none() {
                if let Some(method) = resp.method {
                    let params = resp.params.unwrap_or(serde_json::Value::Null);
                    trace!(method = %method, "CDP event received");
                    let _ = event_tx.send(CdpEvent { method, params });
                }
                continue;
            }

            let id = resp.id.unwrap();
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

        // Connection lost: mark dead, clear writer, and drain pending.
        warn!("CDP WebSocket connection lost, marking as disconnected");
        alive.store(false, Ordering::SeqCst);
        writer.lock().await.take();
        let mut pending = pending.lock().await;
        for (_, sender) in pending.drain() {
            let _ = sender.send(Err(CdpError::NotConnected));
        }
    }

    /// Full health check: HTTP `/json/version` + WebSocket `Browser.getVersion`.
    ///
    /// This mirrors `OpenClaw`'s `isChromeCdpReady()` two-step probe:
    /// 1. HTTP `/json/version` to verify Chrome is responsive.
    /// 2. Open a temporary WebSocket to the browser endpoint and send
    ///    `Browser.getVersion` to verify the CDP pipe is functional.
    ///
    /// Returns `true` only if both steps succeed within their timeouts.
    ///
    /// # Panics
    ///
    /// Panics if the internal `cdp_url` lock is poisoned.
    pub async fn health_check(&self) -> bool {
        let cdp_url_owned = self.cdp_url.read().unwrap().clone();
        let url = cdp_url_owned.trim();

        // For direct WS URLs, try a WS-only probe.
        if url.starts_with("ws://") || url.starts_with("wss://") {
            return ws_health_probe(url, timeouts::HEALTH_WS_TIMEOUT).await;
        }

        health_check_http(url).await
    }
}

/// Standalone HTTP+WS health probe for a CDP endpoint.
///
/// Useful before establishing a persistent connection (e.g. during
/// `ensure_connected()`) to verify that Chrome is actually alive.
pub async fn health_check_http(base_url: &str) -> bool {
    let base = base_url.trim().trim_end_matches('/');
    let version_url = format!("{base}/json/version");

    // Step 1: HTTP /json/version
    let version_resp =
        match timeout(timeouts::HEALTH_HTTP_TIMEOUT, reqwest::get(&version_url)).await {
            Ok(Ok(resp)) if resp.status().is_success() => match resp.json::<VersionInfo>().await {
                Ok(info) => info,
                Err(_) => return false,
            },
            _ => return false,
        };

    // Step 2: Extract browser WS URL and run a WS health command.
    let ws_url = match version_resp
        .web_socket_debugger_url
        .filter(|s| !s.is_empty())
    {
        Some(url) => normalize_ws_url(&url, base),
        None => return true, // HTTP responded but no WS URL -- partial health
    };

    ws_health_probe(&ws_url, timeouts::HEALTH_WS_TIMEOUT).await
}

/// Open a temporary WebSocket and send `Browser.getVersion` as a health
/// probe, mirroring `OpenClaw`'s `canRunCdpHealthCommand()`.
async fn ws_health_probe(ws_url: &str, ws_timeout: Duration) -> bool {
    let Ok(Ok((connect_result, _))) =
        timeout(ws_timeout, tokio_tungstenite::connect_async(ws_url)).await
    else {
        return false;
    };

    let (mut writer, mut reader) = connect_result.split();

    let cmd = serde_json::json!({
        "id": 1,
        "method": "Browser.getVersion"
    });

    if writer
        .send(Message::Text(cmd.to_string().into()))
        .await
        .is_err()
    {
        return false;
    }

    // Wait for response with id=1.
    let result = timeout(ws_timeout, async {
        while let Some(msg) = reader.next().await {
            let text = match msg {
                Ok(Message::Text(t)) => t.to_string(),
                Ok(Message::Close(_)) | Err(_) => break,
                _ => continue,
            };
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&text) {
                if parsed.get("id") == Some(&serde_json::json!(1)) {
                    return parsed.get("result").is_some();
                }
            }
        }
        false
    })
    .await;

    let _ = writer.close().await;
    result.unwrap_or(false)
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
    let is_cdp_loopback = cdp_host == "127.0.0.1" || cdp_host == "localhost";

    if is_ws_loopback && !is_cdp_loopback {
        let _ = ws.set_host(Some(cdp_host));
        if let Some(port) = cdp.port() {
            let _ = ws.set_port(Some(port));
        }
    }

    ws.to_string()
}
