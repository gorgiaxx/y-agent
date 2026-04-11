//! Gateway 1: Diagnostics-aware provider pool wrapper.
//!
//! Wraps `ProviderPool` to automatically record LLM call observations
//! and emit real-time `DiagnosticsEvent`s without any manual wiring
//! in business logic.

use std::sync::Arc;

use async_trait::async_trait;
use uuid::Uuid;

use y_core::provider::{
    ChatRequest, ChatResponse, ChatStreamResponse, ProviderError, ProviderPool, ProviderStatus,
    RouteRequest,
};
use y_diagnostics::{
    DiagnosticsContext, DiagnosticsEvent, DiagnosticsSubscriber, GenerationParams, TraceStore,
    DIAGNOSTICS_CTX,
};

/// A thin wrapper around `ProviderPool` that intercepts every LLM call
/// and automatically records a generation observation + broadcast event.
///
/// Reads [`DIAGNOSTICS_CTX`] from the task-local to obtain trace identity.
/// If no context is set (e.g. the call is outside an agent execution),
/// the call passes through without recording.
pub struct DiagnosticsProviderPool {
    inner: Arc<dyn ProviderPool>,
    diagnostics: Arc<DiagnosticsSubscriber<dyn TraceStore>>,
    broadcast_tx: tokio::sync::broadcast::Sender<DiagnosticsEvent>,
}

impl DiagnosticsProviderPool {
    pub fn new(
        inner: Arc<dyn ProviderPool>,
        diagnostics: Arc<DiagnosticsSubscriber<dyn TraceStore>>,
        broadcast_tx: tokio::sync::broadcast::Sender<DiagnosticsEvent>,
    ) -> Self {
        Self {
            inner,
            diagnostics,
            broadcast_tx,
        }
    }

    /// Record a completed (non-streaming) generation.
    async fn record_generation(
        &self,
        ctx: &DiagnosticsContext,
        response: &ChatResponse,
        elapsed_ms: u64,
        fallback_input: &str,
    ) {
        let iteration = ctx.next_iteration();

        let diag_input = response.raw_request.clone().unwrap_or_else(|| {
            serde_json::from_str(fallback_input).unwrap_or(serde_json::Value::Null)
        });
        let diag_output = response.raw_response.clone().unwrap_or_else(|| {
            let mut output = serde_json::json!({
                "content": response.content.clone().unwrap_or_default(),
                "model": response.model,
                "usage": {
                    "input_tokens": response.usage.input_tokens,
                    "output_tokens": response.usage.output_tokens,
                }
            });
            if let Some(reasoning) = response.reasoning_content.as_ref() {
                output["reasoning_content"] = serde_json::Value::String(reasoning.clone());
            }
            output
        });

        let cost = crate::cost::CostService::compute_cost(
            u64::from(response.usage.input_tokens),
            u64::from(response.usage.output_tokens),
        );

        let gen_id = self
            .diagnostics
            .on_generation(GenerationParams {
                trace_id: ctx.trace_id,
                parent_id: None,
                session_id: ctx.session_id,
                model: response.model.clone(),
                input_tokens: u64::from(response.usage.input_tokens),
                output_tokens: u64::from(response.usage.output_tokens),
                cost_usd: cost,
                input: diag_input,
                output: diag_output,
                duration_ms: elapsed_ms,
            })
            .await
            .ok();

        // Update the last generation ID for parent chaining on tool calls.
        *ctx.last_gen_id.lock().await = gen_id;

        let tool_calls_requested: Vec<String> = response
            .tool_calls
            .iter()
            .map(|tc| tc.name.clone())
            .collect();

        let prompt_preview = response.raw_request.as_ref().map_or_else(
            || fallback_input.to_string(),
            std::string::ToString::to_string,
        );
        let response_text = response.raw_response.as_ref().map_or_else(
            || {
                let mut output = serde_json::json!({
                    "content": response.content.clone().unwrap_or_default(),
                    "model": response.model,
                    "usage": {
                        "input_tokens": response.usage.input_tokens,
                        "output_tokens": response.usage.output_tokens,
                    }
                });
                if let Some(reasoning) = response.reasoning_content.as_ref() {
                    output["reasoning_content"] = serde_json::Value::String(reasoning.clone());
                }
                output.to_string()
            },
            std::string::ToString::to_string,
        );

        // Emit real-time event.
        let _ = self.broadcast_tx.send(DiagnosticsEvent::LlmCallCompleted {
            trace_id: ctx.trace_id,
            observation_id: gen_id.unwrap_or(Uuid::nil()),
            session_id: ctx.session_id,
            agent_name: ctx.agent_name.clone(),
            iteration,
            model: response.model.clone(),
            input_tokens: u64::from(response.usage.input_tokens),
            output_tokens: u64::from(response.usage.output_tokens),
            duration_ms: elapsed_ms,
            cost_usd: cost,
            tool_calls_requested,
            prompt_preview,
            response_text,
            context_window: 0,
        });

        tracing::debug!(
            trace_id = %ctx.trace_id,
            agent = %ctx.agent_name,
            model = %response.model,
            input_tokens = response.usage.input_tokens,
            output_tokens = response.usage.output_tokens,
            llm_ms = elapsed_ms,
            "DiagnosticsProviderPool: generation recorded"
        );
    }

    /// Emit an `LlmCallFailed` broadcast event.
    fn emit_failure(
        &self,
        ctx: &DiagnosticsContext,
        error: &ProviderError,
        elapsed_ms: u64,
        model: &str,
    ) {
        let iteration = ctx.iteration.load(std::sync::atomic::Ordering::Relaxed) + 1;
        let _ = self.broadcast_tx.send(DiagnosticsEvent::LlmCallFailed {
            trace_id: ctx.trace_id,
            observation_id: None,
            session_id: ctx.session_id,
            agent_name: ctx.agent_name.clone(),
            iteration,
            model: model.to_string(),
            error: format!("{error}"),
            duration_ms: elapsed_ms,
        });
    }
}

#[async_trait]
impl ProviderPool for DiagnosticsProviderPool {
    async fn chat_completion(
        &self,
        request: &ChatRequest,
        route: &RouteRequest,
    ) -> Result<ChatResponse, ProviderError> {
        let start = std::time::Instant::now();
        let fallback = serde_json::to_string(&request.messages).unwrap_or_default();

        let result = self.inner.chat_completion(request, route).await;
        let elapsed_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(0);

        // Only record if DIAGNOSTICS_CTX is set.
        if let Ok(ctx) = DIAGNOSTICS_CTX.try_with(Clone::clone) {
            match &result {
                Ok(response) => {
                    self.record_generation(&ctx, response, elapsed_ms, &fallback)
                        .await;
                }
                Err(e) => {
                    let model = request.model.as_deref().unwrap_or("unknown");
                    self.emit_failure(&ctx, e, elapsed_ms, model);
                }
            }
        }

        result
    }

    async fn chat_completion_stream(
        &self,
        request: &ChatRequest,
        route: &RouteRequest,
    ) -> Result<ChatStreamResponse, ProviderError> {
        // For streaming, we pass through directly. The diagnostics recording
        // happens when the stream is consumed and the full response is
        // reconstructed by the caller (agent_service::call_llm_streaming
        // accumulates chunks into a ChatResponse).
        //
        // The caller is responsible for calling record_generation() on the
        // final assembled ChatResponse. This is because the gateway cannot
        // know the final token counts, cost, or content until the stream
        // is fully consumed.
        self.inner.chat_completion_stream(request, route).await
    }

    fn report_error(&self, provider_id: &y_core::types::ProviderId, error: &ProviderError) {
        self.inner.report_error(provider_id, error);
    }

    async fn provider_statuses(&self) -> Vec<ProviderStatus> {
        self.inner.provider_statuses().await
    }

    async fn freeze(&self, provider_id: &y_core::types::ProviderId, reason: String) {
        self.inner.freeze(provider_id, reason).await;
    }

    async fn thaw(&self, provider_id: &y_core::types::ProviderId) -> Result<(), ProviderError> {
        self.inner.thaw(provider_id).await
    }
}
