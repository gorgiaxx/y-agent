//! LLM provider traits and associated types.
//!
//! Design reference: providers-design.md
//!
//! The provider layer manages multiple LLM backends with tag-based routing,
//! intelligent freeze/failover, and per-provider connection pooling.

use std::pin::Pin;

use async_trait::async_trait;
use futures::Stream;
use serde::{Deserialize, Serialize};

use crate::error::ErrorSeverity;
use crate::types::{Message, ProviderId, Timestamp, TokenUsage};

// ---------------------------------------------------------------------------
// Tool calling mode
// ---------------------------------------------------------------------------

/// How tool calling is communicated to the LLM.
///
/// Two-layer design (see `docs/standards/TOOL_CALL_PROTOCOL.md`):
///
/// - **Layer 1 (API type)**: First-party providers (`OpenAI`, Anthropic, Azure,
///   Gemini, `DeepSeek`) default to [`Native`](Self::Native) -- tool definitions
///   are sent via the provider's HTTP API and tool calls are extracted from
///   structured response fields.
///
/// - **Layer 2 (XML tags)**: Compatibility providers (`openai-compat`, `custom`,
///   `ollama`) default to [`PromptBased`](Self::PromptBased) -- tool definitions
///   are injected into the system prompt and the lenient XML parser extracts
///   tool calls from the model's text output.
///
/// Per-provider override is available via `tool_calling_mode` in the provider
/// TOML configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallingMode {
    /// Tool calling via system prompt text protocol (universal, works with any LLM).
    ///
    /// The LLM is taught to emit `<tool_call>` XML tags in its text output.
    /// The `tools` field in [`ChatRequest`] is left empty; providers do not send
    /// tool definitions in the HTTP request body.
    ///
    /// Default for compatibility providers (`openai-compat`, `custom`, `ollama`).
    PromptBased,
    /// Tool calling via provider-native API fields (`OpenAI` `tools`, Anthropic `tools`).
    ///
    /// Tool definitions are sent in the HTTP request body and tool calls are
    /// extracted from provider-specific response fields.
    ///
    /// Default for first-party providers (`openai`, `anthropic`, `azure`,
    /// `gemini`, `deepseek`).
    #[default]
    Native,
}

// ---------------------------------------------------------------------------
// Thinking / Reasoning
// ---------------------------------------------------------------------------

/// Unified thinking/reasoning effort level.
///
/// Maps to provider-specific parameters:
/// - Anthropic: `output_config.effort` (`"low"` | `"medium"` | `"high"` | `"max"`)
/// - `OpenAI`: `reasoning.effort` (`"low"` | `"medium"` | `"high"`)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingEffort {
    Low,
    Medium,
    High,
    Max,
}

/// Thinking/reasoning configuration for an LLM request.
///
/// When `None` on [`ChatRequest`], the provider uses model defaults.
/// When `Some`, the provider translates to its native API format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThinkingConfig {
    pub effort: ThinkingEffort,
}

// ---------------------------------------------------------------------------
// Request / Response
// ---------------------------------------------------------------------------

/// Request to an LLM provider.
#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub messages: Vec<Message>,
    pub model: Option<String>,
    /// Maximum tokens to generate.
    pub max_tokens: Option<u32>,
    /// Sampling temperature (0.0 - 2.0).
    pub temperature: Option<f64>,
    /// Nucleus sampling top-p (0.0 - 1.0).
    pub top_p: Option<f64>,
    /// Tool definitions available for this request (only used in [`ToolCallingMode::Native`]).
    pub tools: Vec<serde_json::Value>,
    /// How tool calling is communicated to the LLM.
    pub tool_calling_mode: ToolCallingMode,
    /// Stop sequences.
    pub stop: Vec<String>,
    /// Arbitrary provider-specific parameters.
    pub extra: serde_json::Value,
    /// Thinking/reasoning configuration (`None` = use model defaults).
    pub thinking: Option<ThinkingConfig>,
}

/// Response from an LLM provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatResponse {
    /// Provider-assigned response ID.
    pub id: String,
    /// Model that generated the response.
    pub model: String,
    /// Generated content (may be empty if tool calls are present).
    pub content: Option<String>,
    /// Reasoning/thinking content from thinking-mode LLMs (e.g. DeepSeek-R1, `QwQ`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    /// Tool calls requested by the model.
    #[serde(default)]
    pub tool_calls: Vec<crate::types::ToolCallRequest>,
    /// Token usage.
    pub usage: TokenUsage,
    /// Why the model stopped generating.
    pub finish_reason: FinishReason,
    /// Raw HTTP request payload sent to the LLM provider (for diagnostics).
    #[serde(skip)]
    pub raw_request: Option<serde_json::Value>,
    /// Raw HTTP response payload received from the LLM provider (for diagnostics).
    #[serde(skip)]
    pub raw_response: Option<serde_json::Value>,
    /// Provider that served this response (set by the pool after routing).
    #[serde(default)]
    pub provider_id: Option<ProviderId>,
}

/// A single chunk in a streaming response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatStreamChunk {
    /// Incremental content delta.
    pub delta_content: Option<String>,
    /// Incremental reasoning/thinking content delta.
    pub delta_reasoning_content: Option<String>,
    /// Incremental tool call delta.
    pub delta_tool_calls: Vec<crate::types::ToolCallRequest>,
    /// Present only in the final chunk.
    pub usage: Option<TokenUsage>,
    /// Present only in the final chunk.
    pub finish_reason: Option<FinishReason>,
}

/// Reason the model stopped generating.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    Stop,
    Length,
    ToolUse,
    ContentFilter,
    Unknown,
}

/// Streaming response type: a pinned boxed stream of chunk results.
pub type ChatStream =
    Pin<Box<dyn Stream<Item = Result<ChatStreamChunk, ProviderError>> + Send + 'static>>;

/// Wrapper returned by streaming methods, bundling the chunk stream with the
/// raw HTTP request body that was sent to the provider (for diagnostics).
pub struct ChatStreamResponse {
    /// The SSE/streaming chunk stream.
    pub stream: ChatStream,
    /// Raw HTTP request body serialized by the provider (for diagnostics).
    pub raw_request: Option<serde_json::Value>,
    /// Provider that served this streaming request (set by the pool after routing).
    pub provider_id: Option<ProviderId>,
    /// Model name from the serving provider's metadata.
    pub model: String,
    /// Context window size of the serving provider (tokens).
    pub context_window: usize,
}

// ---------------------------------------------------------------------------
// Provider metadata
// ---------------------------------------------------------------------------

/// Static metadata describing a configured provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderMetadata {
    pub id: ProviderId,
    pub provider_type: ProviderType,
    pub model: String,
    pub tags: Vec<String>,
    pub max_concurrency: usize,
    pub context_window: usize,
    pub cost_per_1k_input: f64,
    pub cost_per_1k_output: f64,
    /// Effective tool calling mode for this provider (resolved from config).
    pub tool_calling_mode: ToolCallingMode,
}

/// Supported provider backend types.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderType {
    OpenAi,
    Anthropic,
    Gemini,
    Ollama,
    Azure,
    OpenRouter,
    Custom,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors from LLM provider operations.
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("rate limited by {provider}: retry after {retry_after_secs}s")]
    RateLimited {
        provider: String,
        retry_after_secs: u64,
    },

    #[error("quota exhausted for {provider}: {message}")]
    QuotaExhausted { provider: String, message: String },

    #[error("authentication failed for {provider}: {message}")]
    AuthenticationFailed { provider: String, message: String },

    #[error("invalid API key for {provider}: {message}")]
    KeyInvalid { provider: String, message: String },

    #[error("server error from {provider}: {message}")]
    ServerError { provider: String, message: String },

    #[error("network error: {message}")]
    NetworkError { message: String },

    #[error("no provider available matching tags {tags:?}")]
    NoProviderAvailable { tags: Vec<String> },

    #[error("request cancelled")]
    Cancelled,

    #[error("response parse error: {message}")]
    ParseError { message: String },

    #[error("{message}")]
    Other { message: String },
}

impl ProviderError {
    /// Classify error severity for freeze duration decisions.
    pub fn severity(&self) -> ErrorSeverity {
        match self {
            Self::AuthenticationFailed { .. }
            | Self::KeyInvalid { .. }
            | Self::QuotaExhausted { .. } => ErrorSeverity::Permanent,
            Self::NoProviderAvailable { .. } => ErrorSeverity::UserActionRequired,
            // All other errors are transient (rate limits, network, server, parse, etc.)
            _ => ErrorSeverity::Transient,
        }
    }
}

// ---------------------------------------------------------------------------
// Traits
// ---------------------------------------------------------------------------

/// A single LLM provider backend.
///
/// Implementations handle HTTP communication with a specific provider API
/// (`OpenAI`, Anthropic, Ollama, etc.). They do not handle routing or failover;
/// that is the responsibility of [`ProviderPool`].
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Send a chat completion request and wait for the full response.
    async fn chat_completion(&self, request: &ChatRequest) -> Result<ChatResponse, ProviderError>;

    /// Send a chat completion request and return a streaming response.
    async fn chat_completion_stream(
        &self,
        request: &ChatRequest,
    ) -> Result<ChatStreamResponse, ProviderError>;

    /// Return static metadata for this provider.
    fn metadata(&self) -> &ProviderMetadata;
}

/// Routing request specifying how to select a provider.
#[derive(Debug, Clone, Default)]
pub struct RouteRequest {
    /// Required tags the provider must have.
    pub required_tags: Vec<String>,
    /// Preferred provider by ID (exact match, highest priority).
    pub preferred_provider_id: Option<ProviderId>,
    /// Preferred model (exact match, optional).
    pub preferred_model: Option<String>,
    /// Priority level.
    pub priority: RoutePriority,
}

/// Request priority for provider selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RoutePriority {
    /// Reserve 20% of concurrency budget for critical requests.
    Critical,
    #[default]
    Normal,
    /// Lowest priority, may be queued.
    Idle,
}

/// Provider health and freeze status.
#[derive(Debug, Clone)]
pub struct ProviderStatus {
    pub id: ProviderId,
    pub is_frozen: bool,
    pub frozen_since: Option<Timestamp>,
    pub thaw_at: Option<Timestamp>,
    pub freeze_reason: Option<String>,
    pub active_requests: usize,
    pub total_requests: u64,
    pub total_errors: u64,
}

/// Manages a pool of LLM providers with routing, freeze/thaw, and failover.
///
/// The pool selects providers based on tags, availability, and priority.
/// Failed providers are frozen with adaptive durations and thawed after
/// health check verification.
#[async_trait]
pub trait ProviderPool: Send + Sync {
    /// Send a request, routing to the best available provider.
    async fn chat_completion(
        &self,
        request: &ChatRequest,
        route: &RouteRequest,
    ) -> Result<ChatResponse, ProviderError>;

    /// Send a streaming request, routing to the best available provider.
    async fn chat_completion_stream(
        &self,
        request: &ChatRequest,
        route: &RouteRequest,
    ) -> Result<ChatStreamResponse, ProviderError>;

    /// Report an error from a specific provider (triggers freeze evaluation).
    fn report_error(&self, provider_id: &ProviderId, error: &ProviderError);

    /// Get the status of all providers.
    async fn provider_statuses(&self) -> Vec<ProviderStatus>;

    /// Manually freeze a provider.
    async fn freeze(&self, provider_id: &ProviderId, reason: String);

    /// Manually thaw a frozen provider (triggers health check first).
    async fn thaw(&self, provider_id: &ProviderId) -> Result<(), ProviderError>;
}
