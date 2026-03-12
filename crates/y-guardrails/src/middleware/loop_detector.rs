//! `LoopDetectorMiddleware`: loop pattern detection in the agent loop.
//!
//! Wraps the `LoopGuard` and integrates it as middleware in the Tool chain.
//! Each tool execution is recorded, and if a loop pattern is detected,
//! the chain is aborted.

use std::sync::Mutex;

use async_trait::async_trait;
use y_core::hook::{ChainType, Middleware, MiddlewareContext, MiddlewareError, MiddlewareResult};

use crate::config::LoopGuardConfig;
use crate::loop_guard::{ActionRecord, LoopGuard};

/// Middleware that detects loop patterns in tool execution.
///
/// Registered as a `ToolMiddleware` at priority 20 (after `ToolGuard` at 10).
pub struct LoopDetectorMiddleware {
    guard: Mutex<LoopGuard>,
}

impl LoopDetectorMiddleware {
    /// Create a new loop detector middleware.
    pub fn new(config: LoopGuardConfig) -> Self {
        Self {
            guard: Mutex::new(LoopGuard::new(config)),
        }
    }
}

impl std::fmt::Debug for LoopDetectorMiddleware {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoopDetectorMiddleware").finish()
    }
}

#[async_trait]
impl Middleware for LoopDetectorMiddleware {
    async fn execute(
        &self,
        ctx: &mut MiddlewareContext,
    ) -> Result<MiddlewareResult, MiddlewareError> {
        let tool_name = ctx
            .payload
            .get("tool_name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown")
            .to_string();

        let args_hash = ctx
            .payload
            .get("args_hash")
            .and_then(serde_json::Value::as_str)
            .map(String::from);

        let progress = ctx
            .payload
            .get("progress_metric")
            .and_then(serde_json::Value::as_f64);

        let action = ActionRecord {
            action_key: tool_name.clone(),
            args_hash,
        };

        let detection = {
            let mut guard = self.guard.lock().map_err(|e| MiddlewareError::Other {
                message: format!("LoopGuard lock poisoned: {e}"),
            })?;
            guard.record(action, progress)
        };

        if let Some(detection) = detection {
            ctx.abort(format!(
                "Loop detected ({}): {}",
                detection.pattern, detection.details
            ));
            if let Some(meta) = ctx.metadata.as_object_mut() {
                meta.insert(
                    "loop_pattern".to_string(),
                    serde_json::Value::String(detection.pattern.to_string()),
                );
            }
            return Ok(MiddlewareResult::ShortCircuit);
        }

        Ok(MiddlewareResult::Continue)
    }

    fn chain_type(&self) -> ChainType {
        ChainType::Tool
    }

    fn priority(&self) -> u32 {
        20 // After ToolGuard (10), before tool execution
    }

    fn name(&self) -> &'static str {
        "LoopDetectorMiddleware"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use y_core::hook::MiddlewareContext;

    fn make_context(tool_name: &str, args_hash: Option<&str>) -> MiddlewareContext {
        let mut payload = serde_json::json!({ "tool_name": tool_name });
        if let Some(hash) = args_hash {
            payload["args_hash"] = serde_json::Value::String(hash.to_string());
        }
        MiddlewareContext::new(ChainType::Tool, payload)
    }

    #[tokio::test]
    async fn test_loop_detector_no_loop() {
        let mw = LoopDetectorMiddleware::new(LoopGuardConfig::default());

        for i in 0..3 {
            let mut ctx = make_context(&format!("tool_{i}"), None);
            let result = mw.execute(&mut ctx).await.unwrap();
            assert!(matches!(result, MiddlewareResult::Continue));
            assert!(!ctx.aborted);
        }
    }

    #[tokio::test]
    async fn test_loop_detector_detects_redundant() {
        let config = LoopGuardConfig {
            redundant_threshold: 3,
            ..Default::default()
        };
        let mw = LoopDetectorMiddleware::new(config);

        let mut aborted = false;
        for _ in 0..3 {
            let mut ctx = make_context("read_file", Some("same_hash"));
            let result = mw.execute(&mut ctx).await.unwrap();
            if ctx.aborted {
                aborted = true;
                assert!(matches!(result, MiddlewareResult::ShortCircuit));
                break;
            }
        }
        assert!(aborted, "should detect redundant tool calls");
    }
}
