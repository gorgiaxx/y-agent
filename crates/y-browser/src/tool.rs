//! Browser tool implementing `y-core::tool::Tool`.
//!
//! Exposes browser automation as a single unified tool for the agent.
//! Key workflow: snapshot -> get refs -> click/type with refs.
//!
//! This module is a thin dispatch layer; all connection lifecycle is
//! managed by [`crate::session::BrowserSession`].

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tracing::warn;

use y_core::runtime::RuntimeCapability;
use y_core::tool::{
    Tool, ToolCategory, ToolDefinition, ToolError, ToolInput, ToolOutput, ToolType,
};
use y_core::types::ToolName;

use crate::config::BrowserConfig;
use crate::session::BrowserSession;
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
            | "get_html" | "html" | "accessibility" | "a11y" => Some(Self::Snapshot),
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
        "navigate, search, screenshot, snapshot, click, type, get_text, get_title, \
         get_url, evaluate, wait, press_key, scroll, get_page_text, \
         get_console_logs, close"
    }
}

/// Browser tool for agent integration.
///
/// Thin dispatch wrapper around [`BrowserSession`].
pub struct BrowserTool {
    def: ToolDefinition,
    session: Arc<BrowserSession>,
}

impl BrowserTool {
    /// Create a new browser tool with the given configuration.
    pub fn new(config: BrowserConfig) -> Self {
        Self {
            def: Self::tool_definition(),
            session: Arc::new(BrowserSession::new(config)),
        }
    }

    /// Create a browser tool from an existing session (for sharing).
    pub fn from_session(session: Arc<BrowserSession>) -> Self {
        Self {
            def: Self::tool_definition(),
            session,
        }
    }

    /// Get a reference to the underlying session.
    pub fn session(&self) -> &Arc<BrowserSession> {
        &self.session
    }

    /// Hot-reload the browser configuration.
    pub fn reload_config(&self, new_config: BrowserConfig) {
        self.session.reload_config(new_config);
    }

    /// Get the tool definition.
    pub fn tool_definition() -> ToolDefinition {
        ToolDefinition {
            name: ToolName::from_string("browser"),
            description: "Control a web browser for navigation, interaction, and page inspection."
                .into(),
            help: Some(
                concat!(
                    "WORKFLOW: (1) navigate/search, (2) snapshot to get @refs, ",
                    "(3) click/type using @refs.\n\n",
                    "Actions:\n",
                    "- navigate: Open a URL. Args: url (required)\n",
                    "- search: Search via search engine. Args: query (required), ",
                    "search_engine ('google'|'bing'|'duckduckgo'|'baidu', default 'google')\n",
                    "- snapshot: Get page elements with @eN refs. Use these refs for click/type. ",
                    "Args: format ('aria'|'dom'), interactive_only (bool, default true)\n",
                    "- click: Click an element. Args: selector ('@e1' ref from snapshot, ",
                    "or CSS selector)\n",
                    "- type: Type text into an input. Args: selector ('@e1' ref or CSS), ",
                    "text (required)\n",
                    "- screenshot: Capture page image. Args: full_page (bool)\n",
                    "- get_text: Get element text. Args: selector ('@e1' ref or CSS)\n",
                    "- get_title: Get page title\n",
                    "- get_url: Get current URL\n",
                    "- evaluate: Run JavaScript. Args: expression (required)\n",
                    "- wait: Wait for condition. Args: selector (CSS to wait for) ",
                    "or ms (milliseconds)\n",
                    "- press_key: Press keyboard key. Args: key ('Enter', 'Tab', 'Escape', etc.)\n",
                    "- scroll: Scroll page. Args: direction ('up'|'down'|'left'|'right'), ",
                    "pixels (default 300)\n",
                    "- get_page_text: Get all visible page text\n",
                    "- get_console_logs: Get browser console output (errors, warnings, logs)\n",
                    "- close: Close browser\n\n",
                    "IMPORTANT: After 'snapshot', use the @eN refs (e.g. @e3) for click/type ",
                    "-- do NOT guess CSS selectors.",
                )
                .into(),
            ),
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
                        "description": "Element ref from snapshot (e.g. '@e1') or CSS selector"
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
                        "description": "Snapshot format: 'aria' or 'dom'"
                    },
                    "interactive_only": {
                        "type": "boolean",
                        "description": "Snapshot only interactive elements (default: true)"
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
            is_dangerous: true,
        }
    }

    /// Fetch the text content of a web page.
    ///
    /// Public API used by `WebFetchTool` to wrap the browser's
    /// navigate + `get_page_text` workflow into a single call.
    pub async fn fetch_page_text(
        &self,
        url: &str,
        wait_ms: Option<u64>,
    ) -> Result<String, ToolError> {
        if !self.session.config().enabled {
            return Err(ToolError::PermissionDenied {
                name: "web_fetch".into(),
                reason: "browser tool is disabled in configuration".into(),
            });
        }

        self.session
            .security()
            .validate_url(url)
            .map_err(|e| ToolError::PermissionDenied {
                name: "web_fetch".into(),
                reason: e.to_string(),
            })?;

        self.session.ensure_connected().await?;

        self.session
            .actions()
            .navigate(url)
            .await
            .map_err(cdp_to_tool_error)?;

        if let Some(ms) = wait_ms {
            let ms = ms.min(10_000);
            if ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
            }
        }

        let text = self
            .session
            .actions()
            .get_page_text()
            .await
            .map_err(cdp_to_tool_error)?;

        Ok(truncate_output(&text, MAX_OUTPUT_CHARS))
    }

    /// Search via a search engine and return the results page text.
    pub async fn search_page_text(
        &self,
        query: &str,
        search_engine: Option<&str>,
        wait_ms: Option<u64>,
    ) -> Result<String, ToolError> {
        if !self.session.config().enabled {
            return Err(ToolError::PermissionDenied {
                name: "web_fetch".into(),
                reason: "browser tool is disabled in configuration".into(),
            });
        }

        let engine = search_engine.map_or_else(
            || self.session.config().default_search_engine.clone(),
            String::from,
        );

        let search_url = build_search_url(&engine, query)?;

        self.session
            .security()
            .validate_url(&search_url)
            .map_err(|e| ToolError::PermissionDenied {
                name: "web_fetch".into(),
                reason: e.to_string(),
            })?;

        self.session.ensure_connected().await?;

        self.session
            .actions()
            .navigate(&search_url)
            .await
            .map_err(cdp_to_tool_error)?;

        if let Some(ms) = wait_ms {
            let ms = ms.min(10_000);
            if ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
            }
        }

        let text = self
            .session
            .actions()
            .get_page_text()
            .await
            .map_err(cdp_to_tool_error)?;

        Ok(truncate_output(&text, MAX_OUTPUT_CHARS))
    }

    /// Handle the `search` action.
    async fn dispatch_search(&self, input: &ToolInput) -> Result<serde_json::Value, ToolError> {
        let query =
            require_str(&input.arguments, "query").map_err(|_| ToolError::ValidationError {
                message: "Missing 'query' parameter for search. Example: \
                    {\"action\": \"search\", \"query\": \"rust async tutorial\"}"
                    .into(),
            })?;

        let engine = input.arguments["search_engine"].as_str().map_or_else(
            || self.session.config().default_search_engine.clone(),
            String::from,
        );

        let search_url = build_search_url(&engine, query)?;

        self.session
            .security()
            .validate_url(&search_url)
            .map_err(|e| ToolError::PermissionDenied {
                name: "browser".into(),
                reason: e.to_string(),
            })?;

        let nav = self
            .session
            .actions()
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
        let actions = self.session.actions();

        match action {
            BrowserAction::Navigate => {
                let url =
                    input.arguments["url"]
                        .as_str()
                        .ok_or_else(|| ToolError::ValidationError {
                            message: "Missing 'url' parameter for navigate. Example: \
                            {\"action\": \"navigate\", \"url\": \"https://example.com\"}"
                                .into(),
                        })?;

                self.session.security().validate_url(url).map_err(|e| {
                    ToolError::PermissionDenied {
                        name: "browser".into(),
                        reason: e.to_string(),
                    }
                })?;

                let nav = actions.navigate(url).await.map_err(cdp_to_tool_error)?;
                Ok(serde_json::to_value(nav).unwrap_or_default())
            }

            BrowserAction::Search => self.dispatch_search(input).await,

            BrowserAction::Screenshot => {
                let full_page = input.arguments["full_page"].as_bool().unwrap_or(false);
                let format = input.arguments["format"].as_str().unwrap_or("png");
                let quality = input.arguments["quality"]
                    .as_u64()
                    .map(|q| u32::try_from(q).unwrap_or(100));

                let shot = actions
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
                    SnapshotFormat::Aria => actions
                        .snapshot_aria(limit, interactive_only)
                        .await
                        .map_err(cdp_to_tool_error)?,
                    SnapshotFormat::Dom => {
                        let max_text = usize::try_from(
                            input.arguments["max_text_chars"].as_u64().unwrap_or(220),
                        )
                        .unwrap_or(220);
                        actions
                            .snapshot_dom(limit, max_text)
                            .await
                            .map_err(cdp_to_tool_error)?
                    }
                };

                snap.text = truncate_output(&snap.text, MAX_OUTPUT_CHARS);
                Ok(serde_json::to_value(snap).unwrap_or_default())
            }

            BrowserAction::Click => {
                let selector = require_str(&input.arguments, "selector").map_err(|_| {
                    ToolError::ValidationError {
                        message: "Missing 'selector' for click. Use an @ref from snapshot \
                        (e.g. '@e1') or a CSS selector (e.g. '#submit-btn')."
                            .into(),
                    }
                })?;
                actions.click(selector).await.map_err(cdp_to_tool_error)?;
                Ok(serde_json::json!({"action": "click", "selector": selector, "ok": true}))
            }

            BrowserAction::Type => {
                let selector = require_str(&input.arguments, "selector").map_err(|_| {
                    ToolError::ValidationError {
                        message: "Missing 'selector' for type. Use an @ref from snapshot \
                        (e.g. '@e3') or a CSS selector."
                            .into(),
                    }
                })?;
                let text = require_str(&input.arguments, "text").map_err(|_| {
                    ToolError::ValidationError {
                        message: "Missing 'text' for type. Example: \
                        {\"action\": \"type\", \"selector\": \"@e3\", \"text\": \"hello\"}"
                            .into(),
                    }
                })?;
                actions
                    .type_text(selector, text)
                    .await
                    .map_err(cdp_to_tool_error)?;
                Ok(serde_json::json!({"action": "type", "selector": selector, "ok": true}))
            }

            BrowserAction::GetText => {
                let selector = require_str(&input.arguments, "selector").map_err(|_| {
                    ToolError::ValidationError {
                        message: "Missing 'selector' for get_text. Use an @ref \
                        (e.g. '@e5') or CSS selector."
                            .into(),
                    }
                })?;
                let text = actions
                    .get_text(selector)
                    .await
                    .map_err(cdp_to_tool_error)?;
                let text = truncate_output(&text, MAX_OUTPUT_CHARS);
                Ok(serde_json::json!({"text": text, "selector": selector}))
            }

            BrowserAction::GetTitle => {
                let title = actions.get_title().await.map_err(cdp_to_tool_error)?;
                Ok(serde_json::json!({"title": title}))
            }

            BrowserAction::GetUrl => {
                let url = actions.get_url().await.map_err(cdp_to_tool_error)?;
                Ok(serde_json::json!({"url": url}))
            }

            BrowserAction::Evaluate => {
                let expression = require_str(&input.arguments, "expression").map_err(|_| {
                    ToolError::ValidationError {
                        message: "Missing 'expression' for evaluate. Example: \
                        {\"action\": \"evaluate\", \"expression\": \"document.title\"}"
                            .into(),
                    }
                })?;
                let eval = actions
                    .evaluate(expression)
                    .await
                    .map_err(cdp_to_tool_error)?;
                Ok(serde_json::to_value(eval).unwrap_or_default())
            }

            BrowserAction::Wait => {
                let selector = input.arguments["selector"].as_str();
                let ms = input.arguments["ms"].as_u64();
                actions
                    .wait(selector, ms)
                    .await
                    .map_err(cdp_to_tool_error)?;
                Ok(serde_json::json!({"action": "wait", "ok": true}))
            }

            BrowserAction::PressKey => {
                let key = require_str(&input.arguments, "key").map_err(|_| {
                    ToolError::ValidationError {
                        message: "Missing 'key' for press_key. Example: \
                        {\"action\": \"press_key\", \"key\": \"Enter\"}"
                            .into(),
                    }
                })?;
                actions.press_key(key).await.map_err(cdp_to_tool_error)?;
                Ok(serde_json::json!({"action": "press_key", "key": key, "ok": true}))
            }

            BrowserAction::Scroll => {
                let direction = input.arguments["direction"].as_str().unwrap_or("down");
                let pixels =
                    u32::try_from(input.arguments["pixels"].as_u64().unwrap_or(300)).unwrap_or(300);
                actions
                    .scroll(direction, pixels)
                    .await
                    .map_err(cdp_to_tool_error)?;
                Ok(serde_json::json!({
                    "action": "scroll",
                    "direction": direction,
                    "pixels": pixels,
                    "ok": true
                }))
            }

            BrowserAction::GetPageText => {
                let text = actions.get_page_text().await.map_err(cdp_to_tool_error)?;
                let text = truncate_output(&text, MAX_OUTPUT_CHARS);
                Ok(serde_json::json!({"text": text}))
            }

            BrowserAction::GetConsoleLogs => {
                let logs = actions.take_console_logs().await;
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
        self.session
            .actions()
            .peek_console_logs()
            .await
            .iter()
            .filter(|l| l.level == "error" || l.level == "warning")
            .take(5)
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
        if !self.session.config().enabled {
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
            self.session.shutdown().await;
            return Ok(ToolOutput {
                success: true,
                content: serde_json::json!({"action": "close", "status": "disconnected"}),
                warnings: vec![],
                metadata: serde_json::json!({}),
            });
        }

        self.session.ensure_connected().await?;

        // Dispatch the action, with one automatic retry on connection errors.
        let result = match self.dispatch_action(action.clone(), &input).await {
            Ok(v) => v,
            Err(ref e) if BrowserSession::is_connection_error(e) => {
                warn!("browser action failed with connection error, attempting reconnect");
                self.session.reset().await;
                self.session.ensure_connected().await?;
                self.dispatch_action(action, &input).await?
            }
            Err(e) => return Err(e),
        };

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
