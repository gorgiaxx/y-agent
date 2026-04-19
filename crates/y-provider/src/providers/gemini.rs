//! Google Gemini provider backend.
//!
//! Implements the Gemini generateContent API format with:
//! - `contents` array with `parts` structure
//! - System instructions via `system_instruction` field
//! - `x-goog-api-key` header authentication
//! - Streaming support via SSE (generateContent?alt=sse)

use async_trait::async_trait;

use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::instrument;

use y_core::provider::{
    ChatRequest, ChatResponse, ChatStreamChunk, ChatStreamResponse, FinishReason, GeneratedImage,
    LlmProvider, ProviderError, ProviderMetadata, ProviderType, ToolCallingMode,
};
use y_core::types::ToolCallRequest;
use y_core::types::{ProviderId, TokenUsage};

const GEMINI_API_URL: &str = "https://generativelanguage.googleapis.com/v1beta";

/// Google Gemini provider.
#[derive(Debug)]
pub struct GeminiProvider {
    client: Client,
    api_key: String,
    base_url: String,
    metadata: ProviderMetadata,
}

impl GeminiProvider {
    /// Create a new Gemini provider.
    pub fn new(
        id: &str,
        model: &str,
        api_key: String,
        base_url: Option<String>,
        proxy_url: Option<String>,
        tags: Vec<String>,
        max_concurrency: usize,
        context_window: usize,
        tool_calling_mode: ToolCallingMode,
    ) -> Self {
        let base_url = base_url.unwrap_or_else(|| GEMINI_API_URL.to_string());

        let mut builder = Client::builder();
        if let Some(proxy) = proxy_url {
            if let Ok(p) = reqwest::Proxy::all(&proxy) {
                builder = builder.proxy(p);
            }
        }

        Self {
            client: builder.build().unwrap_or_else(|_| Client::new()),
            api_key,
            base_url,
            metadata: ProviderMetadata {
                id: ProviderId::from_string(id),
                provider_type: ProviderType::Gemini,
                model: model.to_string(),
                tags,
                max_concurrency,
                context_window,
                cost_per_1k_input: 0.0,
                cost_per_1k_output: 0.0,
                tool_calling_mode,
            },
        }
    }

    /// Build the generateContent URL (non-streaming).
    fn generate_url(&self) -> String {
        format!(
            "{}/models/{}:generateContent",
            self.base_url.trim_end_matches('/'),
            self.metadata.model
        )
    }

    /// Build the streamGenerateContent URL.
    fn stream_url(&self) -> String {
        format!(
            "{}/models/{}:streamGenerateContent?alt=sse",
            self.base_url.trim_end_matches('/'),
            self.metadata.model
        )
    }

    /// Extract system instruction from messages.
    fn extract_system_instruction(request: &ChatRequest) -> Option<GeminiContent> {
        request
            .messages
            .iter()
            .find(|m| m.role == y_core::types::Role::System)
            .map(|m| GeminiContent {
                role: None,
                parts: vec![GeminiPart::Text {
                    text: m.content.clone(),
                }],
            })
    }

    /// Build Gemini contents from `ChatRequest` messages (excluding system).
    fn build_contents(request: &ChatRequest) -> Vec<GeminiContent> {
        request
            .messages
            .iter()
            .filter(|m| m.role != y_core::types::Role::System)
            .map(|m| {
                let role = match m.role {
                    y_core::types::Role::User => "user",
                    y_core::types::Role::Assistant => "model",
                    y_core::types::Role::Tool => "function",
                    y_core::types::Role::System => unreachable!(),
                };

                // If this is a tool result, wrap it as a function response.
                if m.role == y_core::types::Role::Tool {
                    if let Some(ref tool_call_id) = m.tool_call_id {
                        let response_value: serde_json::Value = serde_json::from_str(&m.content)
                            .unwrap_or_else(|_| serde_json::json!({"result": m.content}));
                        return GeminiContent {
                            role: Some(role.to_string()),
                            parts: vec![GeminiPart::FunctionResponse {
                                function_response: GeminiFunctionResponse {
                                    name: tool_call_id.clone(),
                                    response: response_value,
                                },
                            }],
                        };
                    }
                }

                GeminiContent {
                    role: Some(role.to_string()),
                    parts: vec![GeminiPart::Text {
                        text: m.content.clone(),
                    }],
                }
            })
            .collect()
    }

    /// Build tool declarations for Gemini format.
    fn build_tools(request: &ChatRequest) -> Option<Vec<GeminiToolDeclaration>> {
        use y_core::provider::ToolCallingMode;

        // PromptBased mode: never send tool definitions to the provider.
        if request.tool_calling_mode == ToolCallingMode::PromptBased {
            return None;
        }

        if request.tools.is_empty() {
            return None;
        }

        let declarations: Vec<GeminiFunctionDeclaration> = request
            .tools
            .iter()
            .filter_map(|t| {
                let func = t.get("function")?;
                Some(GeminiFunctionDeclaration {
                    name: func.get("name")?.as_str()?.to_string(),
                    description: func
                        .get("description")
                        .and_then(|d| d.as_str())
                        .map(String::from)
                        .unwrap_or_default(),
                    parameters: func.get("parameters").cloned(),
                })
            })
            .collect();

        if declarations.is_empty() {
            None
        } else {
            Some(vec![GeminiToolDeclaration {
                function_declarations: declarations,
            }])
        }
    }

    /// Build the Gemini request body.
    fn build_request_body(request: &ChatRequest) -> GeminiRequest {
        let system_instruction = Self::extract_system_instruction(request);
        let contents = Self::build_contents(request);
        let tools = Self::build_tools(request);

        let generation_config = Some(GeminiGenerationConfig {
            max_output_tokens: request.max_tokens,
            temperature: request.temperature,
            top_p: request.top_p,
            stop_sequences: if request.stop.is_empty() {
                None
            } else {
                Some(request.stop.clone())
            },
        });

        GeminiRequest {
            contents,
            system_instruction,
            tools,
            generation_config,
        }
    }

    /// Parse a successful Gemini response into `ChatResponse`.
    fn parse_response(
        &self,
        gemini_response: GeminiResponse,
        raw_request: Option<serde_json::Value>,
        raw_response: Option<serde_json::Value>,
    ) -> Result<ChatResponse, ProviderError> {
        let candidate = gemini_response
            .candidates
            .into_iter()
            .next()
            .ok_or_else(|| ProviderError::Other {
                message: "no candidates in Gemini response".into(),
            })?;

        let mut text_parts = Vec::new();
        let mut tool_calls = Vec::new();
        let mut generated_images = Vec::new();

        for part in &candidate.content.parts {
            match part {
                GeminiPart::Text { text } => {
                    text_parts.push(text.clone());
                }
                GeminiPart::FunctionCall { function_call } => {
                    tool_calls.push(ToolCallRequest {
                        id: format!("call_{}", &uuid::Uuid::new_v4().simple().to_string()[..24]),
                        name: function_call.name.clone(),
                        arguments: function_call.args.clone(),
                    });
                }
                GeminiPart::InlineData { inline_data } => {
                    generated_images.push(GeneratedImage {
                        index: generated_images.len(),
                        mime_type: inline_data.mime_type.clone(),
                        data: inline_data.data.clone(),
                    });
                }
                GeminiPart::FunctionResponse { .. } => {}
            }
        }

        let content = if text_parts.is_empty() {
            None
        } else {
            Some(text_parts.join(""))
        };

        let finish_reason = match candidate.finish_reason.as_deref() {
            Some("STOP") => FinishReason::Stop,
            Some("MAX_TOKENS") => FinishReason::Length,
            Some("SAFETY") => FinishReason::ContentFilter,
            Some("TOOL_USE") => FinishReason::ToolUse,
            _ => {
                if tool_calls.is_empty() {
                    FinishReason::Stop
                } else {
                    FinishReason::ToolUse
                }
            }
        };

        let usage = gemini_response.usage_metadata.map_or(
            TokenUsage {
                input_tokens: 0,
                output_tokens: 0,
                cache_read_tokens: None,
                cache_write_tokens: None,
                ..Default::default()
            },
            |u| TokenUsage {
                input_tokens: u.prompt.unwrap_or(0),
                output_tokens: u.candidates.unwrap_or(0),
                cache_read_tokens: u.cached_content,
                cache_write_tokens: None,
                ..Default::default()
            },
        );

        Ok(ChatResponse {
            id: String::new(), // Gemini doesn't return a response ID in the same way.
            model: self.metadata.model.clone(),
            content,
            reasoning_content: None,
            tool_calls,
            usage,
            finish_reason,
            raw_request,
            raw_response,
            provider_id: None,
            generated_images,
        })
    }
}

#[async_trait]
impl LlmProvider for GeminiProvider {
    #[instrument(skip(self, request), fields(model = %self.metadata.model, provider_id = %self.metadata.id))]
    async fn chat_completion(&self, request: &ChatRequest) -> Result<ChatResponse, ProviderError> {
        let body = Self::build_request_body(request);
        let raw_request = serde_json::to_value(&body).ok();

        let mut request_builder = self
            .client
            .post(self.generate_url())
            .header("Content-Type", "application/json");

        if !self.api_key.is_empty() {
            request_builder = request_builder.header("x-goog-api-key", &self.api_key);
        }

        let response =
            request_builder
                .json(&body)
                .send()
                .await
                .map_err(|e| ProviderError::NetworkError {
                    message: e.to_string(),
                })?;

        let status = response.status();

        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse().ok())
                .unwrap_or(60u64);
            return Err(ProviderError::RateLimited {
                provider: self.metadata.id.to_string(),
                retry_after_secs: retry_after,
            });
        }

        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            let error_body = response.text().await.unwrap_or_default();
            return Err(ProviderError::AuthenticationFailed {
                provider: self.metadata.id.to_string(),
                message: error_body,
            });
        }

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

        let gemini_response: GeminiResponse = serde_json::from_value(raw_response.clone())
            .map_err(|e| ProviderError::Other {
                message: format!("parse response: {e}"),
            })?;

        self.parse_response(gemini_response, raw_request, Some(raw_response))
    }

    #[instrument(skip(self, request), fields(model = %self.metadata.model, provider_id = %self.metadata.id))]
    async fn chat_completion_stream(
        &self,
        request: &ChatRequest,
    ) -> Result<ChatStreamResponse, ProviderError> {
        let body = Self::build_request_body(request);
        let raw_request = serde_json::to_value(&body).ok();

        let mut request_builder = self
            .client
            .post(self.stream_url())
            .header("Content-Type", "application/json");

        if !self.api_key.is_empty() {
            request_builder = request_builder.header("x-goog-api-key", &self.api_key);
        }

        let response =
            request_builder
                .json(&body)
                .send()
                .await
                .map_err(|e| ProviderError::NetworkError {
                    message: e.to_string(),
                })?;

        let status = response.status();
        if !status.is_success() {
            let error_body = response.text().await.unwrap_or_default();
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                return Err(ProviderError::RateLimited {
                    provider: self.metadata.id.to_string(),
                    retry_after_secs: 60,
                });
            }
            if status == reqwest::StatusCode::UNAUTHORIZED
                || status == reqwest::StatusCode::FORBIDDEN
            {
                return Err(ProviderError::AuthenticationFailed {
                    provider: self.metadata.id.to_string(),
                    message: error_body,
                });
            }
            return Err(ProviderError::ServerError {
                provider: self.metadata.id.to_string(),
                message: format!("HTTP {status}: {error_body}"),
            });
        }

        let byte_stream = response.bytes_stream();
        let model = self.metadata.model.clone();

        let stream = futures::stream::unfold(
            crate::sse::SseStreamState::new(Box::pin(byte_stream)),
            move |mut state| {
                let model = model.clone();
                async move {
                    if state.done {
                        return None;
                    }

                    loop {
                        // Try to extract a complete SSE event from the buffer.
                        if let Some(data) = crate::sse::extract_sse_data(&mut state.buffer) {
                            let trimmed = data.trim();
                            if trimmed.is_empty() {
                                continue;
                            }

                            // Parse as Gemini response (each SSE chunk is a full response object).
                            match serde_json::from_str::<GeminiResponse>(trimmed) {
                                Ok(resp) => {
                                    let chunk = map_gemini_stream_chunk(&resp, &model);
                                    return Some((Ok(chunk), state));
                                }
                                Err(e) => {
                                    return Some((
                                        Err(ProviderError::ParseError {
                                            message: format!(
                                                "Gemini SSE parse error: {e}, data: {trimmed}"
                                            ),
                                        }),
                                        state,
                                    ));
                                }
                            }
                        }

                        // Need more data from network.
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

// (SSE state and extract_sse_data are now in crate::sse)

/// Map a Gemini streaming response chunk to a `ChatStreamChunk`.
fn map_gemini_stream_chunk(resp: &GeminiResponse, _model: &str) -> ChatStreamChunk {
    let candidate = resp.candidates.first();

    let mut delta_content = None;
    let mut delta_tool_calls = Vec::new();

    if let Some(candidate) = candidate {
        for part in &candidate.content.parts {
            match part {
                GeminiPart::Text { text } => {
                    delta_content = Some(text.clone());
                }
                GeminiPart::FunctionCall { function_call } => {
                    delta_tool_calls.push(ToolCallRequest {
                        id: format!("call_{}", &uuid::Uuid::new_v4().simple().to_string()[..24]),
                        name: function_call.name.clone(),
                        arguments: function_call.args.clone(),
                    });
                }
                GeminiPart::FunctionResponse { .. } => {}
            }
        }
    }

    let finish_reason = candidate.and_then(|c| {
        c.finish_reason.as_deref().map(|r| match r {
            "STOP" => FinishReason::Stop,
            "MAX_TOKENS" => FinishReason::Length,
            "SAFETY" => FinishReason::ContentFilter,
            _ => FinishReason::Unknown,
        })
    });

    let usage = resp.usage_metadata.as_ref().map(|u| TokenUsage {
        input_tokens: u.prompt.unwrap_or(0),
        output_tokens: u.candidates.unwrap_or(0),
        cache_read_tokens: u.cached_content,
        cache_write_tokens: None,
        ..Default::default()
    });

    ChatStreamChunk {
        delta_content,
        delta_reasoning_content: None,
        delta_tool_calls,
        usage,
        finish_reason,
        delta_images: vec![],
    }
}

// ---------------------------------------------------------------------------
// Gemini API types (internal)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiRequest {
    contents: Vec<GeminiContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system_instruction: Option<GeminiContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<GeminiToolDeclaration>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    generation_config: Option<GeminiGenerationConfig>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct GeminiContent {
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<String>,
    parts: Vec<GeminiPart>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(untagged)]
enum GeminiPart {
    Text {
        text: String,
    },
    FunctionCall {
        #[serde(rename = "functionCall")]
        function_call: GeminiFunctionCall,
    },
    FunctionResponse {
        #[serde(rename = "functionResponse")]
        function_response: GeminiFunctionResponse,
    },
    #[serde(rename = "inlineData")]
    InlineData {
        #[serde(rename = "inlineData")]
        inline_data: GeminiInlineData,
    },
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct GeminiInlineData {
    #[serde(rename = "mimeType")]
    mime_type: String,
    data: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct GeminiFunctionCall {
    name: String,
    args: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct GeminiFunctionResponse {
    name: String,
    response: serde_json::Value,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiGenerationConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop_sequences: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiToolDeclaration {
    function_declarations: Vec<GeminiFunctionDeclaration>,
}

#[derive(Debug, Serialize)]
struct GeminiFunctionDeclaration {
    name: String,
    description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    parameters: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiResponse {
    candidates: Vec<GeminiCandidate>,
    usage_metadata: Option<GeminiUsageMetadata>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiCandidate {
    content: GeminiContent,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiUsageMetadata {
    #[serde(rename = "promptTokenCount")]
    prompt: Option<u32>,
    #[serde(rename = "candidatesTokenCount")]
    candidates: Option<u32>,
    #[serde(rename = "cachedContentTokenCount")]
    cached_content: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sse::extract_sse_data;
    use y_core::provider::ToolCallingMode;

    #[test]
    fn test_gemini_provider_metadata() {
        let provider = GeminiProvider::new(
            "gemini-main",
            "gemini-2.0-flash",
            "AIza-test".into(),
            None,
            None,
            vec!["fast".into(), "general".into()],
            5,
            1_000_000,
            ToolCallingMode::default(),
        );

        let meta = provider.metadata();
        assert_eq!(meta.id, ProviderId::from_string("gemini-main"));
        assert_eq!(meta.model, "gemini-2.0-flash");
        assert_eq!(meta.provider_type, ProviderType::Gemini);
        assert_eq!(meta.tags, vec!["fast", "general"]);
    }

    #[test]
    fn test_gemini_generate_url() {
        let provider = GeminiProvider::new(
            "test",
            "gemini-2.0-flash",
            "test-key".into(),
            None,
            None,
            vec![],
            5,
            1_000_000,
            ToolCallingMode::default(),
        );
        assert_eq!(
            provider.generate_url(),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.0-flash:generateContent"
        );
    }

    #[test]
    fn test_gemini_stream_url() {
        let provider = GeminiProvider::new(
            "test",
            "gemini-2.0-flash",
            "test-key".into(),
            None,
            None,
            vec![],
            5,
            1_000_000,
            ToolCallingMode::default(),
        );
        assert_eq!(
            provider.stream_url(),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.0-flash:streamGenerateContent?alt=sse"
        );
    }

    #[test]
    fn test_gemini_custom_base_url() {
        let provider = GeminiProvider::new(
            "test",
            "gemini-2.0-flash",
            "test-key".into(),
            Some("http://localhost:8080/v1beta".into()),
            None,
            vec![],
            5,
            1_000_000,
            ToolCallingMode::default(),
        );
        assert_eq!(
            provider.generate_url(),
            "http://localhost:8080/v1beta/models/gemini-2.0-flash:generateContent"
        );
    }

    #[test]
    fn test_gemini_request_serialization() {
        let req = GeminiRequest {
            contents: vec![GeminiContent {
                role: Some("user".into()),
                parts: vec![GeminiPart::Text {
                    text: "Hello".into(),
                }],
            }],
            system_instruction: Some(GeminiContent {
                role: None,
                parts: vec![GeminiPart::Text {
                    text: "You are a helpful assistant.".into(),
                }],
            }),
            tools: None,
            generation_config: Some(GeminiGenerationConfig {
                max_output_tokens: Some(1024),
                temperature: Some(0.7),
                top_p: None,
                stop_sequences: None,
            }),
        };

        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["contents"][0]["role"], "user");
        assert_eq!(json["contents"][0]["parts"][0]["text"], "Hello");
        assert_eq!(
            json["systemInstruction"]["parts"][0]["text"],
            "You are a helpful assistant."
        );
        assert_eq!(json["generationConfig"]["maxOutputTokens"], 1024);
    }

    #[test]
    fn test_gemini_response_deserialization() {
        let json = serde_json::json!({
            "candidates": [{
                "content": {
                    "parts": [{"text": "Hello!"}],
                    "role": "model"
                },
                "finishReason": "STOP"
            }],
            "usageMetadata": {
                "promptTokenCount": 10,
                "candidatesTokenCount": 5
            }
        });

        let response: GeminiResponse = serde_json::from_value(json).expect("deserialize");
        assert_eq!(response.candidates.len(), 1);
        assert_eq!(
            response.candidates[0].finish_reason.as_deref(),
            Some("STOP")
        );
        // assert_eq!(
        //     response.usage_metadata.as_ref().unwrap().prompt,
        //     Some(10)
        // );
    }

    #[test]
    fn test_gemini_response_with_tool_call() {
        let json = serde_json::json!({
            "candidates": [{
                "content": {
                    "parts": [{
                        "functionCall": {
                            "name": "get_weather",
                            "args": {"location": "San Francisco"}
                        }
                    }],
                    "role": "model"
                },
                "finishReason": "STOP"
            }],
            "usageMetadata": {
                "promptTokenCount": 20,
                "candidatesTokenCount": 10
            }
        });

        let response: GeminiResponse = serde_json::from_value(json).expect("deserialize");
        let candidate = &response.candidates[0];
        assert_eq!(candidate.content.parts.len(), 1);
        match &candidate.content.parts[0] {
            GeminiPart::FunctionCall { function_call } => {
                assert_eq!(function_call.name, "get_weather");
                assert_eq!(function_call.args["location"], "San Francisco");
            }
            _ => panic!("expected FunctionCall part"),
        }
    }

    #[test]
    fn test_gemini_tool_declarations_serialization() {
        let tool = GeminiToolDeclaration {
            function_declarations: vec![GeminiFunctionDeclaration {
                name: "get_weather".into(),
                description: "Get weather for a location".into(),
                parameters: Some(serde_json::json!({
                    "type": "object",
                    "properties": {
                        "location": {"type": "string"}
                    }
                })),
            }],
        };

        let json = serde_json::to_value(&tool).expect("serialize");
        assert_eq!(json["functionDeclarations"][0]["name"], "get_weather");
    }

    #[test]
    fn test_gemini_extract_sse_data() {
        let mut buffer = String::from(
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Hi\"}],\"role\":\"model\"}}]}\n\n",
        );
        let data = extract_sse_data(&mut buffer);
        assert!(data.is_some());
        let data = data.unwrap();
        assert!(data.contains("Hi"));
        assert!(buffer.is_empty());
    }

    #[test]
    fn test_gemini_extract_sse_data_incomplete() {
        let mut buffer = String::from("data: partial data without boundary");
        let data = extract_sse_data(&mut buffer);
        assert!(data.is_none());
        assert!(buffer.contains("partial"));
    }

    #[test]
    fn test_gemini_build_contents() {
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
                Message {
                    message_id: String::new(),
                    role: Role::Assistant,
                    content: "Hi there!".into(),
                    tool_call_id: None,
                    tool_calls: vec![],
                    timestamp: y_core::types::now(),
                    metadata: serde_json::Value::Null,
                },
            ],
            model: None,
            max_tokens: None,
            temperature: None,
            top_p: None,
            tools: vec![],
            tool_calling_mode: ToolCallingMode::default(),
            stop: vec![],
            extra: serde_json::Value::Null,
            thinking: None,
            response_format: None,
        };

        // System should be extracted separately.
        let system = GeminiProvider::extract_system_instruction(&request);
        assert!(system.is_some());

        // Contents should not include the system message.
        let contents = GeminiProvider::build_contents(&request);
        assert_eq!(contents.len(), 2);
        assert_eq!(contents[0].role.as_deref(), Some("user"));
        assert_eq!(contents[1].role.as_deref(), Some("model"));
    }
}
