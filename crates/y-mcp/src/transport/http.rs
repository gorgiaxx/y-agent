//! HTTP Streamable transport: communicates via HTTP POST requests.

use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use tracing::debug;

use super::{JsonRpcNotification, JsonRpcRequest, JsonRpcResponse, McpTransport};
use crate::error::McpError;

const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// MCP transport over HTTP (Streamable HTTP).
///
/// Sends JSON-RPC requests as HTTP POST and handles both JSON and SSE responses.
/// Supports bearer token authentication and custom HTTP headers.
pub struct HttpTransport {
    url: String,
    client: reqwest::Client,
    /// Optional bearer token for `Authorization` header.
    bearer_token: Option<String>,
    /// Custom HTTP headers sent with every request.
    custom_headers: HashMap<String, String>,
    /// Server name for error context.
    server_name: String,
}

/// Builder for configuring an [`HttpTransport`].
#[must_use]
pub struct HttpTransportBuilder {
    url: String,
    timeout: Duration,
    bearer_token: Option<String>,
    custom_headers: HashMap<String, String>,
    server_name: String,
}

impl HttpTransportBuilder {
    /// Set the request timeout.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Set a bearer token for `Authorization: Bearer <token>`.
    pub fn bearer_token(mut self, token: impl Into<String>) -> Self {
        self.bearer_token = Some(token.into());
        self
    }

    /// Set an optional bearer token.
    pub fn bearer_token_opt(mut self, token: Option<String>) -> Self {
        self.bearer_token = token;
        self
    }

    /// Add custom HTTP headers sent with every request.
    pub fn headers(mut self, headers: HashMap<String, String>) -> Self {
        self.custom_headers = headers;
        self
    }

    /// Set the server name (used in error messages).
    pub fn server_name(mut self, name: impl Into<String>) -> Self {
        self.server_name = name.into();
        self
    }

    /// Build the HTTP transport.
    pub fn build(self) -> Result<HttpTransport, McpError> {
        let client = reqwest::Client::builder()
            .timeout(self.timeout)
            .build()
            .map_err(|e| McpError::ConnectionFailed {
                message: format!("failed to build HTTP client: {e}"),
            })?;
        Ok(HttpTransport {
            url: self.url,
            client,
            bearer_token: self.bearer_token,
            custom_headers: self.custom_headers,
            server_name: self.server_name,
        })
    }
}

impl HttpTransport {
    /// Create a new HTTP transport pointing at the given URL.
    pub fn new(url: &str) -> Result<Self, McpError> {
        Self::builder(url).build()
    }

    /// Create a builder for configuring an HTTP transport.
    pub fn builder(url: &str) -> HttpTransportBuilder {
        HttpTransportBuilder {
            url: url.to_string(),
            timeout: DEFAULT_REQUEST_TIMEOUT,
            bearer_token: None,
            custom_headers: HashMap::new(),
            server_name: String::new(),
        }
    }

    /// POST a JSON body and return the raw response.
    async fn post_json(&self, body: &[u8]) -> Result<reqwest::Response, McpError> {
        let mut request = self
            .client
            .post(&self.url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream");

        // Attach bearer token if available.
        if let Some(ref token) = self.bearer_token {
            request = request.header("Authorization", format!("Bearer {token}"));
        }

        // Attach custom headers.
        for (key, value) in &self.custom_headers {
            request = request.header(key.as_str(), value.as_str());
        }

        let resp = request.body(body.to_vec()).send().await.map_err(|e| {
            if e.is_timeout() {
                McpError::Timeout {
                    message: format!("HTTP request timed out: {e}"),
                }
            } else {
                McpError::TransportError {
                    message: format!("HTTP request failed: {e}"),
                }
            }
        })?;

        let status = resp.status();

        // Handle 401 Unauthorized as a distinct error.
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(McpError::AuthenticationRequired {
                server: self.server_name.clone(),
            });
        }

        // Handle 404 Not Found as session expiration (server dropped our session).
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(McpError::SessionExpired {
                server: self.server_name.clone(),
            });
        }

        if !status.is_success() {
            let body_text = resp.text().await.unwrap_or_default();
            return Err(McpError::TransportError {
                message: format!("HTTP {status}: {body_text}"),
            });
        }

        Ok(resp)
    }
}

#[async_trait]
impl McpTransport for HttpTransport {
    async fn send(&self, request: JsonRpcRequest) -> Result<JsonRpcResponse, McpError> {
        let body = serde_json::to_vec(&request).map_err(McpError::SerializationError)?;
        let resp = self.post_json(&body).await?;

        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        let text = resp.text().await.map_err(|e| McpError::TransportError {
            message: format!("failed to read response body: {e}"),
        })?;

        let resp_parsed: JsonRpcResponse = if content_type.contains("text/event-stream") {
            debug!("parsing SSE response");
            parse_sse_response(&text)?
        } else {
            serde_json::from_str(&text).map_err(|e| McpError::ProtocolError {
                message: format!("failed to parse JSON-RPC response: {e}"),
            })?
        };

        // JSON-RPC error code -32001 is the conventional MCP session-expired code.
        if let Some(ref err) = resp_parsed.error {
            if err.code == -32001 {
                return Err(McpError::SessionExpired {
                    server: self.server_name.clone(),
                });
            }
        }

        Ok(resp_parsed)
    }

    async fn send_notification(&self, notification: JsonRpcNotification) -> Result<(), McpError> {
        let body = serde_json::to_vec(&notification).map_err(McpError::SerializationError)?;
        self.post_json(&body).await?;
        Ok(())
    }

    async fn close(&self) -> Result<(), McpError> {
        // HTTP is stateless; nothing to close.
        Ok(())
    }

    fn transport_type(&self) -> &'static str {
        "http"
    }
}

/// Extract a JSON-RPC response from an SSE text body.
///
/// SSE events are separated by double newlines. Each event may have multiple
/// lines prefixed with `data:`. We concatenate the data lines and try to parse
/// each event as a `JsonRpcResponse`, returning the first successful parse.
fn parse_sse_response(text: &str) -> Result<JsonRpcResponse, McpError> {
    for event_block in text.split("\n\n") {
        let mut data = String::new();
        for line in event_block.lines() {
            if let Some(payload) = line.strip_prefix("data:") {
                if !data.is_empty() {
                    data.push('\n');
                }
                data.push_str(payload.trim_start());
            }
        }
        if data.is_empty() {
            continue;
        }
        if let Ok(resp) = serde_json::from_str::<JsonRpcResponse>(&data) {
            return Ok(resp);
        }
    }
    Err(McpError::ProtocolError {
        message: "no valid JSON-RPC response found in SSE stream".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_sse_response_basic() {
        let sse = "data: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"tools\":[]}}\n\n";
        let resp = parse_sse_response(sse).unwrap();
        assert_eq!(resp.id, 1);
        assert!(resp.result.is_some());
    }

    #[test]
    fn test_parse_sse_response_multiline_data() {
        // Some servers split data across multiple `data:` lines.
        let sse = "data: {\"jsonrpc\":\"2.0\",\ndata: \"id\":2,\"result\":null}\n\n";
        let resp = parse_sse_response(sse).unwrap();
        assert_eq!(resp.id, 2);
    }

    #[test]
    fn test_parse_sse_response_with_event_type() {
        let sse = "event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":3,\"result\":{}}\n\n";
        let resp = parse_sse_response(sse).unwrap();
        assert_eq!(resp.id, 3);
    }

    #[test]
    fn test_parse_sse_response_no_valid_data() {
        let sse = "event: ping\n\n";
        let result = parse_sse_response(sse);
        assert!(matches!(result, Err(McpError::ProtocolError { .. })));
    }

    #[test]
    fn test_http_transport_new() {
        let transport = HttpTransport::new("http://localhost:3000/mcp").unwrap();
        assert_eq!(transport.transport_type(), "http");
    }
}
