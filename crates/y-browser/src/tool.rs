//! Browser tool implementing `y-core::tool::Tool`.
//!
//! Exposes browser automation as a single unified tool for the agent.
//! Key workflow: snapshot → get refs → click/type with refs.

use std::sync::Arc;
use std::sync::RwLock;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tracing::debug;

use y_core::runtime::RuntimeCapability;
use y_core::tool::{
    Tool, ToolCategory, ToolDefinition, ToolError, ToolInput, ToolOutput, ToolType,
};
use y_core::types::ToolName;

use crate::actions::BrowserActions;
use crate::cdp_client::CdpClient;
use crate::config::BrowserConfig;
use crate::launcher::ChromeLauncher;
use crate::security::SecurityPolicy;
use crate::snapshot::{truncate_output, SnapshotFormat};

/// Maximum output characters before truncation.
const MAX_OUTPUT_CHARS: usize = 50_000;

/// Supported browser actions (parsed from tool input).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserAction {
    Navigate,
    Search,
    Screenshot,
    Snapshot,
    Click,
    Type,
    GetText,
    GetTitle,
    GetUrl,
    Evaluate,
    Wait,
    PressKey,
    Scroll,
    GetPageText,
    GetConsoleLogs,
    Close,
}

impl BrowserAction {
    fn from_str(s: &str) -> Option<Self> {
        match s {
            // Navigation
            "navigate" | "open" | "goto" | "go" | "load" | "visit" | "open_url" | "go_to" => {
                Some(Self::Navigate)
            }
            // Search via search engine
            "search" | "web_search" | "google" | "find" | "lookup" | "query" => Some(Self::Search),
            // Screenshot
            "screenshot" | "capture" | "screen" | "take_screenshot" | "capture_screenshot" => {
                Some(Self::Screenshot)
            }
            // Snapshot (accessibility tree)
            "snapshot" | "inspect" | "get_elements" | "list_elements" | "dom" | "get_dom"
            | "accessibility" | "a11y" => Some(Self::Snapshot),
            // Click
            "click" | "tap" | "press_button" | "click_element" => Some(Self::Click),
            // Type / fill text
            "type" | "type_text" | "fill" | "input" | "enter_text" | "set_value" | "fill_text"
            | "input_text" | "write" => Some(Self::Type),
            // Get element text
            "get_text" | "read_text" | "text" | "element_text" | "get_element_text"
            | "extract_text" => Some(Self::GetText),
            // Get title
            "get_title" | "title" | "page_title" => Some(Self::GetTitle),
            // Get URL
            "get_url" | "url" | "current_url" | "page_url" => Some(Self::GetUrl),
            // Evaluate JS
            "evaluate" | "eval" | "execute" | "exec" | "run_js" | "javascript" | "js"
            | "execute_script" | "run_script" | "eval_js" => Some(Self::Evaluate),
            // Wait
            "wait" | "sleep" | "delay" | "wait_for" | "pause" => Some(Self::Wait),
            // Press key
            "press_key" | "press" | "key" | "keypress" | "send_key" | "keyboard" => {
                Some(Self::PressKey)
            }
            // Scroll
            "scroll" | "scroll_page" | "scroll_down" | "scroll_up" => Some(Self::Scroll),
            // Get full page text
            "get_page_text" | "page_text" | "get_content" | "get_page_content" | "read_page"
            | "page_content" | "body_text" | "get_body" | "read" | "read_content" | "extract"
            | "scrape" => Some(Self::GetPageText),
            // Console logs
            "get_console_logs" | "console" | "console_logs" | "logs" | "get_logs"
            | "get_errors" | "errors" => Some(Self::GetConsoleLogs),
            // Close
            "close" | "quit" | "exit" | "disconnect" | "stop" => Some(Self::Close),
            _ => None,
        }
    }

    /// All valid action names, for error messages.
    fn all_names() -> &'static str {
        "navigate, search, screenshot, snapshot, click, type, get_text, get_title, get_url, evaluate, wait, press_key, scroll, get_page_text, get_console_logs, close"
    }
}

/// Browser tool for agent integration.
pub struct BrowserTool {
    def: ToolDefinition,
    config: RwLock<BrowserConfig>,
    client: Arc<CdpClient>,
    actions: BrowserActions,
    security: RwLock<SecurityPolicy>,
    /// Locally launched Chrome process (if `auto_launch` is enabled).
    launcher: Mutex<Option<ChromeLauncher>>,
    /// Whether console monitoring has been started.
    console_started: Mutex<bool>,
}

impl BrowserTool {
    /// Create a new browser tool with the given configuration.
    pub fn new(config: BrowserConfig) -> Self {
        // When auto_launch is true, the CDP URL is determined at launch time.
        // Use the local port for the client; the launcher will spawn Chrome there.
        let cdp_url = if config.launch_mode.is_auto_launch() {
            format!("http://127.0.0.1:{}", config.local_cdp_port)
        } else {
            config.cdp_url.clone()
        };

        let client = Arc::new(CdpClient::new(
            cdp_url,
            Duration::from_millis(config.timeout_ms),
        ));
        let actions = BrowserActions::new(Arc::clone(&client));
        let security = SecurityPolicy::new(
            config.allowed_domains.clone(),
            config.block_private_networks,
        );

        Self {
            def: Self::tool_definition(),
            config: RwLock::new(config),
            client,
            actions,
            security: RwLock::new(security),
            launcher: Mutex::new(None),
            console_started: Mutex::new(false),
        }
    }

    /// Hot-reload the browser configuration.
    ///
    /// Updates the stored config and rebuilds the security policy.
    /// Does NOT affect an already-running Chrome session; changes
    /// take effect on the next connection.
    ///
    /// # Panics
    ///
    /// Panics if the internal locks are poisoned.
    pub fn reload_config(&self, new_config: BrowserConfig) {
        let new_security = SecurityPolicy::new(
            new_config.allowed_domains.clone(),
            new_config.block_private_networks,
        );
        *self.security.write().unwrap() = new_security;
        *self.config.write().unwrap() = new_config;
        tracing::info!("Browser config hot-reloaded");
    }

    /// Get the tool definition.
    pub fn tool_definition() -> ToolDefinition {
        ToolDefinition {
            name: ToolName::from_string("browser"),
            description: concat!(
                "Control a web browser. ",
                "WORKFLOW: (1) navigate/search, (2) snapshot to get @refs, (3) click/type using @refs.\n\n",
                "Actions:\n",
                "- navigate: Open a URL. Args: url (required)\n",
                "- search: Search via search engine. Args: query (required), search_engine ('google'|'bing'|'duckduckgo'|'baidu', default 'google')\n",
                "- snapshot: Get page elements with @eN refs. Use these refs for click/type. Args: format ('aria'|'dom'), interactive_only (bool, default true)\n",
                "- click: Click an element. Args: selector ('@e1' ref from snapshot, or CSS selector)\n",
                "- type: Type text into an input. Args: selector ('@e1' ref or CSS), text (required)\n",
                "- screenshot: Capture page image. Args: full_page (bool)\n",
                "- get_text: Get element text. Args: selector ('@e1' ref or CSS)\n",
                "- get_title: Get page title\n",
                "- get_url: Get current URL\n",
                "- evaluate: Run JavaScript. Args: expression (required)\n",
                "- wait: Wait for condition. Args: selector (CSS to wait for) or ms (milliseconds)\n",
                "- press_key: Press keyboard key. Args: key ('Enter', 'Tab', 'Escape', etc.)\n",
                "- scroll: Scroll page. Args: direction ('up'|'down'|'left'|'right'), pixels (default 300)\n",
                "- get_page_text: Get all visible page text\n",
                "- get_console_logs: Get browser console output (errors, warnings, logs)\n",
                "- close: Close browser\n\n",
                "IMPORTANT: After 'snapshot', use the @eN refs (e.g. @e3) for click/type -- do NOT guess CSS selectors.",
            ).into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": [
                            "navigate", "search", "screenshot", "snapshot",
                            "click", "type", "get_text", "get_title", "get_url",
                            "evaluate", "wait", "press_key", "scroll",
                            "get_page_text", "get_console_logs", "close"
                        ],
                        "description": "Browser action to perform"
                    },
                    "url": {
                        "type": "string",
                        "description": "URL to navigate to (for 'navigate')"
                    },
                    "query": {
                        "type": "string",
                        "description": "Search query text (for 'search')"
                    },
                    "search_engine": {
                        "type": "string",
                        "enum": ["google", "bing", "duckduckgo", "baidu"],
                        "description": "Search engine to use (default: 'google')"
                    },
                    "selector": {
                        "type": "string",
                        "description": "Element ref from snapshot (e.g. '@e1') or CSS selector (for click/type/get_text)"
                    },
                    "text": {
                        "type": "string",
                        "description": "Text to type (for 'type')"
                    },
                    "expression": {
                        "type": "string",
                        "description": "JavaScript expression (for 'evaluate')"
                    },
                    "full_page": {
                        "type": "boolean",
                        "description": "Capture full page screenshot (default: false)"
                    },
                    "format": {
                        "type": "string",
                        "enum": ["aria", "dom"],
                        "description": "Snapshot format: 'aria' (accessibility, default) or 'dom' (HTML tree)"
                    },
                    "interactive_only": {
                        "type": "boolean",
                        "description": "Snapshot only interactive elements like buttons, links, inputs (default: true, much fewer tokens)"
                    },
                    "key": {
                        "type": "string",
                        "description": "Key to press (Enter, Tab, Escape, etc.)"
                    },
                    "direction": {
                        "type": "string",
                        "enum": ["up", "down", "left", "right"],
                        "description": "Scroll direction"
                    },
                    "pixels": {
                        "type": "integer",
                        "description": "Pixels to scroll (default: 300)"
                    },
                    "ms": {
                        "type": "integer",
                        "description": "Milliseconds to wait"
                    }
                },
                "required": ["action"]
            }),
            result_schema: None,
            category: ToolCategory::Network,
            tool_type: ToolType::BuiltIn,
            capabilities: RuntimeCapability::default(),
            is_dangerous: true, // Browser automation can navigate to arbitrary URLs
        }
    }

    /// Ensure CDP connection is established, connecting if needed.
    /// If `auto_launch` is enabled and no local Chrome is running, spawns one.
    async fn ensure_connected(&self) -> Result<(), ToolError> {
        if self.client.is_connected().await {
            return Ok(());
        }

        let config = self.config.read().unwrap().clone();

        // Auto-launch Chrome if configured.
        if config.launch_mode.is_auto_launch() {
            let mut launcher_guard = self.launcher.lock().await;
            if launcher_guard.is_none() {
                debug!("auto-launching Chrome");
                let chrome = ChromeLauncher::launch(
                    &config.chrome_path,
                    config.local_cdp_port,
                    config.launch_mode.is_headless(),
                )
                .await
                .map_err(|e| ToolError::ExternalServiceError {
                    name: "browser".into(),
                    message: format!("Failed to launch Chrome: {e}"),
                })?;
                *launcher_guard = Some(chrome);
            }

            // Use the launcher's actual port (may differ from config if
            // the configured port was already in use).
            let actual_port = launcher_guard.as_ref().unwrap().cdp_port();
            let cdp_url = format!("http://127.0.0.1:{actual_port}");

            debug!(cdp_url = %cdp_url, "connecting to CDP (auto-launched)");
            self.client.set_cdp_url(cdp_url.clone());
            self.client.connect().await.map_err(|e| {
                ToolError::ExternalServiceError {
                    name: "browser".into(),
                    message: format!(
                        "Failed to connect to Chrome CDP at '{cdp_url}': {e}. Chrome was auto-launched but CDP connection failed.",
                    ),
                }
            })?;
        } else {
            let cdp_url = &config.cdp_url;
            debug!(cdp_url = %cdp_url, "connecting to CDP");
            self.client.connect().await.map_err(|e| {
                ToolError::ExternalServiceError {
                    name: "browser".into(),
                    message: format!(
                        "Failed to connect to Chrome CDP at '{cdp_url}': {e}. Make sure Chrome is running with --remote-debugging-port=9222",
                    ),
                }
            })?;
        }

        // Start console monitoring on first connection.
        let mut started = self.console_started.lock().await;
        if !*started {
            self.actions.enable_console_monitoring().await;
            *started = true;
        }

        Ok(())
    }

    /// Shutdown the launcher and disconnect.
    async fn shutdown(&self) {
        self.client.disconnect().await;
        let mut launcher_guard = self.launcher.lock().await;
        if let Some(mut chrome) = launcher_guard.take() {
            chrome.shutdown().await;
        }
        *self.console_started.lock().await = false;
    }
    /// Handle the `search` action: build a search engine URL and navigate.
    async fn dispatch_search(&self, input: &ToolInput) -> Result<serde_json::Value, ToolError> {
        let query =
            require_str(&input.arguments, "query").map_err(|_| ToolError::ValidationError {
                message: "Missing 'query' parameter for search. Example: \
                    {\"action\": \"search\", \"query\": \"rust async tutorial\"}"
                    .into(),
            })?;

        let engine = input.arguments["search_engine"].as_str().map_or_else(
            || self.config.read().unwrap().default_search_engine.clone(),
            String::from,
        );

        let search_url = build_search_url(&engine, query)?;

        // Security check on the generated URL.
        self.security
            .read()
            .unwrap()
            .validate_url(&search_url)
            .map_err(|e| ToolError::PermissionDenied {
                name: "browser".into(),
                reason: e.to_string(),
            })?;

        let nav = self
            .actions
            .navigate(&search_url)
            .await
            .map_err(cdp_to_tool_error)?;
        Ok(serde_json::json!({
            "action": "search",
            "query": query,
            "search_engine": engine,
            "url": search_url,
            "navigation": serde_json::to_value(&nav).unwrap_or_default(),
        }))
    }

    /// Dispatch a browser action and return its JSON result.
    async fn dispatch_action(
        &self,
        action: BrowserAction,
        input: &ToolInput,
    ) -> Result<serde_json::Value, ToolError> {
        match action {
            BrowserAction::Navigate => {
                let url = input.arguments["url"]
                    .as_str()
                    .ok_or_else(|| ToolError::ValidationError {
                        message: "Missing 'url' parameter for navigate. Example: {\"action\": \"navigate\", \"url\": \"https://example.com\"}".into(),
                    })?;

                // Security check.
                self.security
                    .read()
                    .unwrap()
                    .validate_url(url)
                    .map_err(|e| ToolError::PermissionDenied {
                        name: "browser".into(),
                        reason: e.to_string(),
                    })?;

                let nav = self
                    .actions
                    .navigate(url)
                    .await
                    .map_err(cdp_to_tool_error)?;
                Ok(serde_json::to_value(nav).unwrap_or_default())
            }

            BrowserAction::Search => self.dispatch_search(input).await,

            BrowserAction::Screenshot => {
                let full_page = input.arguments["full_page"].as_bool().unwrap_or(false);
                let format = input.arguments["format"].as_str().unwrap_or("png");
                let quality = input.arguments["quality"]
                    .as_u64()
                    .map(|q| u32::try_from(q).unwrap_or(100));

                let shot = self
                    .actions
                    .screenshot(full_page, format, quality)
                    .await
                    .map_err(cdp_to_tool_error)?;
                Ok(serde_json::to_value(shot).unwrap_or_default())
            }

            BrowserAction::Snapshot => {
                let format = match input.arguments["format"].as_str() {
                    Some("dom") => SnapshotFormat::Dom,
                    _ => SnapshotFormat::Aria,
                };
                let limit = usize::try_from(input.arguments["limit"].as_u64().unwrap_or(500))
                    .unwrap_or(usize::MAX);
                let interactive_only = input.arguments["interactive_only"]
                    .as_bool()
                    .unwrap_or(true);

                let mut snap = match format {
                    SnapshotFormat::Aria => self
                        .actions
                        .snapshot_aria(limit, interactive_only)
                        .await
                        .map_err(cdp_to_tool_error)?,
                    SnapshotFormat::Dom => {
                        let max_text = usize::try_from(
                            input.arguments["max_text_chars"].as_u64().unwrap_or(220),
                        )
                        .unwrap_or(220);
                        self.actions
                            .snapshot_dom(limit, max_text)
                            .await
                            .map_err(cdp_to_tool_error)?
                    }
                };

                // Truncate large snapshot text.
                snap.text = truncate_output(&snap.text, MAX_OUTPUT_CHARS);
                Ok(serde_json::to_value(snap).unwrap_or_default())
            }

            BrowserAction::Click => {
                let selector = require_str(&input.arguments, "selector")
                    .map_err(|_| ToolError::ValidationError {
                        message: "Missing 'selector' for click. Use an @ref from snapshot (e.g. '@e1') or a CSS selector (e.g. '#submit-btn').".into(),
                    })?;
                self.actions
                    .click(selector)
                    .await
                    .map_err(cdp_to_tool_error)?;
                Ok(serde_json::json!({"action": "click", "selector": selector, "ok": true}))
            }

            BrowserAction::Type => {
                let selector = require_str(&input.arguments, "selector")
                    .map_err(|_| ToolError::ValidationError {
                        message: "Missing 'selector' for type. Use an @ref from snapshot (e.g. '@e3') or a CSS selector.".into(),
                    })?;
                let text = require_str(&input.arguments, "text")
                    .map_err(|_| ToolError::ValidationError {
                        message: "Missing 'text' for type. Example: {\"action\": \"type\", \"selector\": \"@e3\", \"text\": \"hello\"}".into(),
                    })?;
                self.actions
                    .type_text(selector, text)
                    .await
                    .map_err(cdp_to_tool_error)?;
                Ok(serde_json::json!({"action": "type", "selector": selector, "ok": true}))
            }

            BrowserAction::GetText => {
                let selector = require_str(&input.arguments, "selector")
                    .map_err(|_| ToolError::ValidationError {
                        message: "Missing 'selector' for get_text. Use an @ref (e.g. '@e5') or CSS selector.".into(),
                    })?;
                let text = self
                    .actions
                    .get_text(selector)
                    .await
                    .map_err(cdp_to_tool_error)?;
                let text = truncate_output(&text, MAX_OUTPUT_CHARS);
                Ok(serde_json::json!({"text": text, "selector": selector}))
            }

            BrowserAction::GetTitle => {
                let title = self.actions.get_title().await.map_err(cdp_to_tool_error)?;
                Ok(serde_json::json!({"title": title}))
            }

            BrowserAction::GetUrl => {
                let url = self.actions.get_url().await.map_err(cdp_to_tool_error)?;
                Ok(serde_json::json!({"url": url}))
            }

            BrowserAction::Evaluate => {
                let expression = require_str(&input.arguments, "expression")
                    .map_err(|_| ToolError::ValidationError {
                        message: "Missing 'expression' for evaluate. Example: {\"action\": \"evaluate\", \"expression\": \"document.title\"}".into(),
                    })?;
                let eval = self
                    .actions
                    .evaluate(expression)
                    .await
                    .map_err(cdp_to_tool_error)?;
                Ok(serde_json::to_value(eval).unwrap_or_default())
            }

            BrowserAction::Wait => {
                let selector = input.arguments["selector"].as_str();
                let ms = input.arguments["ms"].as_u64();
                self.actions
                    .wait(selector, ms)
                    .await
                    .map_err(cdp_to_tool_error)?;
                Ok(serde_json::json!({"action": "wait", "ok": true}))
            }

            BrowserAction::PressKey => {
                let key = require_str(&input.arguments, "key")
                    .map_err(|_| ToolError::ValidationError {
                        message: "Missing 'key' for press_key. Example: {\"action\": \"press_key\", \"key\": \"Enter\"}".into(),
                    })?;
                self.actions
                    .press_key(key)
                    .await
                    .map_err(cdp_to_tool_error)?;
                Ok(serde_json::json!({"action": "press_key", "key": key, "ok": true}))
            }

            BrowserAction::Scroll => {
                let direction = input.arguments["direction"].as_str().unwrap_or("down");
                let pixels =
                    u32::try_from(input.arguments["pixels"].as_u64().unwrap_or(300)).unwrap_or(300);
                self.actions
                    .scroll(direction, pixels)
                    .await
                    .map_err(cdp_to_tool_error)?;
                Ok(
                    serde_json::json!({"action": "scroll", "direction": direction, "pixels": pixels, "ok": true}),
                )
            }

            BrowserAction::GetPageText => {
                let text = self
                    .actions
                    .get_page_text()
                    .await
                    .map_err(cdp_to_tool_error)?;
                let text = truncate_output(&text, MAX_OUTPUT_CHARS);
                Ok(serde_json::json!({"text": text}))
            }

            BrowserAction::GetConsoleLogs => {
                let logs = self.actions.take_console_logs().await;
                Ok(serde_json::json!({
                    "logs": logs,
                    "count": logs.len(),
                }))
            }

            BrowserAction::Close => unreachable!(), // handled in execute
        }
    }

    /// Collect console errors/warnings for inclusion in tool output.
    async fn collect_console_warnings(&self) -> Vec<String> {
        self.actions
            .peek_console_logs()
            .await
            .iter()
            .filter(|l| l.level == "error" || l.level == "warning")
            .take(5) // limit to avoid flooding
            .map(|l| format!("[console.{}] {}", l.level, l.text))
            .collect()
    }
}

/// Build a search engine URL from engine name and query text.
fn build_search_url(engine: &str, query: &str) -> Result<String, ToolError> {
    let encoded_query = urlencoding::encode(query);
    let url = match engine {
        "google" => format!("https://www.google.com/search?q={encoded_query}"),
        "bing" => format!("https://www.bing.com/search?q={encoded_query}"),
        "duckduckgo" | "ddg" => format!("https://duckduckgo.com/?q={encoded_query}"),
        "baidu" => format!("https://www.baidu.com/s?wd={encoded_query}"),
        other => {
            return Err(ToolError::ValidationError {
                message: format!(
                    "Unknown search engine: '{other}'. \
                     Supported: google, bing, duckduckgo, baidu"
                ),
            });
        }
    };
    Ok(url)
}

impl Default for BrowserTool {
    fn default() -> Self {
        Self::new(BrowserConfig::default())
    }
}

#[async_trait]
impl Tool for BrowserTool {
    async fn execute(&self, input: ToolInput) -> Result<ToolOutput, ToolError> {
        if !self.config.read().unwrap().enabled {
            return Err(ToolError::PermissionDenied {
                name: "browser".into(),
                reason: "browser tool is disabled in configuration".into(),
            });
        }

        let action_str =
            input.arguments["action"]
                .as_str()
                .ok_or_else(|| ToolError::ValidationError {
                    message: format!(
                        "Missing 'action' parameter. Valid actions: {}",
                        BrowserAction::all_names()
                    ),
                })?;

        let action =
            BrowserAction::from_str(action_str).ok_or_else(|| ToolError::ValidationError {
                message: format!(
                    "Unknown browser action: '{}'. Valid actions: {}",
                    action_str,
                    BrowserAction::all_names()
                ),
            })?;

        // Close doesn't need connection.
        if matches!(action, BrowserAction::Close) {
            self.shutdown().await;
            return Ok(ToolOutput {
                success: true,
                content: serde_json::json!({"action": "close", "status": "disconnected"}),
                warnings: vec![],
                metadata: serde_json::json!({}),
            });
        }

        self.ensure_connected().await?;

        let result = self.dispatch_action(action, &input).await?;

        // Attach any console errors/warnings that occurred during the action.
        let console_warnings = self.collect_console_warnings().await;

        Ok(ToolOutput {
            success: true,
            content: result,
            warnings: console_warnings,
            metadata: serde_json::json!({}),
        })
    }

    fn definition(&self) -> &ToolDefinition {
        &self.def
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Extract a required string parameter.
fn require_str<'a>(args: &'a serde_json::Value, key: &str) -> Result<&'a str, ToolError> {
    args[key]
        .as_str()
        .ok_or_else(|| ToolError::ValidationError {
            message: format!("missing '{key}' parameter"),
        })
}

/// Convert CDP errors to tool errors.
fn cdp_to_tool_error(e: crate::cdp_client::CdpError) -> ToolError {
    use crate::cdp_client::CdpError;
    match e {
        CdpError::NotConnected => ToolError::ExternalServiceError {
            name: "browser".into(),
            message: "CDP connection lost. Try the action again.".into(),
        },
        CdpError::Timeout(ms) => ToolError::Timeout {
            timeout_secs: ms / 1000,
        },
        other => ToolError::ExternalServiceError {
            name: "browser".into(),
            message: other.to_string(),
        },
    }
}
