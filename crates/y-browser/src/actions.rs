//! High-level browser actions built on top of `CdpClient`.
//!
//! Each action maps to one or more CDP commands and returns
//! structured results.
//!
//! Key concept: **element refs** (`@e1`, `@e2`, ...) assigned during
//! `snapshot_aria` are backed by CDP `backendDOMNodeId` values. Actions
//! like `click` and `type_text` accept both CSS selectors and `@eN` refs.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use tokio::sync::Mutex;
use tracing::debug;

use crate::cdp_client::{CdpClient, CdpError, CdpEvent};
use crate::snapshot::{
    format_aria_snapshot, AriaSnapshotNode, DomSnapshotNode, RawAxNode, SnapshotFormat,
};

/// Result of a navigate action.
#[derive(Debug, Clone, Serialize)]
pub struct NavigateResult {
    pub url: String,
    pub frame_id: Option<String>,
}

/// Result of a screenshot action.
#[derive(Debug, Clone, Serialize)]
pub struct ScreenshotResult {
    /// Base64-encoded image data.
    pub data_base64: String,
    /// Image format.
    pub format: String,
}

/// Result of a JavaScript evaluation.
#[derive(Debug, Clone, Serialize)]
pub struct EvalResult {
    pub value: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exception: Option<String>,
}

/// Result of a snapshot action.
#[derive(Debug, Clone, Serialize)]
pub struct SnapshotResult {
    pub format: SnapshotFormat,
    pub nodes_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aria_nodes: Option<Vec<AriaSnapshotNode>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dom_nodes: Option<Vec<DomSnapshotNode>>,
    pub text: String,
}

/// A console log entry captured from the browser.
#[derive(Debug, Clone, Serialize)]
pub struct ConsoleEntry {
    pub level: String,
    pub text: String,
}

/// High-level browser actions.
pub struct BrowserActions {
    client: Arc<CdpClient>,
    /// Ref registry: maps ref IDs (e.g. "e1") to CDP `backendDOMNodeId`.
    /// Updated on each snapshot. Old refs are invalidated.
    ref_registry: Arc<Mutex<HashMap<String, i64>>>,
    /// Console log buffer, populated by the event listener.
    console_logs: Arc<Mutex<Vec<ConsoleEntry>>>,
    /// Handle to the console listener task.
    console_listener: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl BrowserActions {
    pub fn new(client: Arc<CdpClient>) -> Self {
        Self {
            client,
            ref_registry: Arc::new(Mutex::new(HashMap::new())),
            console_logs: Arc::new(Mutex::new(Vec::new())),
            console_listener: Mutex::new(None),
        }
    }

    /// Start listening for console-related CDP events.
    /// Call this after connecting to CDP.
    pub async fn enable_console_monitoring(&self) {
        // Enable the Runtime domain to receive console events.
        let _ = self.client.send("Runtime.enable", None).await;

        let mut rx = self.client.subscribe_events();
        let logs = Arc::clone(&self.console_logs);

        let handle = tokio::spawn(async move {
            while let Ok(event) = rx.recv().await {
                match event.method.as_str() {
                    "Runtime.consoleAPICalled" => {
                        let level = event
                            .params
                            .get("type")
                            .and_then(|v| v.as_str())
                            .unwrap_or("log")
                            .to_string();
                        let text = extract_console_text(&event);
                        let mut buf = logs.lock().await;
                        // Cap buffer at 100 entries to avoid memory issues
                        if buf.len() >= 100 {
                            buf.remove(0);
                        }
                        buf.push(ConsoleEntry { level, text });
                    }
                    "Runtime.exceptionThrown" => {
                        let text = event
                            .params
                            .get("exceptionDetails")
                            .and_then(|d| d.get("text"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("Unknown exception")
                            .to_string();
                        let mut buf = logs.lock().await;
                        if buf.len() >= 100 {
                            buf.remove(0);
                        }
                        buf.push(ConsoleEntry {
                            level: "error".into(),
                            text,
                        });
                    }
                    _ => {} // Ignore other events
                }
            }
        });

        *self.console_listener.lock().await = Some(handle);
    }

    /// Get and drain captured console logs.
    pub async fn take_console_logs(&self) -> Vec<ConsoleEntry> {
        let mut logs = self.console_logs.lock().await;
        std::mem::take(&mut *logs)
    }

    /// Get captured console logs without draining.
    pub async fn peek_console_logs(&self) -> Vec<ConsoleEntry> {
        self.console_logs.lock().await.clone()
    }

    /// Navigate to a URL.
    pub async fn navigate(&self, url: &str) -> Result<NavigateResult, CdpError> {
        debug!(url, "browser navigate");

        // Ensure the Page domain is enabled.
        let _ = self.client.send("Page.enable", None).await;

        let result = self
            .client
            .send("Page.navigate", Some(serde_json::json!({ "url": url })))
            .await?;

        // Invalidate refs after navigation.
        self.ref_registry.lock().await.clear();

        Ok(NavigateResult {
            url: url.to_string(),
            frame_id: result
                .get("frameId")
                .and_then(|v| v.as_str())
                .map(String::from),
        })
    }

    /// Capture a screenshot.
    pub async fn screenshot(
        &self,
        full_page: bool,
        format: &str,
        quality: Option<u32>,
    ) -> Result<ScreenshotResult, CdpError> {
        debug!(full_page, format, "browser screenshot");

        // Enable Page domain if needed.
        let _ = self.client.send("Page.enable", None).await;

        let mut params = serde_json::json!({
            "format": format,
            "fromSurface": true,
            "captureBeyondViewport": true,
        });

        if let Some(q) = quality {
            if format == "jpeg" {
                params["quality"] = serde_json::json!(q.min(100));
            }
        }

        if full_page {
            // Get full page dimensions.
            let metrics = self.client.send("Page.getLayoutMetrics", None).await?;

            let content_size = metrics
                .get("cssContentSize")
                .or_else(|| metrics.get("contentSize"));

            if let Some(size) = content_size {
                let width = size.get("width").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let height = size.get("height").and_then(|v| v.as_f64()).unwrap_or(0.0);
                if width > 0.0 && height > 0.0 {
                    params["clip"] = serde_json::json!({
                        "x": 0,
                        "y": 0,
                        "width": width,
                        "height": height,
                        "scale": 1,
                    });
                }
            }
        }

        let result = self
            .client
            .send("Page.captureScreenshot", Some(params))
            .await?;

        let data =
            result
                .get("data")
                .and_then(|v| v.as_str())
                .ok_or_else(|| CdpError::ProtocolError {
                    code: -1,
                    message: "screenshot returned no data".into(),
                })?;

        Ok(ScreenshotResult {
            data_base64: data.to_string(),
            format: format.to_string(),
        })
    }

    /// Evaluate JavaScript in the page context.
    pub async fn evaluate(&self, expression: &str) -> Result<EvalResult, CdpError> {
        debug!(expression_len = expression.len(), "browser evaluate");

        let _ = self.client.send("Runtime.enable", None).await;

        let result = self
            .client
            .send(
                "Runtime.evaluate",
                Some(serde_json::json!({
                    "expression": expression,
                    "awaitPromise": true,
                    "returnByValue": true,
                    "userGesture": true,
                })),
            )
            .await?;

        let value = result
            .get("result")
            .and_then(|r| r.get("value"))
            .cloned()
            .unwrap_or(serde_json::Value::Null);

        let exception = result
            .get("exceptionDetails")
            .and_then(|e| e.get("text"))
            .and_then(|t| t.as_str())
            .map(String::from);

        Ok(EvalResult { value, exception })
    }

    /// Get the current page URL.
    pub async fn get_url(&self) -> Result<String, CdpError> {
        let result = self.evaluate("window.location.href").await?;
        Ok(result.value.as_str().unwrap_or_default().to_string())
    }

    /// Get the current page title.
    pub async fn get_title(&self) -> Result<String, CdpError> {
        let result = self.evaluate("document.title").await?;
        Ok(result.value.as_str().unwrap_or_default().to_string())
    }

    /// Get text content of an element by CSS selector or `@eN` ref.
    pub async fn get_text(&self, selector: &str) -> Result<String, CdpError> {
        if let Some(ref_id) = selector.strip_prefix('@') {
            let object_id = self.resolve_ref_to_object_id(ref_id).await?;
            let result = self
                .client
                .send(
                    "Runtime.callFunctionOn",
                    Some(serde_json::json!({
                        "objectId": object_id,
                        "functionDeclaration": "function() { return (this.innerText || this.textContent || '').trim(); }",
                        "returnByValue": true,
                    })),
                )
                .await?;
            let text = result
                .get("result")
                .and_then(|r| r.get("value"))
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            Ok(text.to_string())
        } else {
            let js = format!(
                r#"(() => {{
                    const el = document.querySelector({});
                    return el ? (el.innerText || el.textContent || '').trim() : null;
                }})()"#,
                serde_json::to_string(selector).unwrap_or_default()
            );
            let result = self.evaluate(&js).await?;
            Ok(result.value.as_str().unwrap_or_default().to_string())
        }
    }

    /// Click an element by CSS selector or `@eN` ref.
    pub async fn click(&self, selector: &str) -> Result<(), CdpError> {
        debug!(selector, "browser click");

        if let Some(ref_id) = selector.strip_prefix('@') {
            let object_id = self.resolve_ref_to_object_id(ref_id).await?;
            // scrollIntoView + click via callFunctionOn
            let result = self
                .client
                .send(
                    "Runtime.callFunctionOn",
                    Some(serde_json::json!({
                        "objectId": object_id,
                        "functionDeclaration": "function() { this.scrollIntoView({block:'center'}); this.click(); return true; }",
                        "returnByValue": true,
                        "userGesture": true,
                    })),
                )
                .await?;
            if let Some(err) = result.get("exceptionDetails") {
                let msg = err
                    .get("text")
                    .and_then(|v| v.as_str())
                    .unwrap_or("click failed");
                return Err(CdpError::ProtocolError {
                    code: -1,
                    message: msg.to_string(),
                });
            }
            Ok(())
        } else {
            let js = format!(
                r#"(() => {{
                    const el = document.querySelector({});
                    if (!el) throw new Error('Element not found: ' + {});
                    el.scrollIntoView({{ block: 'center' }});
                    el.click();
                    return true;
                }})()"#,
                serde_json::to_string(selector).unwrap_or_default(),
                serde_json::to_string(selector).unwrap_or_default(),
            );
            let result = self.evaluate(&js).await?;
            if let Some(err) = result.exception {
                return Err(CdpError::ProtocolError {
                    code: -1,
                    message: err,
                });
            }
            Ok(())
        }
    }

    /// Type text into an element by CSS selector or `@eN` ref.
    pub async fn type_text(&self, selector: &str, text: &str) -> Result<(), CdpError> {
        debug!(selector, text_len = text.len(), "browser type");

        if let Some(ref_id) = selector.strip_prefix('@') {
            let object_id = self.resolve_ref_to_object_id(ref_id).await?;
            let text_json = serde_json::to_string(text).unwrap_or_default();
            let fn_decl = format!(
                "function() {{ this.scrollIntoView({{block:'center'}}); this.focus(); this.value = {text_json}; this.dispatchEvent(new Event('input', {{bubbles:true}})); this.dispatchEvent(new Event('change', {{bubbles:true}})); return true; }}"
            );
            let result = self
                .client
                .send(
                    "Runtime.callFunctionOn",
                    Some(serde_json::json!({
                        "objectId": object_id,
                        "functionDeclaration": fn_decl,
                        "returnByValue": true,
                        "userGesture": true,
                    })),
                )
                .await?;
            if let Some(err) = result.get("exceptionDetails") {
                let msg = err
                    .get("text")
                    .and_then(|v| v.as_str())
                    .unwrap_or("type failed");
                return Err(CdpError::ProtocolError {
                    code: -1,
                    message: msg.to_string(),
                });
            }
            Ok(())
        } else {
            let js = format!(
                r#"(() => {{
                    const el = document.querySelector({});
                    if (!el) throw new Error('Element not found: ' + {});
                    el.scrollIntoView({{ block: 'center' }});
                    el.focus();
                    el.value = {};
                    el.dispatchEvent(new Event('input', {{ bubbles: true }}));
                    el.dispatchEvent(new Event('change', {{ bubbles: true }}));
                    return true;
                }})()"#,
                serde_json::to_string(selector).unwrap_or_default(),
                serde_json::to_string(selector).unwrap_or_default(),
                serde_json::to_string(text).unwrap_or_default(),
            );
            let result = self.evaluate(&js).await?;
            if let Some(err) = result.exception {
                return Err(CdpError::ProtocolError {
                    code: -1,
                    message: err,
                });
            }
            Ok(())
        }
    }

    /// Press a keyboard key via CDP Input domain.
    pub async fn press_key(&self, key: &str) -> Result<(), CdpError> {
        debug!(key, "browser press_key");
        self.client
            .send(
                "Input.dispatchKeyEvent",
                Some(serde_json::json!({
                    "type": "keyDown",
                    "key": key,
                })),
            )
            .await?;
        self.client
            .send(
                "Input.dispatchKeyEvent",
                Some(serde_json::json!({
                    "type": "keyUp",
                    "key": key,
                })),
            )
            .await?;
        Ok(())
    }

    /// Scroll the page.
    pub async fn scroll(&self, direction: &str, pixels: u32) -> Result<(), CdpError> {
        debug!(direction, pixels, "browser scroll");
        let (dx, dy) = match direction {
            "up" => (0, -(pixels as i32)),
            "down" => (0, pixels as i32),
            "left" => (-(pixels as i32), 0),
            "right" => (pixels as i32, 0),
            _ => (0, pixels as i32),
        };

        let js = format!("window.scrollBy({dx}, {dy})");
        self.evaluate(&js).await?;
        Ok(())
    }

    /// Wait for a condition: time (ms), or selector to appear.
    pub async fn wait(&self, selector: Option<&str>, ms: Option<u64>) -> Result<(), CdpError> {
        if let Some(ms) = ms {
            debug!(ms, "browser wait (time)");
            tokio::time::sleep(Duration::from_millis(ms)).await;
            return Ok(());
        }

        if let Some(selector) = selector {
            debug!(selector, "browser wait (selector)");
            let timeout_ms = 10_000u64;
            let poll_ms = 200u64;
            let start = std::time::Instant::now();

            loop {
                let js = format!(
                    "!!document.querySelector({})",
                    serde_json::to_string(selector).unwrap_or_default()
                );
                let result = self.evaluate(&js).await?;
                if result.value.as_bool().unwrap_or(false) {
                    return Ok(());
                }

                if start.elapsed() > Duration::from_millis(timeout_ms) {
                    return Err(CdpError::Timeout(timeout_ms));
                }

                tokio::time::sleep(Duration::from_millis(poll_ms)).await;
            }
        }

        Ok(())
    }

    /// Take an accessibility snapshot.
    ///
    /// When `interactive_only` is true, only interactive elements and their
    /// structural ancestors are included (dramatically fewer tokens).
    pub async fn snapshot_aria(
        &self,
        limit: usize,
        interactive_only: bool,
    ) -> Result<SnapshotResult, CdpError> {
        debug!(limit, interactive_only, "browser snapshot (aria)");

        // Enable DOM domain (needed for resolving refs later).
        let _ = self.client.send("DOM.enable", None).await;
        let _ = self.client.send("Accessibility.enable", None).await;

        let result = self
            .client
            .send("Accessibility.getFullAXTree", None)
            .await?;

        let raw_nodes: Vec<RawAxNode> = result
            .get("nodes")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let aria_nodes = format_aria_snapshot(&raw_nodes, limit, interactive_only);

        // Update ref registry with new refs.
        {
            let mut registry = self.ref_registry.lock().await;
            registry.clear();
            for node in &aria_nodes {
                if let Some(backend_id) = node.backend_dom_node_id {
                    registry.insert(node.ref_id.clone(), backend_id);
                }
            }
            debug!(ref_count = registry.len(), "ref registry updated");
        }

        let text = crate::snapshot::aria_snapshot_to_text(&aria_nodes);

        Ok(SnapshotResult {
            format: SnapshotFormat::Aria,
            nodes_count: aria_nodes.len(),
            aria_nodes: Some(aria_nodes),
            dom_nodes: None,
            text,
        })
    }

    /// Take a DOM snapshot via JavaScript.
    pub async fn snapshot_dom(
        &self,
        limit: usize,
        max_text_chars: usize,
    ) -> Result<SnapshotResult, CdpError> {
        debug!(limit, max_text_chars, "browser snapshot (dom)");

        let js = format!(
            r#"(() => {{
                const maxNodes = {limit};
                const maxText = {max_text_chars};
                const nodes = [];
                const root = document.documentElement;
                if (!root) return {{ nodes }};
                const stack = [{{ el: root, depth: 0, parentRef: null }}];
                while (stack.length && nodes.length < maxNodes) {{
                    const cur = stack.pop();
                    const el = cur.el;
                    if (!el || el.nodeType !== 1) continue;
                    const ref_id = "n" + String(nodes.length + 1);
                    const tag = (el.tagName || "").toLowerCase();
                    const id = el.id ? String(el.id) : undefined;
                    const className = el.className ? String(el.className).slice(0, 300) : undefined;
                    const role = el.getAttribute && el.getAttribute("role") ? String(el.getAttribute("role")) : undefined;
                    let text = "";
                    try {{ text = String(el.innerText || "").trim(); }} catch {{}}
                    if (maxText && text.length > maxText) text = text.slice(0, maxText) + "…";
                    const href = el.href != null ? String(el.href) : undefined;
                    nodes.push({{
                        ref_id,
                        parent_ref: cur.parentRef,
                        depth: cur.depth,
                        tag,
                        ...(id ? {{ id }} : {{}}),
                        ...(className ? {{ class_name: className }} : {{}}),
                        ...(role ? {{ role }} : {{}}),
                        ...(text ? {{ text }} : {{}}),
                        ...(href ? {{ href }} : {{}}),
                    }});
                    const children = el.children ? Array.from(el.children) : [];
                    for (let i = children.length - 1; i >= 0; i--) {{
                        stack.push({{ el: children[i], depth: cur.depth + 1, parentRef: ref_id }});
                    }}
                }}
                return {{ nodes }};
            }})()"#
        );

        let result = self.evaluate(&js).await?;
        let dom_nodes: Vec<DomSnapshotNode> = result
            .value
            .get("nodes")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let text = dom_nodes
            .iter()
            .map(|n| {
                let indent = "  ".repeat(n.depth);
                let mut line = format!("{indent}<{}>", n.tag);
                if let Some(id) = &n.id {
                    line.push_str(&format!(" id=\"{id}\""));
                }
                if let Some(text) = &n.text {
                    if text.len() <= 80 {
                        line.push_str(&format!(" \"{text}\""));
                    }
                }
                line
            })
            .collect::<Vec<_>>()
            .join("\n");

        Ok(SnapshotResult {
            format: SnapshotFormat::Dom,
            nodes_count: dom_nodes.len(),
            aria_nodes: None,
            dom_nodes: Some(dom_nodes),
            text,
        })
    }

    /// Get page text content (inner text of body).
    pub async fn get_page_text(&self) -> Result<String, CdpError> {
        let result = self
            .evaluate("document.body ? document.body.innerText : ''")
            .await?;
        Ok(result.value.as_str().unwrap_or_default().to_string())
    }

    /// Helper: resolve a ref id (e.g. "e1") to a CDP RemoteObject objectId.
    async fn resolve_ref_to_object_id(&self, ref_id: &str) -> Result<String, CdpError> {
        let registry = self.ref_registry.lock().await;
        let backend_id = registry
            .get(ref_id)
            .ok_or_else(|| CdpError::ProtocolError {
                code: -1,
                message: format!(
                    "Ref '@{}' not found. {}Run 'snapshot' to get fresh refs.",
                    ref_id,
                    if registry.is_empty() {
                        "No refs available. "
                    } else {
                        ""
                    }
                ),
            })?;
        let backend_id = *backend_id;
        drop(registry);

        // Enable DOM domain if not yet enabled.
        let _ = self.client.send("DOM.enable", None).await;

        let resolve_result = self
            .client
            .send(
                "DOM.resolveNode",
                Some(serde_json::json!({ "backendNodeId": backend_id })),
            )
            .await?;

        resolve_result
            .get("object")
            .and_then(|o| o.get("objectId"))
            .and_then(|v| v.as_str())
            .map(String::from)
            .ok_or_else(|| CdpError::ProtocolError {
                code: -1,
                message: format!(
                    "Element @{} no longer exists in the DOM. The page may have changed — re-run snapshot.",
                    ref_id
                ),
            })
    }
}

/// Extract human-readable text from a `Runtime.consoleAPICalled` event.
fn extract_console_text(event: &CdpEvent) -> String {
    event
        .params
        .get("args")
        .and_then(|args| args.as_array())
        .map(|args| {
            args.iter()
                .filter_map(|arg| {
                    arg.get("value")
                        .and_then(|v| match v {
                            serde_json::Value::String(s) => Some(s.clone()),
                            other => Some(other.to_string()),
                        })
                        .or_else(|| {
                            arg.get("description")
                                .and_then(|v| v.as_str())
                                .map(String::from)
                        })
                })
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default()
}
