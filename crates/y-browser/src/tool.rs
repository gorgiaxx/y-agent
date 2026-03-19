//! Browser tool implementing `y-core::tool::Tool`.
//!
//! Exposes browser automation as a single unified tool for the agent.

use std::sync::Arc;
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
use crate::snapshot::SnapshotFormat;

/// Supported browser actions (parsed from tool input).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserAction {
    Navigate,
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
    Close,
}

impl BrowserAction {
    fn from_str(s: &str) -> Option<Self> {
        match s {
            "navigate" | "open" => Some(Self::Navigate),
            "screenshot" => Some(Self::Screenshot),
            "snapshot" => Some(Self::Snapshot),
            "click" => Some(Self::Click),
            "type" | "type_text" => Some(Self::Type),
            "get_text" => Some(Self::GetText),
            "get_title" => Some(Self::GetTitle),
            "get_url" => Some(Self::GetUrl),
            "evaluate" => Some(Self::Evaluate),
            "wait" => Some(Self::Wait),
            "press_key" | "press" => Some(Self::PressKey),
            "scroll" => Some(Self::Scroll),
            "get_page_text" => Some(Self::GetPageText),
            "close" => Some(Self::Close),
            _ => None,
        }
    }
}

/// Browser tool for agent integration.
pub struct BrowserTool {
    def: ToolDefinition,
    config: BrowserConfig,
    client: Arc<CdpClient>,
    actions: BrowserActions,
    security: SecurityPolicy,
    /// Locally launched Chrome process (if `auto_launch` is enabled).
    launcher: Mutex<Option<ChromeLauncher>>,
}

impl BrowserTool {
    /// Create a new browser tool with the given configuration.
    pub fn new(config: BrowserConfig) -> Self {
        // When auto_launch is true, the CDP URL is determined at launch time.
        // Use the local port for the client; the launcher will spawn Chrome there.
        let cdp_url = if config.auto_launch {
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
            config,
            client,
            actions,
            security,
            launcher: Mutex::new(None),
        }
    }

    /// Get the tool definition.
    pub fn tool_definition() -> ToolDefinition {
        ToolDefinition {
            name: ToolName::from_string("browser"),
            description: concat!(
                "Control a browser via Chrome DevTools Protocol. ",
                "Actions: navigate (open URL), screenshot (capture page), ",
                "snapshot (accessibility tree with refs like @e1), ",
                "click (CSS selector), type (fill input), get_text, get_title, get_url, ",
                "evaluate (run JS), wait (selector or ms), press_key, scroll, get_page_text, close. ",
                "Use 'snapshot' first to get element refs, then 'click'/'type' with those refs. ",
                "Requires Chrome running with --remote-debugging-port or auto_launch enabled."
            ).into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": [
                            "navigate", "screenshot", "snapshot",
                            "click", "type", "get_text", "get_title", "get_url",
                            "evaluate", "wait", "press_key", "scroll",
                            "get_page_text", "close"
                        ],
                        "description": "Browser action to perform"
                    },
                    "url": {
                        "type": "string",
                        "description": "URL to navigate to (for 'navigate')"
                    },
                    "selector": {
                        "type": "string",
                        "description": "CSS selector for click/type/get_text"
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
                        "description": "Snapshot format: 'aria' (accessibility) or 'dom' (HTML tree)"
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

        // Auto-launch Chrome if configured.
        if self.config.auto_launch {
            let mut launcher_guard = self.launcher.lock().await;
            if launcher_guard.is_none() {
                debug!("auto-launching Chrome");
                let chrome = ChromeLauncher::launch(
                    &self.config.chrome_path,
                    self.config.local_cdp_port,
                ).await.map_err(|e| ToolError::ExternalServiceError {
                    name: "browser".into(),
                    message: format!("Failed to launch Chrome: {e}"),
                })?;
                *launcher_guard = Some(chrome);
            }
        }

        let cdp_url = if self.config.auto_launch {
            format!("http://127.0.0.1:{}", self.config.local_cdp_port)
        } else {
            self.config.cdp_url.clone()
        };

        debug!(cdp_url = %cdp_url, "connecting to CDP");
        self.client.connect().await.map_err(|e| {
            ToolError::ExternalServiceError {
                name: "browser".into(),
                message: format!(
                    "Failed to connect to Chrome CDP at '{}': {}. {}",
                    cdp_url, e,
                    if self.config.auto_launch {
                        "Chrome was auto-launched but CDP connection failed."
                    } else {
                        "Make sure Chrome is running with --remote-debugging-port=9222"
                    }
                ),
            }
        })
    }

    /// Shutdown the launcher and disconnect.
    async fn shutdown(&self) {
        self.client.disconnect().await;
        let mut launcher_guard = self.launcher.lock().await;
        if let Some(mut chrome) = launcher_guard.take() {
            chrome.shutdown().await;
        }
    }
}

impl Default for BrowserTool {
    fn default() -> Self {
        Self::new(BrowserConfig::default())
    }
}

#[async_trait]
impl Tool for BrowserTool {
    async fn execute(&self, input: ToolInput) -> Result<ToolOutput, ToolError> {
        if !self.config.enabled {
            return Err(ToolError::PermissionDenied {
                name: "browser".into(),
                reason: "browser tool is disabled in configuration".into(),
            });
        }

        let action_str = input.arguments["action"]
            .as_str()
            .ok_or_else(|| ToolError::ValidationError {
                message: "missing 'action' parameter".into(),
            })?;

        let action = BrowserAction::from_str(action_str).ok_or_else(|| {
            ToolError::ValidationError {
                message: format!("unknown browser action: '{action_str}'"),
            }
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

        let result = match action {
            BrowserAction::Navigate => {
                let url = input.arguments["url"]
                    .as_str()
                    .ok_or_else(|| ToolError::ValidationError {
                        message: "missing 'url' parameter for navigate".into(),
                    })?;

                // Security check.
                self.security.validate_url(url).map_err(|e| {
                    ToolError::PermissionDenied {
                        name: "browser".into(),
                        reason: e.to_string(),
                    }
                })?;

                let nav = self.actions.navigate(url).await.map_err(cdp_to_tool_error)?;
                serde_json::to_value(nav).unwrap_or_default()
            }

            BrowserAction::Screenshot => {
                let full_page = input.arguments["full_page"].as_bool().unwrap_or(false);
                let format = input.arguments["format"]
                    .as_str()
                    .unwrap_or("png");
                let quality = input.arguments["quality"]
                    .as_u64()
                    .map(|q| q as u32);

                let shot = self
                    .actions
                    .screenshot(full_page, format, quality)
                    .await
                    .map_err(cdp_to_tool_error)?;
                serde_json::to_value(shot).unwrap_or_default()
            }

            BrowserAction::Snapshot => {
                let format = match input.arguments["format"].as_str() {
                    Some("dom") => SnapshotFormat::Dom,
                    _ => SnapshotFormat::Aria,
                };
                let limit = input.arguments["limit"]
                    .as_u64()
                    .unwrap_or(500) as usize;

                let snap = match format {
                    SnapshotFormat::Aria => {
                        self.actions.snapshot_aria(limit).await.map_err(cdp_to_tool_error)?
                    }
                    SnapshotFormat::Dom => {
                        let max_text = input.arguments["max_text_chars"]
                            .as_u64()
                            .unwrap_or(220) as usize;
                        self.actions
                            .snapshot_dom(limit, max_text)
                            .await
                            .map_err(cdp_to_tool_error)?
                    }
                };
                serde_json::to_value(snap).unwrap_or_default()
            }

            BrowserAction::Click => {
                let selector = require_str(&input.arguments, "selector")?;
                self.actions.click(selector).await.map_err(cdp_to_tool_error)?;
                serde_json::json!({"action": "click", "selector": selector, "ok": true})
            }

            BrowserAction::Type => {
                let selector = require_str(&input.arguments, "selector")?;
                let text = require_str(&input.arguments, "text")?;
                self.actions
                    .type_text(selector, text)
                    .await
                    .map_err(cdp_to_tool_error)?;
                serde_json::json!({"action": "type", "selector": selector, "ok": true})
            }

            BrowserAction::GetText => {
                let selector = require_str(&input.arguments, "selector")?;
                let text = self.actions.get_text(selector).await.map_err(cdp_to_tool_error)?;
                serde_json::json!({"text": text, "selector": selector})
            }

            BrowserAction::GetTitle => {
                let title = self.actions.get_title().await.map_err(cdp_to_tool_error)?;
                serde_json::json!({"title": title})
            }

            BrowserAction::GetUrl => {
                let url = self.actions.get_url().await.map_err(cdp_to_tool_error)?;
                serde_json::json!({"url": url})
            }

            BrowserAction::Evaluate => {
                let expression = require_str(&input.arguments, "expression")?;
                let eval = self
                    .actions
                    .evaluate(expression)
                    .await
                    .map_err(cdp_to_tool_error)?;
                serde_json::to_value(eval).unwrap_or_default()
            }

            BrowserAction::Wait => {
                let selector = input.arguments["selector"].as_str();
                let ms = input.arguments["ms"].as_u64();
                self.actions
                    .wait(selector, ms)
                    .await
                    .map_err(cdp_to_tool_error)?;
                serde_json::json!({"action": "wait", "ok": true})
            }

            BrowserAction::PressKey => {
                let key = require_str(&input.arguments, "key")?;
                self.actions.press_key(key).await.map_err(cdp_to_tool_error)?;
                serde_json::json!({"action": "press_key", "key": key, "ok": true})
            }

            BrowserAction::Scroll => {
                let direction = input.arguments["direction"]
                    .as_str()
                    .unwrap_or("down");
                let pixels = input.arguments["pixels"]
                    .as_u64()
                    .unwrap_or(300) as u32;
                self.actions
                    .scroll(direction, pixels)
                    .await
                    .map_err(cdp_to_tool_error)?;
                serde_json::json!({"action": "scroll", "direction": direction, "pixels": pixels, "ok": true})
            }

            BrowserAction::GetPageText => {
                let text = self
                    .actions
                    .get_page_text()
                    .await
                    .map_err(cdp_to_tool_error)?;
                serde_json::json!({"text": text})
            }

            BrowserAction::Close => unreachable!(), // handled above
        };

        Ok(ToolOutput {
            success: true,
            content: result,
            warnings: vec![],
            metadata: serde_json::json!({}),
        })
    }

    fn definition(&self) -> &ToolDefinition {
        &self.def
    }
}

/// Extract a required string parameter.
fn require_str<'a>(args: &'a serde_json::Value, key: &str) -> Result<&'a str, ToolError> {
    args[key].as_str().ok_or_else(|| ToolError::ValidationError {
        message: format!("missing '{key}' parameter"),
    })
}

/// Convert CDP errors to tool errors.
fn cdp_to_tool_error(e: crate::cdp_client::CdpError) -> ToolError {
    use crate::cdp_client::CdpError;
    match e {
        CdpError::NotConnected => ToolError::ExternalServiceError {
            name: "browser".into(),
            message: "CDP connection lost".into(),
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
