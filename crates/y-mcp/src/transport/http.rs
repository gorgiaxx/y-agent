//! HTTP Streamable transport: communicates via HTTP POST requests.

use std::time::Duration;

use async_trait::async_trait;
use tracing::debug;

use super::{JsonRpcNotification, JsonRpcRequest, JsonRpcResponse, McpTransport};
use crate::error::McpError;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// MCP transport over HTTP (Streamable HTTP).
///
/// Sends JSON-RPC requests as HTTP POST and handles both JSON and SSE responses.
pub struct HttpTransport {
    url: String,
    client: reqwest::Client,
}

impl HttpTransport {
    /// Create a new HTTP transport pointing at the given URL.
    pub fn new(url: &str) -> Result<Self, McpError> {
        let client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|e| McpError::ConnectionFailed {
                message: format!("failed to build HTTP client: {e}"),
            })?;
        Ok(Self {
            url: url.to_string(),
            client,
        })
    }

    /// POST a JSON body and return the raw response.
    async fn post_json(&self, body: &[u8]) -> Result<reqwest::Response, McpError> {
        let resp = self
            .client
            .post(&self.url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .body(body.to_vec())
            .send()
            .await
            .map_err(|e| {
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

        if content_type.contains("text/event-stream") {
            debug!("parsing SSE response");
            parse_sse_response(&text)
        } else {
            serde_json::from_str(&text).map_err(|e| McpError::ProtocolError {
                message: format!("failed to parse JSON-RPC response: {e}"),
            })
        }
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
