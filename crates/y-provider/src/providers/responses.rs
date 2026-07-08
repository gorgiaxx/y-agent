//! `OpenAI` Responses API wire types and conversion logic.
//!
//! Implements the `/responses` endpoint format (input array of typed items,
//! `max_output_tokens`, nested `reasoning`, `response.output[]` with
//! `message`/`function_call`/`reasoning` items) as documented in the
//! `OpenAI` API reference and the Vercel `@ai-sdk/openai` reference SDK.
//!
//! This is distinct from the OpenAI-compatible Chat Completions wire format
//! (`/chat/completions`) which lives in [`super`].

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use y_core::provider::{FinishReason, ResponseFormat, ThinkingEffort};
use y_core::types::{Message, Role, TokenUsage, ToolCallRequest};

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

/// `OpenAI` Responses API request body.
///
/// See <https://platform.openai.com/docs/api-reference/responses/create>.
#[derive(Debug, Serialize)]
pub(crate) struct ResponsesRequest {
    pub model: String,
    pub input: Vec<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    pub stream: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ResponsesReasoning>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<ResponsesText>,
    /// `store = false` tells the Responses API not to persist the response
    /// server-side. We always set this to `false` because y-agent manages its
    /// own conversation history and does not use `previous_response_id`
    /// chaining.
    pub store: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct ResponsesReasoning {
    pub effort: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ResponsesText {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<ResponsesTextFormat>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
pub(crate) enum ResponsesTextFormat {
    #[serde(rename = "json_object")]
    JsonObject,
    #[serde(rename = "json_schema")]
    JsonSchema {
        name: String,
        schema: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        strict: bool,
    },
}

// ---------------------------------------------------------------------------
// Response types (non-streaming)
// ---------------------------------------------------------------------------

/// `OpenAI` Responses API non-streaming response.
#[derive(Debug, Deserialize)]
pub(crate) struct ResponsesResponse {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub output: Vec<ResponsesOutputItem>,
    #[serde(default)]
    pub usage: Option<ResponsesUsage>,
    #[serde(default)]
    pub incomplete_details: Option<ResponsesIncompleteDetails>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ResponsesIncompleteDetails {
    #[serde(default)]
    pub reason: Option<String>,
}

/// A single output item in a Responses API response.
///
/// We only model the item types y-agent needs: `message`, `function_call`,
/// and `reasoning`. Other item types (`web_search_call`, `computer_call`, etc.)
/// are tolerated via the `#[serde(other)]` catch-all.
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub(crate) enum ResponsesOutputItem {
    #[serde(rename = "message")]
    Message {
        #[serde(default)]
        #[allow(dead_code)]
        id: Option<String>,
        #[serde(default)]
        content: Vec<ResponsesMessageContent>,
    },
    #[serde(rename = "function_call")]
    FunctionCall {
        #[serde(default)]
        #[allow(dead_code)]
        id: Option<String>,
        call_id: String,
        name: String,
        arguments: String,
    },
    #[serde(rename = "reasoning")]
    Reasoning {
        #[serde(default)]
        #[allow(dead_code)]
        id: Option<String>,
        #[serde(default)]
        summary: Vec<ResponsesReasoningSummary>,
    },
    /// Catch-all for item types we don't model (`web_search_call`, etc.).
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub(crate) enum ResponsesMessageContent {
    #[serde(rename = "output_text")]
    OutputText { text: String },
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub(crate) enum ResponsesReasoningSummary {
    #[serde(rename = "summary_text")]
    SummaryText { text: String },
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ResponsesUsage {
    #[serde(default)]
    pub input_tokens: u32,
    #[serde(default)]
    pub output_tokens: u32,
    #[serde(default)]
    pub input_tokens_details: Option<ResponsesInputTokensDetails>,
    #[serde(default)]
    #[allow(dead_code)]
    pub output_tokens_details: Option<ResponsesOutputTokensDetails>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ResponsesInputTokensDetails {
    #[serde(default)]
    pub cached_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ResponsesOutputTokensDetails {
    #[serde(default)]
    #[allow(dead_code)]
    pub reasoning_tokens: Option<u32>,
}

// ---------------------------------------------------------------------------
// Input conversion: ChatRequest messages -> Responses API input array
// ---------------------------------------------------------------------------

/// Convert y-agent messages into the `OpenAI` Responses API `input` array.
///
/// The Responses API uses a flat array of typed items rather than the
/// Chat Completions `messages` array with `role` strings. The mapping is:
///
/// | y-agent Role | Responses API item type                 |
/// |--------------|-----------------------------------------|
/// | System       | `{ role: "system", content: "..." }`   |
/// | User         | `{ role: "user", content: [{...}] }`    |
/// | Assistant    | `{ role: "assistant", content: [...] }` |
/// |              |   + `function_call` items for tool calls |
/// | Tool         | `{ type: "function_call_output", ... }` |
///
/// System messages map to `role: "system"` (not `developer`) for
/// compatibility with non-reasoning models. Reasoning models that reject
/// `system` will surface an error from the API, which is preferable to
/// silently rewriting instructions.
pub(crate) fn build_responses_input(messages: &[Message]) -> Vec<Value> {
    let mut input: Vec<Value> = Vec::new();

    for msg in messages {
        match msg.role {
            Role::System => {
                input.push(json!({
                    "role": "system",
                    "content": msg.content,
                }));
            }
            Role::User => {
                let content =
                    if let Some(arr) = msg.metadata.get("attachments").and_then(|v| v.as_array()) {
                        if arr.is_empty() {
                            vec![json!({ "type": "input_text", "text": msg.content })]
                        } else {
                            let mut parts: Vec<Value> = Vec::new();
                            for att in arr {
                                if let (Some(mime), Some(data)) = (
                                    att.get("mime_type").and_then(|v| v.as_str()),
                                    att.get("base64_data").and_then(|v| v.as_str()),
                                ) {
                                    if mime.starts_with("image/") {
                                        parts.push(json!({
                                            "type": "input_image",
                                            "image_url": format!("data:{mime};base64,{data}"),
                                        }));
                                    }
                                }
                            }
                            if !msg.content.is_empty() {
                                parts.push(json!({ "type": "input_text", "text": msg.content }));
                            }
                            parts
                        }
                    } else {
                        vec![json!({ "type": "input_text", "text": msg.content })]
                    };
                input.push(json!({ "role": "user", "content": content }));
            }
            Role::Assistant => {
                for tc in &msg.tool_calls {
                    let args_str = match &tc.arguments {
                        Value::String(s) => s.clone(),
                        other => serde_json::to_string(other).unwrap_or_else(|_| "{}".into()),
                    };
                    input.push(json!({
                        "type": "function_call",
                        "call_id": tc.id,
                        "name": tc.name,
                        "arguments": args_str,
                    }));
                }
                if !msg.content.is_empty() {
                    input.push(json!({
                        "role": "assistant",
                        "content": [{ "type": "output_text", "text": msg.content }],
                    }));
                }
            }
            Role::Tool => {
                let output = if msg.content.is_empty() {
                    String::from("{}")
                } else {
                    msg.content.clone()
                };
                input.push(json!({
                    "type": "function_call_output",
                    "call_id": msg.tool_call_id,
                    "output": output,
                }));
            }
        }
    }

    input
}

/// Convert y-agent tool definitions into the Responses API tool format.
///
/// y-agent tool defs are already `{ "type": "function", "function": {...} }`
/// (Chat Completions shape). The Responses API uses
/// `{ "type": "function", "name": ..., "parameters": ..., "description": ... }`.
pub(crate) fn build_responses_tools(tools: &[Value]) -> Vec<Value> {
    let mut out = Vec::with_capacity(tools.len());
    for tool in tools {
        if tool.get("type").and_then(|t| t.as_str()) == Some("function")
            && tool.get("name").is_some()
        {
            out.push(tool.clone());
            continue;
        }
        if let Some(func) = tool.get("function") {
            let mut obj = serde_json::Map::new();
            obj.insert("type".to_string(), Value::String("function".to_string()));
            if let Some(name) = func.get("name") {
                obj.insert("name".to_string(), name.clone());
            }
            if let Some(desc) = func.get("description") {
                obj.insert("description".to_string(), desc.clone());
            }
            if let Some(params) = func.get("parameters").or_else(|| func.get("schema")) {
                obj.insert("parameters".to_string(), params.clone());
            }
            if let Some(strict) = func.get("strict") {
                obj.insert("strict".to_string(), strict.clone());
            }
            out.push(Value::Object(obj));
            continue;
        }
        out.push(tool.clone());
    }
    out
}

/// Build the `reasoning` object for the Responses API request.
pub(crate) fn build_responses_reasoning(
    thinking: Option<&y_core::provider::ThinkingConfig>,
) -> Option<ResponsesReasoning> {
    let cfg = thinking?;
    let effort = thinking_effort_str(cfg.effort);
    Some(ResponsesReasoning {
        effort,
        summary: Some("detailed".to_string()),
    })
}

fn thinking_effort_str(effort: ThinkingEffort) -> String {
    match effort {
        ThinkingEffort::Low => "low".to_string(),
        ThinkingEffort::Medium => "medium".to_string(),
        ThinkingEffort::High => "high".to_string(),
        ThinkingEffort::Max => {
            tracing::warn!(
                "ThinkingEffort::Max not supported by OpenAI Responses API; downgrading to 'high'"
            );
            "high".to_string()
        }
    }
}

/// Build the `text.format` object for the Responses API request from the
/// y-agent [`ResponseFormat`].
pub(crate) fn build_responses_text_format(rf: &ResponseFormat) -> Option<ResponsesTextFormat> {
    match rf {
        ResponseFormat::Text => None,
        ResponseFormat::JsonObject => Some(ResponsesTextFormat::JsonObject),
        ResponseFormat::JsonSchema { name, schema } => Some(ResponsesTextFormat::JsonSchema {
            name: name.clone(),
            schema: schema.clone(),
            description: None,
            strict: true,
        }),
    }
}

// ---------------------------------------------------------------------------
// Non-streaming response parsing
// ---------------------------------------------------------------------------

/// Parsed content from a Responses API non-streaming response.
pub(crate) struct ResponsesParsed {
    pub id: String,
    pub model: String,
    pub content: Option<String>,
    pub reasoning_content: Option<String>,
    pub tool_calls: Vec<ToolCallRequest>,
    pub usage: TokenUsage,
    pub finish_reason: FinishReason,
}

/// Parse a Responses API non-streaming JSON response into y-agent types.
pub(crate) fn parse_responses_response(raw: &Value) -> Result<ResponsesParsed, String> {
    let response: ResponsesResponse =
        serde_json::from_value(raw.clone()).map_err(|e| format!("parse response: {e}"))?;

    let mut content_parts: Vec<String> = Vec::new();
    let mut reasoning_parts: Vec<String> = Vec::new();
    let mut tool_calls: Vec<ToolCallRequest> = Vec::new();
    let mut has_function_call = false;

    for item in &response.output {
        match item {
            ResponsesOutputItem::Message { content, .. } => {
                for part in content {
                    if let ResponsesMessageContent::OutputText { text } = part {
                        content_parts.push(text.clone());
                    }
                }
            }
            ResponsesOutputItem::FunctionCall {
                call_id,
                name,
                arguments,
                ..
            } => {
                has_function_call = true;
                let args_val = serde_json::from_str::<Value>(arguments)
                    .unwrap_or_else(|_| Value::String(arguments.clone()));
                tool_calls.push(ToolCallRequest {
                    id: call_id.clone(),
                    name: name.clone(),
                    arguments: args_val,
                });
            }
            ResponsesOutputItem::Reasoning { summary, .. } => {
                for s in summary {
                    if let ResponsesReasoningSummary::SummaryText { text } = s {
                        reasoning_parts.push(text.clone());
                    }
                }
            }
            ResponsesOutputItem::Other => {}
        }
    }

    let content = if content_parts.is_empty() {
        None
    } else {
        Some(content_parts.join(""))
    };
    let reasoning_content = if reasoning_parts.is_empty() {
        None
    } else {
        Some(reasoning_parts.join(""))
    };

    let usage = response.usage.map(|u| {
        let cached = u
            .input_tokens_details
            .as_ref()
            .and_then(|d| d.cached_tokens)
            .unwrap_or(0);
        TokenUsage {
            input_tokens: u.input_tokens.saturating_sub(cached),
            output_tokens: u.output_tokens,
            cache_read_tokens: (cached > 0).then_some(cached),
            cache_write_tokens: None,
            ..Default::default()
        }
    });

    let finish_reason = map_responses_finish_reason(
        response
            .incomplete_details
            .as_ref()
            .and_then(|d| d.reason.as_deref()),
        has_function_call,
    );

    Ok(ResponsesParsed {
        id: response.id,
        model: response.model,
        content,
        reasoning_content,
        tool_calls,
        usage: usage.unwrap_or_default(),
        finish_reason,
    })
}

/// Map Responses API finish reason to y-agent [`FinishReason`].
fn map_responses_finish_reason(
    incomplete_reason: Option<&str>,
    has_function_call: bool,
) -> FinishReason {
    match incomplete_reason {
        None => {
            if has_function_call {
                FinishReason::ToolUse
            } else {
                FinishReason::Stop
            }
        }
        Some("max_output_tokens") => FinishReason::Length,
        Some("content_filter") => FinishReason::ContentFilter,
        Some(_) => {
            if has_function_call {
                FinishReason::ToolUse
            } else {
                FinishReason::Unknown
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Streaming types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ResponsesChunkType {
    #[serde(rename = "response.created")]
    ResponseCreated,
    #[serde(rename = "response.output_text.delta")]
    ResponseOutputTextDelta,
    #[serde(rename = "response.output_text.done")]
    ResponseOutputTextDone,
    #[serde(rename = "response.reasoning_summary_text.delta")]
    ResponseReasoningSummaryTextDelta,
    #[serde(rename = "response.reasoning_summary_text.done")]
    ResponseReasoningSummaryTextDone,
    #[serde(rename = "response.output_item.added")]
    ResponseOutputItemAdded,
    #[serde(rename = "response.output_item.done")]
    ResponseOutputItemDone,
    #[serde(rename = "response.function_call_arguments.delta")]
    ResponseFunctionCallArgumentsDelta,
    #[serde(rename = "response.function_call_arguments.done")]
    ResponseFunctionCallArgumentsDone,
    #[serde(rename = "response.completed")]
    ResponseCompleted,
    #[serde(rename = "response.incomplete")]
    ResponseIncomplete,
    #[serde(rename = "response.failed")]
    ResponseFailed,
    #[serde(rename = "error")]
    Error,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone)]
pub(crate) enum ResponsesStreamEvent {
    TextDelta {
        delta: String,
    },
    ReasoningDelta {
        delta: String,
    },
    FunctionCallDone {
        call_id: String,
        name: String,
        arguments: String,
    },
    Completed {
        usage: Option<TokenUsage>,
        finish_reason: FinishReason,
    },
    Failed {
        message: String,
    },
    Error {
        message: String,
    },
}

/// Parse a single Responses API SSE data payload into stream events.
///
/// `has_function_call` is the caller-tracked flag indicating whether any
/// `FunctionCallDone` event has been emitted in this stream so far. It is
/// used to resolve the finish reason when `response.completed` arrives.
pub(crate) fn parse_responses_chunk(
    data: &str,
    has_function_call: bool,
) -> Vec<ResponsesStreamEvent> {
    use ResponsesStreamEvent::{
        Completed, Error, Failed, FunctionCallDone, ReasoningDelta, TextDelta,
    };

    let value: Value = match serde_json::from_str(data) {
        Ok(v) => v,
        Err(_) => return vec![],
    };

    let chunk_type: ResponsesChunkType = match serde_json::from_value(
        value
            .get("type")
            .cloned()
            .unwrap_or(Value::String("__unknown__".to_string())),
    ) {
        Ok(t) => t,
        Err(_) => return vec![],
    };

    match chunk_type {
        ResponsesChunkType::ResponseOutputTextDelta => {
            if let Some(delta) = value.get("delta").and_then(|d| d.as_str()) {
                if !delta.is_empty() {
                    return vec![TextDelta {
                        delta: delta.to_string(),
                    }];
                }
            }
            vec![]
        }
        ResponsesChunkType::ResponseReasoningSummaryTextDelta => {
            if let Some(delta) = value.get("delta").and_then(|d| d.as_str()) {
                if !delta.is_empty() {
                    return vec![ReasoningDelta {
                        delta: delta.to_string(),
                    }];
                }
            }
            vec![]
        }
        ResponsesChunkType::ResponseOutputItemDone => {
            if let Some(item) = value.get("item") {
                if item.get("type").and_then(|t| t.as_str()) == Some("function_call") {
                    let call_id = item
                        .get("call_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let name = item
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let arguments = item
                        .get("arguments")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    if !call_id.is_empty() && !name.is_empty() {
                        return vec![FunctionCallDone {
                            call_id,
                            name,
                            arguments,
                        }];
                    }
                }
            }
            vec![]
        }
        ResponsesChunkType::ResponseCompleted | ResponsesChunkType::ResponseIncomplete => {
            let (usage, finish_reason) = if let Some(resp) = value.get("response") {
                let usage = resp.get("usage").map(|u| {
                    let input_tokens = u
                        .get("input_tokens")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0) as u32;
                    let output_tokens = u
                        .get("output_tokens")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0) as u32;
                    let cached = u
                        .get("input_tokens_details")
                        .and_then(|d| d.get("cached_tokens"))
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0) as u32;
                    TokenUsage {
                        input_tokens: input_tokens.saturating_sub(cached),
                        output_tokens,
                        cache_read_tokens: (cached > 0).then_some(cached),
                        cache_write_tokens: None,
                        ..Default::default()
                    }
                });
                let incomplete_reason = resp
                    .get("incomplete_details")
                    .and_then(|d| d.get("reason"))
                    .and_then(|r| r.as_str());
                let fr = map_responses_finish_reason(incomplete_reason, has_function_call);
                (usage, fr)
            } else {
                (None, FinishReason::Stop)
            };
            vec![Completed {
                usage,
                finish_reason,
            }]
        }
        ResponsesChunkType::ResponseFailed => {
            let message = value
                .get("response")
                .and_then(|r| r.get("error"))
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
                .unwrap_or("response failed")
                .to_string();
            vec![Failed { message }]
        }
        ResponsesChunkType::Error => {
            let message = value
                .get("message")
                .and_then(|m| m.as_str())
                .or_else(|| value.get("error").and_then(|e| e.as_str()))
                .unwrap_or("unknown error")
                .to_string();
            vec![Error { message }]
        }
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_msg(role: Role, content: &str) -> Message {
        Message {
            message_id: "m1".into(),
            role,
            content: content.into(),
            tool_call_id: None,
            tool_calls: vec![],
            timestamp: y_core::types::now(),
            metadata: serde_json::json!({}),
        }
    }

    #[test]
    fn build_input_system_user() {
        let msgs = vec![
            make_msg(Role::System, "You are helpful."),
            make_msg(Role::User, "Hello"),
        ];
        let input = build_responses_input(&msgs);
        assert_eq!(input.len(), 2);
        assert_eq!(input[0]["role"], "system");
        assert_eq!(input[0]["content"], "You are helpful.");
        assert_eq!(input[1]["role"], "user");
        assert_eq!(input[1]["content"][0]["type"], "input_text");
        assert_eq!(input[1]["content"][0]["text"], "Hello");
    }

    #[test]
    fn build_input_assistant_with_tool_calls() {
        let mut msg = make_msg(Role::Assistant, "I called a tool");
        msg.tool_calls.push(ToolCallRequest {
            id: "call_1".into(),
            name: "get_weather".into(),
            arguments: serde_json::json!({"city": "Paris"}),
        });
        let input = build_responses_input(&[msg]);
        assert_eq!(input.len(), 2);
        assert_eq!(input[0]["type"], "function_call");
        assert_eq!(input[0]["call_id"], "call_1");
        assert_eq!(input[0]["name"], "get_weather");
        assert_eq!(input[0]["arguments"], r#"{"city":"Paris"}"#);
        assert_eq!(input[1]["role"], "assistant");
    }

    #[test]
    fn build_input_tool_result() {
        let mut msg = make_msg(Role::Tool, r#"{"temp": 22}"#);
        msg.tool_call_id = Some("call_1".into());
        let input = build_responses_input(&[msg]);
        assert_eq!(input.len(), 1);
        assert_eq!(input[0]["type"], "function_call_output");
        assert_eq!(input[0]["call_id"], "call_1");
        assert_eq!(input[0]["output"], r#"{"temp": 22}"#);
    }

    #[test]
    fn build_input_user_with_image_attachment() {
        let mut msg = make_msg(Role::User, "What is this?");
        msg.metadata = serde_json::json!({
            "attachments": [{
                "mime_type": "image/png",
                "base64_data": "iVBORw0KGgo=",
            }]
        });
        let input = build_responses_input(&[msg]);
        assert_eq!(input.len(), 1);
        let content = input[0]["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        assert_eq!(content[0]["type"], "input_image");
        assert!(content[0]["image_url"]
            .as_str()
            .unwrap()
            .starts_with("data:image/png"));
        assert_eq!(content[1]["type"], "input_text");
        assert_eq!(content[1]["text"], "What is this?");
    }

    #[test]
    fn build_tools_converts_chat_completions_shape() {
        let tools = vec![json!({
            "type": "function",
            "function": {
                "name": "get_weather",
                "description": "Get weather",
                "parameters": {"type": "object", "properties": {}},
                "strict": true,
            }
        })];
        let result = build_responses_tools(&tools);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["type"], "function");
        assert_eq!(result[0]["name"], "get_weather");
        assert_eq!(result[0]["description"], "Get weather");
        assert!(result[0]["parameters"].is_object());
        assert_eq!(result[0]["strict"], true);
    }

    #[test]
    fn build_tools_passes_through_responses_shape() {
        let tools = vec![json!({
            "type": "function",
            "name": "get_weather",
            "parameters": {"type": "object"},
        })];
        let result = build_responses_tools(&tools);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["name"], "get_weather");
    }

    #[test]
    fn parse_response_text_message() {
        let raw = json!({
            "id": "resp_123",
            "model": "gpt-4o",
            "output": [{
                "type": "message",
                "id": "msg_1",
                "content": [{"type": "output_text", "text": "Hello!"}]
            }],
            "usage": {
                "input_tokens": 10,
                "output_tokens": 5,
                "input_tokens_details": {"cached_tokens": 2}
            }
        });
        let parsed = parse_responses_response(&raw).unwrap();
        assert_eq!(parsed.id, "resp_123");
        assert_eq!(parsed.model, "gpt-4o");
        assert_eq!(parsed.content.as_deref(), Some("Hello!"));
        assert!(parsed.tool_calls.is_empty());
        assert_eq!(parsed.usage.input_tokens, 8);
        assert_eq!(parsed.usage.cache_read_tokens, Some(2));
        assert_eq!(parsed.usage.output_tokens, 5);
        assert_eq!(parsed.finish_reason, FinishReason::Stop);
    }

    #[test]
    fn parse_response_with_function_call() {
        let raw = json!({
            "id": "resp_456",
            "model": "gpt-4o",
            "output": [{
                "type": "function_call",
                "id": "fc_1",
                "call_id": "call_abc",
                "name": "get_weather",
                "arguments": "{\"city\":\"Paris\"}"
            }],
            "usage": {"input_tokens": 10, "output_tokens": 5}
        });
        let parsed = parse_responses_response(&raw).unwrap();
        assert_eq!(parsed.tool_calls.len(), 1);
        assert_eq!(parsed.tool_calls[0].id, "call_abc");
        assert_eq!(parsed.tool_calls[0].name, "get_weather");
        assert_eq!(parsed.tool_calls[0].arguments, json!({"city": "Paris"}));
        assert_eq!(parsed.finish_reason, FinishReason::ToolUse);
    }

    #[test]
    fn parse_response_with_reasoning() {
        let raw = json!({
            "id": "resp_789",
            "model": "o3",
            "output": [
                {"type": "reasoning", "id": "rs_1", "summary": [{"type": "summary_text", "text": "Thinking..."}]},
                {"type": "message", "id": "msg_1", "content": [{"type": "output_text", "text": "Answer"}]}
            ],
            "usage": {"input_tokens": 100, "output_tokens": 50, "output_tokens_details": {"reasoning_tokens": 20}}
        });
        let parsed = parse_responses_response(&raw).unwrap();
        assert_eq!(parsed.reasoning_content.as_deref(), Some("Thinking..."));
        assert_eq!(parsed.content.as_deref(), Some("Answer"));
    }

    #[test]
    fn parse_response_incomplete_max_tokens() {
        let raw = json!({
            "id": "resp_000",
            "model": "gpt-4o",
            "output": [],
            "incomplete_details": {"reason": "max_output_tokens"},
            "usage": {"input_tokens": 10, "output_tokens": 100}
        });
        let parsed = parse_responses_response(&raw).unwrap();
        assert_eq!(parsed.finish_reason, FinishReason::Length);
    }

    #[test]
    fn parse_response_unknown_output_item_ignored() {
        let raw = json!({
            "id": "resp_001",
            "model": "gpt-4o",
            "output": [
                {"type": "web_search_call", "id": "ws_1", "status": "completed"},
                {"type": "message", "id": "msg_1", "content": [{"type": "output_text", "text": "Hi"}]}
            ],
            "usage": {"input_tokens": 10, "output_tokens": 5}
        });
        let parsed = parse_responses_response(&raw).unwrap();
        assert_eq!(parsed.content.as_deref(), Some("Hi"));
        assert!(parsed.tool_calls.is_empty());
    }

    #[test]
    fn parse_streaming_text_delta() {
        let events = parse_responses_chunk(
            r#"{"type":"response.output_text.delta","item_id":"msg_1","delta":"Hello"}"#,
            false,
        );
        assert_eq!(events.len(), 1);
        match &events[0] {
            ResponsesStreamEvent::TextDelta { delta } => assert_eq!(delta, "Hello"),
            _ => panic!("expected TextDelta"),
        }
    }

    #[test]
    fn parse_streaming_reasoning_delta() {
        let events = parse_responses_chunk(
            r#"{"type":"response.reasoning_summary_text.delta","item_id":"rs_1","summary_index":0,"delta":"Thinking"}"#,
            false,
        );
        assert_eq!(events.len(), 1);
        match &events[0] {
            ResponsesStreamEvent::ReasoningDelta { delta } => assert_eq!(delta, "Thinking"),
            _ => panic!("expected ReasoningDelta"),
        }
    }

    #[test]
    fn parse_streaming_function_call_done() {
        let events = parse_responses_chunk(
            r#"{"type":"response.output_item.done","output_index":0,"item":{"type":"function_call","id":"fc_1","call_id":"call_abc","name":"get_weather","arguments":"{\"city\":\"Paris\"}","status":"completed"}}"#,
            false,
        );
        assert_eq!(events.len(), 1);
        match &events[0] {
            ResponsesStreamEvent::FunctionCallDone {
                call_id,
                name,
                arguments,
            } => {
                assert_eq!(call_id, "call_abc");
                assert_eq!(name, "get_weather");
                assert_eq!(arguments, r#"{"city":"Paris"}"#);
            }
            _ => panic!("expected FunctionCallDone"),
        }
    }

    #[test]
    fn parse_streaming_completed_with_usage() {
        let data = r#"{"type":"response.completed","response":{"usage":{"input_tokens":10,"output_tokens":5,"input_tokens_details":{"cached_tokens":3}}}}"#;
        let events = parse_responses_chunk(data, false);
        assert_eq!(events.len(), 1);
        match &events[0] {
            ResponsesStreamEvent::Completed {
                usage,
                finish_reason,
            } => {
                let u = usage.as_ref().unwrap();
                assert_eq!(u.input_tokens, 7);
                assert_eq!(u.cache_read_tokens, Some(3));
                assert_eq!(u.output_tokens, 5);
                assert_eq!(*finish_reason, FinishReason::Stop);
            }
            _ => panic!("expected Completed"),
        }
    }

    #[test]
    fn parse_streaming_completed_with_tool_use() {
        let data = r#"{"type":"response.completed","response":{"usage":{"input_tokens":10,"output_tokens":5}}}"#;
        let events = parse_responses_chunk(data, true);
        assert_eq!(events.len(), 1);
        match &events[0] {
            ResponsesStreamEvent::Completed { finish_reason, .. } => {
                assert_eq!(*finish_reason, FinishReason::ToolUse);
            }
            _ => panic!("expected Completed"),
        }
    }

    #[test]
    fn parse_streaming_unknown_event_ignored() {
        let events = parse_responses_chunk(
            r#"{"type":"response.created","response":{"id":"resp_1","created_at":123,"model":"gpt-4o"}}"#,
            false,
        );
        assert!(events.is_empty());
    }

    #[test]
    fn parse_streaming_malformed_json_ignored() {
        let events = parse_responses_chunk("not json at all", false);
        assert!(events.is_empty());
    }

    #[test]
    fn parse_streaming_failed_event() {
        let data = r#"{"type":"response.failed","sequence_number":1,"response":{"error":{"code":"rate_limit_exceeded","message":"You exceeded your quota"}}}"#;
        let events = parse_responses_chunk(data, false);
        assert_eq!(events.len(), 1);
        match &events[0] {
            ResponsesStreamEvent::Failed { message } => {
                assert!(message.contains("quota"));
            }
            _ => panic!("expected Failed"),
        }
    }

    #[test]
    fn reasoning_max_downgrades_to_high() {
        let cfg = y_core::provider::ThinkingConfig {
            effort: ThinkingEffort::Max,
        };
        let r = build_responses_reasoning(Some(&cfg)).unwrap();
        assert_eq!(r.effort, "high");
    }

    #[test]
    fn text_format_json_schema() {
        let rf = ResponseFormat::JsonSchema {
            name: "my_schema".into(),
            schema: json!({"type": "object"}),
        };
        let fmt = build_responses_text_format(&rf).unwrap();
        match fmt {
            ResponsesTextFormat::JsonSchema { name, strict, .. } => {
                assert_eq!(name, "my_schema");
                assert!(strict);
            }
            _ => panic!("expected JsonSchema"),
        }
    }
}
