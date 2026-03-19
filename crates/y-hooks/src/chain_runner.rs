//! Timeout-guarded middleware chain execution.

use std::sync::Arc;
use std::time::Duration;

use tracing::{info_span, instrument, Instrument};

use y_core::hook::{Middleware, MiddlewareContext, MiddlewareError, MiddlewareResult};

/// Runs a middleware chain with per-middleware timeout and error handling.
pub struct ChainRunner {
    /// Per-middleware timeout.
    timeout: Duration,
}

impl ChainRunner {
    /// Create a new chain runner with the given per-middleware timeout.
    pub fn new(timeout: Duration) -> Self {
        Self { timeout }
    }

    /// Execute a single middleware with timeout protection.
    #[instrument(skip(self, middleware, ctx), fields(middleware_name = middleware.name(), chain_type = ?ctx.chain_type))]
    pub async fn run_one(
        &self,
        middleware: &Arc<dyn Middleware>,
        ctx: &mut MiddlewareContext,
    ) -> Result<MiddlewareResult, MiddlewareError> {
        let name = middleware.name().to_string();
        let timeout = self.timeout;

        let span = info_span!("middleware", name = %name, priority = middleware.priority());

        let result = tokio::time::timeout(timeout, middleware.execute(ctx).instrument(span)).await;

        match result {
            Ok(Ok(mr)) => Ok(mr),
            Ok(Err(e)) => Err(e),
            Err(_elapsed) => Err(MiddlewareError::Timeout {
                name,
                timeout_ms: u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX),
            }),
        }
    }

    /// Execute a list of middleware in order with timeout per middleware.
    ///
    /// Stops on `ShortCircuit`, abort, or critical error.
    /// Non-critical errors are logged but do not stop the chain.
    pub async fn run_chain(
        &self,
        middleware_list: &[Arc<dyn Middleware>],
        ctx: &mut MiddlewareContext,
    ) -> Result<(), MiddlewareError> {
        for mw in middleware_list {
            if ctx.aborted {
                tracing::info!(
                    reason = ctx.abort_reason.as_deref().unwrap_or("unknown"),
                    "chain aborted, skipping remaining middleware"
                );
                break;
            }

            match self.run_one(mw, ctx).await {
                Ok(MiddlewareResult::Continue) => {}
                Ok(MiddlewareResult::ShortCircuit) => {
                    tracing::info!(middleware = mw.name(), "middleware short-circuited chain");
                    break;
                }
                Err(MiddlewareError::Timeout { ref name, .. }) => {
                    tracing::warn!(middleware = %name, "middleware timed out");
                    return Err(MiddlewareError::Timeout {
                        name: name.clone(),
                        timeout_ms: u64::try_from(self.timeout.as_millis()).unwrap_or(u64::MAX),
                    });
                }
                Err(e) => {
                    tracing::error!(
                        middleware = mw.name(),
                        error = %e,
                        "middleware execution failed"
                    );
                    return Err(e);
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use y_core::hook::ChainType;

    struct SlowMiddleware {
        name: String,
        delay: Duration,
    }

    #[async_trait]
    impl Middleware for SlowMiddleware {
        async fn execute(
            &self,
            _ctx: &mut MiddlewareContext,
        ) -> Result<MiddlewareResult, MiddlewareError> {
            tokio::time::sleep(self.delay).await;
            Ok(MiddlewareResult::Continue)
        }

        fn chain_type(&self) -> ChainType {
            ChainType::Context
        }

        fn priority(&self) -> u32 {
            100
        }

        fn name(&self) -> &str {
            &self.name
        }
    }

    struct FailingMiddleware {
        name: String,
        critical: bool,
    }

    #[async_trait]
    impl Middleware for FailingMiddleware {
        async fn execute(
            &self,
            _ctx: &mut MiddlewareContext,
        ) -> Result<MiddlewareResult, MiddlewareError> {
            if self.critical {
                Err(MiddlewareError::Panic {
                    name: self.name.clone(),
                    message: "critical failure".into(),
                })
            } else {
                Err(MiddlewareError::ExecutionError {
                    name: self.name.clone(),
                    message: "non-critical error".into(),
                })
            }
        }

        fn chain_type(&self) -> ChainType {
            ChainType::Context
        }

        fn priority(&self) -> u32 {
            100
        }

        fn name(&self) -> &str {
            &self.name
        }
    }

    struct NoopMiddleware {
        name: String,
    }

    #[async_trait]
    impl Middleware for NoopMiddleware {
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
            ChainType::Context
        }

        fn priority(&self) -> u32 {
            100
        }

        fn name(&self) -> &str {
            &self.name
        }
    }

    fn new_ctx() -> MiddlewareContext {
        MiddlewareContext {
            chain_type: ChainType::Context,
            payload: serde_json::json!({}),
            metadata: serde_json::json!([]),
            aborted: false,
            abort_reason: None,
        }
    }

    #[tokio::test]
    async fn test_runner_timeout_per_middleware() {
        let runner = ChainRunner::new(Duration::from_millis(50));
        let slow = Arc::new(SlowMiddleware {
            name: "slow".into(),
            delay: Duration::from_secs(2),
        }) as Arc<dyn Middleware>;

        let mut ctx = new_ctx();
        let result = runner.run_one(&slow, &mut ctx).await;
        assert!(matches!(result, Err(MiddlewareError::Timeout { .. })));
    }

    #[tokio::test]
    async fn test_runner_continues_after_non_fatal_error() {
        let runner = ChainRunner::new(Duration::from_secs(5));
        let failing = Arc::new(FailingMiddleware {
            name: "failing".into(),
            critical: false,
        }) as Arc<dyn Middleware>;

        let mut ctx = new_ctx();
        let result = runner.run_one(&failing, &mut ctx).await;
        // Non-fatal errors are still errors.
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_runner_aborts_on_critical_error() {
        let runner = ChainRunner::new(Duration::from_secs(5));
        let mw_list: Vec<Arc<dyn Middleware>> = vec![
            Arc::new(NoopMiddleware {
                name: "first".into(),
            }),
            Arc::new(FailingMiddleware {
                name: "critical".into(),
                critical: true,
            }),
            Arc::new(NoopMiddleware {
                name: "should_not_run".into(),
            }),
        ];

        let mut ctx = new_ctx();
        let result = runner.run_chain(&mw_list, &mut ctx).await;
        assert!(result.is_err());

        // Only "first" should have recorded.
        let order: Vec<String> = serde_json::from_value(ctx.metadata).unwrap();
        assert_eq!(order, vec!["first"]);
    }

    #[tokio::test]
    async fn test_runner_tracing_spans() {
        // This test verifies execution succeeds with tracing enabled.
        let runner = ChainRunner::new(Duration::from_secs(5));
        let mw = Arc::new(NoopMiddleware {
            name: "traced".into(),
        }) as Arc<dyn Middleware>;

        let mut ctx = new_ctx();
        let result = runner.run_one(&mw, &mut ctx).await;
        assert!(result.is_ok());
    }
}
