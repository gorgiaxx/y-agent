//! `WebFetch` built-in tool: fetch web pages or search the web.
//!
//! Wraps the Browser tool's CDP-based navigation to provide a simplified
//! interface for the LLM.  Two actions are supported:
//!
//! - **fetch** (default): navigate to a URL and return the page text.
//! - **search**: query a search engine and return the results page text.
//!
//! The tool shares the Chrome session with the `Browser` tool via
//! `Arc<BrowserTool>`, so no extra Browser process is spawned.

use std::sync::Arc;

use async_trait::async_trait;

use y_browser::BrowserTool;
use y_core::runtime::RuntimeCapability;
use y_core::tool::{
    Tool, ToolCategory, ToolDefinition, ToolError, ToolInput, ToolOutput, ToolType,
};
use y_core::types::ToolName;

/// Built-in tool for fetching web page content or searching the web.
pub struct WebFetchTool {
    def: ToolDefinition,
    browser: Arc<BrowserTool>,
}

impl WebFetchTool {
    /// Create a new web fetch tool backed by a shared Browser instance.
    pub fn new(browser: Arc<BrowserTool>) -> Self {
        Self {
            def: Self::tool_definition(),
            browser,
        }
    }

    pub fn tool_definition() -> ToolDefinition {
        ToolDefinition {
            name: ToolName::from_string("WebFetch"),
            description: "Fetch web page content or search the web via a real Browser engine."
                .into(),
            help: Some(
                concat!(
                    "Actions:\n",
                    "- fetch (default): Navigate to a URL and return accessibility-tree text. ",
                    "Args: url (required), wait_ms (optional, default 1000)\n",
                    "- search: Search the web via a search engine. ",
                    "Args: query (required), search_engine ('google'|'bing'|'duckduckgo'|'baidu', ",
                    "default 'google'), wait_ms (optional, default 2000). Returns AX snapshot text.\n\n",
                    "Aliases: web_search, search, read_url, fetch_url, scrape",
                )
                .into(),
            ),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["fetch", "search"],
                        "description": "Action to perform: 'fetch' (default) or 'search'"
                    },
                    "url": {
                        "type": "string",
                        "description": "URL to fetch (for 'fetch' action)"
                    },
                    "query": {
                        "type": "string",
                        "description": "Search query (for 'search' action)"
                    },
                    "search_engine": {
                        "type": "string",
                        "enum": ["google", "bing", "duckduckgo", "baidu"],
                        "description": "Search engine to use (default: 'google')"
                    },
                    "wait_ms": {
                        "type": "integer",
                        "description": "Milliseconds to wait for page rendering (default: 1000 for fetch, 2000 for search; max 10000)"
                    }
                }
            }),
            result_schema: None,
            category: ToolCategory::Network,
            tool_type: ToolType::BuiltIn,
            capabilities: RuntimeCapability::default(),
            is_dangerous: true,
        }
    }

    /// Resolve the action from the tool input.
    ///
    /// The tool name itself may carry intent: if the LLM calls it as
    /// "`web_search`" or "search", we default to the search action.
    fn resolve_action(input: &ToolInput) -> WebFetchAction {
        // Explicit action parameter takes priority.
        if let Some(action_str) = input.arguments.get("action").and_then(|v| v.as_str()) {
            return match action_str {
                "search" | "WebSearch" => WebFetchAction::Search,
                _ => WebFetchAction::Fetch,
            };
        }

        // Infer from which required parameter is present.
        if input
            .arguments
            .get("query")
            .and_then(|v| v.as_str())
            .is_some()
        {
            return WebFetchAction::Search;
        }

        WebFetchAction::Fetch
    }
}

/// Internal action discriminant.
enum WebFetchAction {
    Fetch,
    Search,
}

#[async_trait]
impl Tool for WebFetchTool {
    async fn execute(&self, input: ToolInput) -> Result<ToolOutput, ToolError> {
        let action = Self::resolve_action(&input);

        match action {
            WebFetchAction::Fetch => {
                let url =
                    input.arguments["url"]
                        .as_str()
                        .ok_or_else(|| ToolError::ValidationError {
                            message: "Missing 'url' parameter for fetch. Example: \
                                {\"action\": \"fetch\", \"url\": \"https://example.com\"}"
                                .into(),
                        })?;
                let wait_ms = input.arguments["wait_ms"].as_u64().or(Some(1000));

                let text = self.browser.fetch_page_text(url, wait_ms).await?;
                // Best-effort page metadata for GUI rendering.
                let (title, favicon) = self.browser.fetch_page_meta().await;

                Ok(ToolOutput {
                    success: true,
                    content: serde_json::json!({
                        "action": "fetch",
                        "url": url,
                        "title": title,
                        "favicon_url": favicon,
                        "text": text,
                    }),
                    warnings: vec![],
                    metadata: serde_json::json!({}),
                })
            }
            WebFetchAction::Search => {
                let query = input.arguments["query"].as_str().ok_or_else(|| {
                    ToolError::ValidationError {
                        message: "Missing 'query' parameter for search. Example: \
                            {\"action\": \"search\", \"query\": \"rust async tutorial\"}"
                            .into(),
                    }
                })?;
                let search_engine = input.arguments["search_engine"].as_str();
                let engine_name = search_engine.unwrap_or("google");
                let wait_ms = input.arguments["wait_ms"].as_u64().or(Some(2000));

                let text = self
                    .browser
                    .search_page_text(query, search_engine, wait_ms)
                    .await?;
                // Best-effort page metadata for GUI rendering.
                let (title, favicon) = self.browser.fetch_page_meta().await;

                // Build the actual search URL so the GUI can render a clickable
                // URL tag (favicon + open-in-browser button). Best-effort for
                // display only -- the real navigation is done inside BrowserTool.
                let search_url = y_browser::build_search_url(engine_name, query)
                    .ok()
                    .unwrap_or_default();

                Ok(ToolOutput {
                    success: true,
                    content: y_browser::build_search_result(
                        query,
                        engine_name,
                        &search_url,
                        &title,
                        &favicon,
                        &text,
                    ),
                    warnings: vec![],
                    metadata: serde_json::json!({}),
                })
            }
        }
    }

    fn definition(&self) -> &ToolDefinition {
        &self.def
    }

    fn is_read_only(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use y_core::types::SessionId;

    fn make_tool() -> WebFetchTool {
        let browser = Arc::new(BrowserTool::new(y_browser::BrowserConfig::default()));
        WebFetchTool::new(browser)
    }

    fn make_input(args: serde_json::Value) -> ToolInput {
        ToolInput {
            call_id: "call_001".into(),
            name: ToolName::from_string("WebFetch"),
            arguments: args,
            session_id: SessionId::new(),
            working_dir: None,
            command_runner: None,
        }
    }

    #[test]
    fn test_definition() {
        let def = WebFetchTool::tool_definition();
        assert_eq!(def.name.as_str(), "WebFetch");
        assert_eq!(def.category, ToolCategory::Network);
        assert!(def.is_dangerous);
    }

    #[test]
    fn test_resolve_action_defaults_to_fetch() {
        let input = make_input(serde_json::json!({"url": "https://example.com"}));
        assert!(matches!(
            WebFetchTool::resolve_action(&input),
            WebFetchAction::Fetch
        ));
    }

    #[test]
    fn test_resolve_action_explicit_search() {
        let input = make_input(serde_json::json!({"action": "search", "query": "test"}));
        assert!(matches!(
            WebFetchTool::resolve_action(&input),
            WebFetchAction::Search
        ));
    }

    #[test]
    fn test_resolve_action_infers_search_from_query() {
        let input = make_input(serde_json::json!({"query": "rust async"}));
        assert!(matches!(
            WebFetchTool::resolve_action(&input),
            WebFetchAction::Search
        ));
    }

    #[tokio::test]
    async fn test_fetch_missing_url_returns_error() {
        let tool = make_tool();
        let input = make_input(serde_json::json!({"action": "fetch"}));
        let result = tool.execute(input).await;
        assert!(result.is_err());
        if let Err(ToolError::ValidationError { message }) = result {
            assert!(message.contains("url"));
        }
    }

    #[tokio::test]
    async fn test_search_missing_query_returns_error() {
        let tool = make_tool();
        let input = make_input(serde_json::json!({"action": "search"}));
        let result = tool.execute(input).await;
        assert!(result.is_err());
        if let Err(ToolError::ValidationError { message }) = result {
            assert!(message.contains("query"));
        }
    }
}
