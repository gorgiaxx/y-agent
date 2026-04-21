//! Ollama provider backend.
//!
//! Implements the Ollama REST API format with:
//! - `/api/chat` endpoint for chat completions
//! - Streaming JSON responses (one JSON object per line)
//! - No API key required (local provider)
//! - Tool calling support via Ollama's function calling

use async_trait::async_trait;

use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::instrument;

use y_core::provider::{
    ChatRequest, ChatResponse, ChatStreamChunk, ChatStreamResponse, FinishReason, LlmProvider,
    ProviderCapability, ProviderError, ProviderMetadata, ProviderType, RequestMode,
    ToolCallingMode,
};
use y_core::types::ToolCallRequest;
use y_core::types::{ProviderId, TokenUsage};

const OLLAMA_DEFAULT_URL: &str = "http://localhost:11434";

/// Ollama local LLM provider.
#[derive(Debug)]
pub struct OllamaProvider {
    client: Client,
    base_url: String,
    metadata: ProviderMetadata,
}

impl OllamaProvider {
    /// Create a new Ollama provider.
    ///
    /// Ollama runs locally so no API key is needed. The `api_key` argument
    /// is accepted for interface consistency but ignored.
    pub fn new(
        id: &str,
        model: &str,
        _api_key: String,
        base_url: Option<String>,
        proxy_url: Option<String>,
        tags: Vec<String>,
        capabilities: Vec<ProviderCapability>,
        max_concurrency: usize,
        context_window: usize,
        tool_calling_mode: ToolCallingMode,
    ) -> Self {
        let base_url = base_url.unwrap_or_else(|| OLLAMA_DEFAULT_URL.to_string());

        // Ollama is typically local, but proxy is still applied if configured.
        // Operators should set `enabled = false` in proxy config to bypass.
        let mut builder = Client::builder();
        if let Some(proxy) = proxy_url {
            if let Ok(p) = reqwest::Proxy::all(&proxy) {
                builder = builder.proxy(p);
            }
        }
        let client = builder.build().unwrap_or_else(|_| Client::new());

        Self {
            client,
            base_url,
            metadata: ProviderMetadata {
                id: ProviderId::from_string(id),
                provider_type: ProviderType::Ollama,
                model: model.to_string(),
                tags,
                capabilities,
                max_concurrency,
                context_window,
                cost_per_1k_input: 0.0,
                cost_per_1k_output: 0.0,
                tool_calling_mode,
            },
        }
    }

    /// Build the full API URL for a given endpoint.
    fn api_url(&self, endpoint: &str) -> String {
        format!("{}/{}", self.base_url.trim_end_matches('/'), endpoint)
    }

    /// Build Ollama messages from `ChatRequest`.
    fn build_messages(request: &ChatRequest) -> Vec<OllamaMessage> {
        request
            .messages
            .iter()
            .map(|m| {
                let role = match m.role {
                    y_core::types::Role::User => "user",
                    y_core::types::Role::Assistant => "assistant",
                    y_core::types::Role::System => "system",
                    y_core::types::Role::Tool => "tool",
                };
                OllamaMessage {
                    role: role.to_string(),
                    content: m.content.clone(),
                    tool_calls: None,
                }
            })
            .collect()
    }

    /// Build Ollama tool definitions.
    fn build_tools(request: &ChatRequest) -> Option<Vec<OllamaTool>> {
        use y_core::provider::ToolCallingMode;

        // PromptBased mode: never send tool definitions to the provider.
        if request.tool_calling_mode == ToolCallingMode::PromptBased {
            return None;
        }

        if request.tools.is_empty() {
            return None;
        }

        let tools: Vec<OllamaTool> = request
            .tools
            .iter()
            .filter_map(|t| {
                let func = t.get("function")?;
                Some(OllamaTool {
                    r#type: "function".into(),
                    function: OllamaFunction {
                        name: func.get("name")?.as_str()?.to_string(),
                        description: func
                            .get("description")
                            .and_then(|d| d.as_str())
                            .map(String::from)
                            .unwrap_or_default(),
                        parameters: func
                            .get("parameters")
                            .cloned()
                            .unwrap_or(serde_json::json!({"type": "object", "properties": {}})),
                    },
                })
            })
            .collect();

        if tools.is_empty() {
            None
        } else {
            Some(tools)
        }
    }

    /// Build the Ollama request body.
    fn build_request_body(&self, request: &ChatRequest, stream: bool) -> OllamaRequest {
        let model = request
            .model
            .as_deref()
            .unwrap_or(&self.metadata.model)
            .to_string();

        let options = OllamaOptions {
            temperature: request.temperature,
            top_p: request.top_p,
            num_predict: request.max_tokens.map(i64::from),
            stop: if request.stop.is_empty() {
                None
            } else {
                Some(request.stop.clone())
            },
        };

        OllamaRequest {
            model,
            messages: Self::build_messages(request),
            stream,
            tools: Self::build_tools(request),
            options: Some(options),
        }
    }
}

#[async_trait]
impl LlmProvider for OllamaProvider {
    #[instrument(skip(self, request), fields(model = %self.metadata.model, provider_id = %self.metadata.id))]
    async fn chat_completion(&self, request: &ChatRequest) -> Result<ChatResponse, ProviderError> {
        if request.request_mode == RequestMode::ImageGeneration {
            return Err(ProviderError::Other {
                message: "dedicated image generation is not implemented for ollama providers"
                    .into(),
            });
        }

        let body = self.build_request_body(request, false);
        let raw_request = serde_json::to_value(&body).ok();

        let response = self
            .client
            .post(self.api_url("api/chat"))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::NetworkError {
                message: format!("Ollama connection error (is Ollama running?): {e}"),
            })?;

        let status = response.status();

        if !status.is_success() {
            let error_body = response.text().await.unwrap_or_default();
            return Err(ProviderError::ServerError {
                provider: self.metadata.id.to_string(),
                message: format!("HTTP {status}: {error_body}"),
            });
        }

        let response_text = response.text().await.map_err(|e| ProviderError::Other {
            message: format!("read response body: {e}"),
        })?;
        let raw_response: serde_json::Value =
            serde_json::from_str(&response_text).map_err(|e| ProviderError::Other {
                message: format!("parse response JSON: {e}"),
            })?;

        let ollama_response: OllamaResponse = serde_json::from_value(raw_response.clone())
            .map_err(|e| ProviderError::Other {
                message: format!("parse response: {e}"),
            })?;

        let content = if ollama_response.message.content.is_empty() {
            None
        } else {
            Some(ollama_response.message.content)
        };

        let tool_calls = ollama_response
            .message
            .tool_calls
            .unwrap_or_default()
            .into_iter()
            .enumerate()
            .map(|(i, tc)| ToolCallRequest {
                id: format!("call_{i}"),
                name: tc.function.name,
                arguments: tc.function.arguments,
            })
            .collect::<Vec<_>>();

        let finish_reason = if ollama_response.done {
            if tool_calls.is_empty() {
                match ollama_response.done_reason.as_deref() {
                    Some("length") => FinishReason::Length,
                    _ => FinishReason::Stop,
                }
            } else {
                FinishReason::ToolUse
            }
        } else {
            FinishReason::Unknown
        };

        // Ollama reports token counts.
        let usage = TokenUsage {
            input_tokens: ollama_response.prompt_eval_count.unwrap_or(0),
            output_tokens: ollama_response.eval_count.unwrap_or(0),
            cache_read_tokens: None,
            cache_write_tokens: None,
            ..Default::default()
        };

        Ok(ChatResponse {
            id: String::new(),
            model: ollama_response.model,
            content,
            reasoning_content: None,
            tool_calls,
            usage,
            finish_reason,
            raw_request,
            raw_response: Some(raw_response),
            provider_id: None,
            generated_images: vec![],
        })
    }

    #[instrument(skip(self, request), fields(model = %self.metadata.model, provider_id = %self.metadata.id))]
    async fn chat_completion_stream(
        &self,
        request: &ChatRequest,
    ) -> Result<ChatStreamResponse, ProviderError> {
        if request.request_mode == RequestMode::ImageGeneration {
            return Err(ProviderError::Other {
                message: "dedicated image generation is not implemented for ollama providers"
                    .into(),
            });
        }

        let body = self.build_request_body(request, true);
        let raw_request = serde_json::to_value(&body).ok();

        let response = self
            .client
            .post(self.api_url("api/chat"))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::NetworkError {
                message: format!("Ollama connection error (is Ollama running?): {e}"),
            })?;

        let status = response.status();
        if !status.is_success() {
            let error_body = response.text().await.unwrap_or_default();
            return Err(ProviderError::ServerError {
                provider: self.metadata.id.to_string(),
                message: format!("HTTP {status}: {error_body}"),
            });
        }

        let byte_stream = response.bytes_stream();

        let stream = futures::stream::unfold(
            crate::sse::SseStreamState::new(Box::pin(byte_stream)),
            move |mut state| async move {
                if state.done {
                    return None;
                }

                loop {
                    // Ollama streams one JSON object per line.
                    if let Some(line) = crate::sse::extract_json_line(&mut state.buffer) {
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }

                        match serde_json::from_str::<OllamaStreamChunk>(trimmed) {
                            Ok(chunk) => {
                                if chunk.done {
                                    state.done = true;
                                    // Emit final chunk with usage.
                                    let usage = Some(TokenUsage {
                                        input_tokens: chunk.prompt_eval_count.unwrap_or(0),
                                        output_tokens: chunk.eval_count.unwrap_or(0),
                                        cache_read_tokens: None,
                                        cache_write_tokens: None,
                                        ..Default::default()
                                    });
                                    return Some((
                                        Ok(ChatStreamChunk {
                                            delta_content: None,
                                            delta_reasoning_content: None,
                                            delta_tool_calls: vec![],
                                            usage,
                                            finish_reason: Some(FinishReason::Stop),
                                            delta_images: vec![],
                                        }),
                                        state,
                                    ));
                                }

                                let content = if chunk.message.content.is_empty() {
                                    None
                                } else {
                                    Some(chunk.message.content)
                                };

                                return Some((
                                    Ok(ChatStreamChunk {
                                        delta_content: content,
                                        delta_reasoning_content: None,
                                        delta_tool_calls: vec![],
                                        usage: None,
                                        finish_reason: None,
                                        delta_images: vec![],
                                    }),
                                    state,
                                ));
                            }
                            Err(e) => {
                                return Some((
                                    Err(ProviderError::ParseError {
                                        message: format!(
                                            "Ollama JSON parse error: {e}, line: {trimmed}"
                                        ),
                                    }),
                                    state,
                                ));
                            }
                        }
                    }

                    // Need more data.
                    match state.read_next().await {
                        Ok(true) => {} // Data appended to buffer, loop again.
                        Ok(false) => {
                            // Stream ended.
                            return None;
                        }
                        Err(e) => {
                            return Some((Err(e), state));
                        }
                    }
                }
            },
        );

        Ok(ChatStreamResponse {
            stream: Box::pin(stream),
            raw_request,
            provider_id: None,
            model: String::new(),
            context_window: 0,
        })
    }

    fn metadata(&self) -> &ProviderMetadata {
        &self.metadata
    }
}

// (SSE state and extract_json_line are now in crate::sse)

// ---------------------------------------------------------------------------
// Ollama API types (internal)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct OllamaRequest {
    model: String,
    messages: Vec<OllamaMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<OllamaTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    options: Option<OllamaOptions>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OllamaMessage {
    role: String,
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OllamaToolCall>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OllamaToolCall {
    function: OllamaToolCallFunction,
}

#[derive(Debug, Serialize, Deserialize)]
struct OllamaToolCallFunction {
    name: String,
    arguments: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct OllamaTool {
    r#type: String,
    function: OllamaFunction,
}

#[derive(Debug, Serialize)]
struct OllamaFunction {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct OllamaOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    num_predict: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct OllamaResponse {
    model: String,
    message: OllamaMessage,
    done: bool,
    done_reason: Option<String>,
    prompt_eval_count: Option<u32>,
    eval_count: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct OllamaStreamChunk {
    #[allow(dead_code)]
    model: Option<String>,
    message: OllamaStreamMessage,
    done: bool,
    prompt_eval_count: Option<u32>,
    eval_count: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct OllamaStreamMessage {
    #[allow(dead_code)]
    role: Option<String>,
    content: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sse::extract_json_line;
    use y_core::provider::ToolCallingMode;

    #[test]
    fn test_ollama_provider_metadata() {
        let provider = OllamaProvider::new(
            "ollama-local",
            "llama3.1:8b",
            String::new(), // No API key needed.
            None,
            None,
            vec!["local".into(), "fast".into(), "free".into()],
            vec![],
            3,
            32_768,
            ToolCallingMode::default(),
        );

        let meta = provider.metadata();
        assert_eq!(meta.id, ProviderId::from_string("ollama-local"));
        assert_eq!(meta.model, "llama3.1:8b");
        assert_eq!(meta.provider_type, ProviderType::Ollama);
        assert_eq!(meta.tags, vec!["local", "fast", "free"]);
        // Local provider has zero cost.
        assert_eq!(meta.cost_per_1k_input, 0.0);
        assert_eq!(meta.cost_per_1k_output, 0.0);
    }

    #[test]
    fn test_ollama_api_url() {
        let provider = OllamaProvider::new(
            "test",
            "llama3",
            String::new(),
            None,
            None,
            vec![],
            vec![],
            3,
            32_768,
            ToolCallingMode::default(),
        );
        assert_eq!(
            provider.api_url("api/chat"),
            "http://localhost:11434/api/chat"
        );
    }

    #[test]
    fn test_ollama_custom_base_url() {
        let provider = OllamaProvider::new(
            "test",
            "llama3",
            String::new(),
            Some("http://192.168.1.100:11434".into()),
            None,
            vec![],
            vec![],
            3,
            32_768,
            ToolCallingMode::default(),
        );
        assert_eq!(
            provider.api_url("api/chat"),
            "http://192.168.1.100:11434/api/chat"
        );
    }

    #[test]
    fn test_ollama_request_serialization() {
        let req = OllamaRequest {
            model: "llama3.1:8b".into(),
            messages: vec![OllamaMessage {
                role: "user".into(),
                content: "Hello".into(),
                tool_calls: None,
            }],
            stream: false,
            tools: None,
            options: Some(OllamaOptions {
                temperature: Some(0.7),
                top_p: None,
                num_predict: Some(100),
                stop: None,
            }),
        };

        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["model"], "llama3.1:8b");
        assert_eq!(json["messages"][0]["role"], "user");
        assert_eq!(json["messages"][0]["content"], "Hello");
        assert!(!json["stream"].as_bool().unwrap());
        let temp = json["options"]["temperature"].as_f64().unwrap();
        assert!(
            (temp - 0.7).abs() < 0.001,
            "temperature should be ~0.7, got {temp}"
        );
        assert_eq!(json["options"]["num_predict"], 100);
    }

    #[test]
    fn test_ollama_response_deserialization() {
        let json = serde_json::json!({
            "model": "llama3.1:8b",
            "message": {
                "role": "assistant",
                "content": "Hello!"
            },
            "done": true,
            "done_reason": "stop",
            "prompt_eval_count": 10,
            "eval_count": 5
        });

        let response: OllamaResponse = serde_json::from_value(json).expect("deserialize");
        assert_eq!(response.model, "llama3.1:8b");
        assert_eq!(response.message.content, "Hello!");
        assert!(response.done);
        assert_eq!(response.prompt_eval_count, Some(10));
        assert_eq!(response.eval_count, Some(5));
    }

    #[test]
    fn test_ollama_stream_chunk_deserialization() {
        let json = serde_json::json!({
            "model": "llama3.1:8b",
            "message": {"role": "assistant", "content": "Hi"},
            "done": false
        });

        let chunk: OllamaStreamChunk = serde_json::from_value(json).expect("deserialize");
        assert_eq!(chunk.message.content, "Hi");
        assert!(!chunk.done);
    }

    #[test]
    fn test_ollama_extract_json_line() {
        let mut buffer = String::from("{\"done\": false, \"message\": {\"content\": \"hi\"}}\n");
        let line = extract_json_line(&mut buffer);
        assert!(line.is_some());
        assert!(buffer.is_empty());
    }

    #[test]
    fn test_ollama_extract_json_line_incomplete() {
        let mut buffer = String::from("{\"done\": false, \"message\": {\"cont");
        let line = extract_json_line(&mut buffer);
        assert!(line.is_none());
        assert!(buffer.contains("cont"));
    }

    #[test]
    fn test_ollama_response_with_tool_calls() {
        let json = serde_json::json!({
            "model": "llama3.1:8b",
            "message": {
                "role": "assistant",
                "content": "",
                "tool_calls": [{
                    "function": {
                        "name": "get_weather",
                        "arguments": {"location": "Tokyo"}
                    }
                }]
            },
            "done": true,
            "done_reason": "stop",
            "prompt_eval_count": 20,
            "eval_count": 15
        });

        let response: OllamaResponse = serde_json::from_value(json).expect("deserialize");
        assert!(response.message.tool_calls.is_some());
        let tool_calls = response.message.tool_calls.unwrap();
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].function.name, "get_weather");
    }

    #[test]
    fn test_ollama_build_messages() {
        use y_core::types::{Message, Role};

        let request = ChatRequest {
            messages: vec![
                Message {
                    message_id: String::new(),
                    role: Role::System,
                    content: "Be helpful".into(),
                    tool_call_id: None,
                    tool_calls: vec![],
                    timestamp: y_core::types::now(),
                    metadata: serde_json::Value::Null,
                },
                Message {
                    message_id: String::new(),
                    role: Role::User,
                    content: "Hello".into(),
                    tool_call_id: None,
                    tool_calls: vec![],
                    timestamp: y_core::types::now(),
                    metadata: serde_json::Value::Null,
                },
            ],
            model: None,
            request_mode: RequestMode::TextChat,
            max_tokens: None,
            temperature: None,
            top_p: None,
            tools: vec![],
            tool_calling_mode: ToolCallingMode::default(),
            stop: vec![],
            extra: serde_json::Value::Null,
            thinking: None,
            response_format: None,
            image_generation_options: None,
        };

        let messages = OllamaProvider::build_messages(&request);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "system");
        assert_eq!(messages[1].role, "user");
    }
}
