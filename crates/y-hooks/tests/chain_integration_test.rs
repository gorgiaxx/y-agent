//! Integration tests for middleware chains.
//!
//! Tests verify end-to-end chain behavior including multi-middleware
//! pipelines, guardrail-style short-circuits, and abort propagation.

use std::sync::Arc;

use async_trait::async_trait;
use y_core::hook::{ChainType, Middleware, MiddlewareContext, MiddlewareError, MiddlewareResult};
use y_hooks::chain::MiddlewareChain;

/// Middleware that appends its name to a JSON array in metadata.
struct RecordingMiddleware {
    name: String,
    priority: u32,
    chain: ChainType,
}

#[async_trait]
impl Middleware for RecordingMiddleware {
    async fn execute(
        &self,
        ctx: &mut MiddlewareContext,
    ) -> Result<MiddlewareResult, MiddlewareError> {
        if let Some(arr) = ctx.metadata.as_array_mut() {
            arr.push(serde_json::json!(self.name));
        }
        Ok(MiddlewareResult::Continue)
    }
    fn chain_type(&self) -> ChainType {
        self.chain
    }
    fn priority(&self) -> u32 {
        self.priority
    }
    fn name(&self) -> &str {
        &self.name
    }
}

/// Middleware that short-circuits when a condition is met.
struct GuardrailMiddleware {
    name: String,
    priority: u32,
    chain: ChainType,
    block_tool: String,
}

#[async_trait]
impl Middleware for GuardrailMiddleware {
    async fn execute(
        &self,
        ctx: &mut MiddlewareContext,
    ) -> Result<MiddlewareResult, MiddlewareError> {
        let tool_name = ctx.payload["tool_name"].as_str().unwrap_or("");
        if tool_name == self.block_tool {
            ctx.abort(format!("blocked by guardrail: {}", self.name));
            return Ok(MiddlewareResult::ShortCircuit);
        }
        Ok(MiddlewareResult::Continue)
    }
    fn chain_type(&self) -> ChainType {
        self.chain
    }
    fn priority(&self) -> u32 {
        self.priority
    }
    fn name(&self) -> &str {
        &self.name
    }
}

fn recording_mw(name: &str, priority: u32, chain: ChainType) -> Arc<dyn Middleware> {
    Arc::new(RecordingMiddleware {
        name: name.to_string(),
        priority,
        chain,
    })
}

// T-HOOK-INT-01: Full context chain pipeline.
#[tokio::test]
async fn test_context_chain_full_pipeline() {
    let mut chain = MiddlewareChain::new(ChainType::Context);
    chain
        .register(recording_mw("BuildSystemPrompt", 100, ChainType::Context))
        .unwrap();
    chain
        .register(recording_mw("InjectMemory", 300, ChainType::Context))
        .unwrap();
    chain
        .register(recording_mw("InjectTools", 500, ChainType::Context))
        .unwrap();
    chain
        .register(recording_mw("LoadHistory", 600, ChainType::Context))
        .unwrap();

    let mut ctx = MiddlewareContext {
        chain_type: ChainType::Context,
        payload: serde_json::json!({"messages": []}),
        metadata: serde_json::json!([]),
        aborted: false,
        abort_reason: None,
    };
    chain.execute(&mut ctx).await.unwrap();

    let order: Vec<String> = serde_json::from_value(ctx.metadata).unwrap();
    assert_eq!(
        order,
        vec![
            "BuildSystemPrompt",
            "InjectMemory",
            "InjectTools",
            "LoadHistory"
        ]
    );
    assert!(!ctx.aborted);
}

// T-HOOK-INT-02: Tool chain with guardrail short-circuit.
#[tokio::test]
async fn test_tool_chain_with_guardrail() {
    let mut chain = MiddlewareChain::new(ChainType::Tool);
    chain
        .register(recording_mw("validation", 100, ChainType::Tool))
        .unwrap();

    let guardrail: Arc<dyn Middleware> = Arc::new(GuardrailMiddleware {
        name: "dangerous-tool-guard".to_string(),
        priority: 200,
        chain: ChainType::Tool,
        block_tool: "rm_rf".to_string(),
    });
    chain.register(guardrail).unwrap();
    chain
        .register(recording_mw("execution", 300, ChainType::Tool))
        .unwrap();

    // Secure tool — goes through.
    let mut ctx = MiddlewareContext {
        chain_type: ChainType::Tool,
        payload: serde_json::json!({"tool_name": "search"}),
        metadata: serde_json::json!([]),
        aborted: false,
        abort_reason: None,
    };
    chain.execute(&mut ctx).await.unwrap();
    let order: Vec<String> = serde_json::from_value(ctx.metadata).unwrap();
    assert_eq!(order, vec!["validation", "execution"]);
    assert!(!ctx.aborted);

    // Dangerous tool — blocked by guardrail.
    let mut ctx = MiddlewareContext {
        chain_type: ChainType::Tool,
        payload: serde_json::json!({"tool_name": "rm_rf"}),
        metadata: serde_json::json!([]),
        aborted: false,
        abort_reason: None,
    };
    chain.execute(&mut ctx).await.unwrap();
    // Should be aborted, execution middleware skipped.
    assert!(ctx.aborted);
    assert!(ctx
        .abort_reason
        .as_ref()
        .unwrap()
        .contains("dangerous-tool-guard"));
}

// T-HOOK-INT-03: Abort propagation.
#[tokio::test]
async fn test_chain_abort_propagation() {
    let mut chain = MiddlewareChain::new(ChainType::Tool);
    chain
        .register(recording_mw("step-1", 100, ChainType::Tool))
        .unwrap();

    let guardrail: Arc<dyn Middleware> = Arc::new(GuardrailMiddleware {
        name: "security-check".to_string(),
        priority: 200,
        chain: ChainType::Tool,
        block_tool: "insecure_op".to_string(),
    });
    chain.register(guardrail).unwrap();
    chain
        .register(recording_mw("step-3", 300, ChainType::Tool))
        .unwrap();

    let mut ctx = MiddlewareContext {
        chain_type: ChainType::Tool,
        payload: serde_json::json!({"tool_name": "insecure_op"}),
        metadata: serde_json::json!([]),
        aborted: false,
        abort_reason: None,
    };
    chain.execute(&mut ctx).await.unwrap();

    assert!(ctx.aborted);
    assert_eq!(
        ctx.abort_reason.as_deref(),
        Some("blocked by guardrail: security-check")
    );
    // step-1 executed (before guardrail), step-3 skipped (after guardrail).
    let order: Vec<String> = serde_json::from_value(ctx.metadata).unwrap();
    assert_eq!(order, vec!["step-1"]);
}
