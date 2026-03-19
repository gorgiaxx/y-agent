//! Benchmarks for middleware chain and event bus dispatch.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use std::sync::Arc;

use async_trait::async_trait;
use y_core::hook::{
    ChainType, Event, EventFilter, Middleware, MiddlewareContext, MiddlewareError, MiddlewareResult,
};
use y_hooks::chain::MiddlewareChain;
use y_hooks::event_bus::EventBus;

/// Simple pass-through middleware for benchmarking chain overhead.
struct NoOpMiddleware {
    name: String,
    priority: u32,
}

impl NoOpMiddleware {
    fn new(name: &str, priority: u32) -> Self {
        Self {
            name: name.to_string(),
            priority,
        }
    }
}

#[async_trait]
impl Middleware for NoOpMiddleware {
    async fn execute(
        &self,
        _ctx: &mut MiddlewareContext,
    ) -> Result<MiddlewareResult, MiddlewareError> {
        Ok(MiddlewareResult::Continue)
    }

    fn chain_type(&self) -> ChainType {
        ChainType::Context
    }

    fn priority(&self) -> u32 {
        self.priority
    }

    fn name(&self) -> &str {
        &self.name
    }
}

fn bench_middleware_chain(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("middleware_chain_10", |b| {
        b.iter(|| {
            rt.block_on(async {
                let mut chain = MiddlewareChain::new(ChainType::Context);
                for i in 0..10 {
                    let mw: Arc<dyn Middleware> =
                        Arc::new(NoOpMiddleware::new(&format!("mw-{i}"), i * 100));
                    chain.register(mw).unwrap();
                }
                let mut ctx = MiddlewareContext::new(
                    ChainType::Context,
                    black_box(serde_json::json!({"messages": []})),
                );
                let _result = chain.execute(&mut ctx).await;
            });
        });
    });

    c.bench_function("middleware_chain_50", |b| {
        b.iter(|| {
            rt.block_on(async {
                let mut chain = MiddlewareChain::new(ChainType::Context);
                for i in 0..50 {
                    let mw: Arc<dyn Middleware> =
                        Arc::new(NoOpMiddleware::new(&format!("mw-{i}"), i * 100));
                    chain.register(mw).unwrap();
                }
                let mut ctx = MiddlewareContext::new(
                    ChainType::Context,
                    black_box(serde_json::json!({"messages": []})),
                );
                let _result = chain.execute(&mut ctx).await;
            });
        });
    });
}

fn bench_event_bus(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("event_bus_publish_1000", |b| {
        b.iter(|| {
            rt.block_on(async {
                let bus = EventBus::new(2048);
                let _sub = bus.subscribe(EventFilter::all()).await;
                for i in 0..1000 {
                    let event = Event::ToolExecuted {
                        tool_name: format!("tool-{i}"),
                        success: true,
                        duration_ms: 42,
                    };
                    let _ = bus.publish(black_box(event)).await;
                }
            });
        });
    });
}

fn bench_middleware_register(c: &mut Criterion) {
    c.bench_function("middleware_register_unregister", |b| {
        b.iter(|| {
            let mut chain = MiddlewareChain::new(ChainType::Context);
            for i in 0..10 {
                let mw: Arc<dyn Middleware> =
                    Arc::new(NoOpMiddleware::new(&format!("mw-{i}"), i * 100));
                chain.register(mw).unwrap();
            }
            for i in 0..10 {
                chain.unregister(&format!("mw-{i}")).unwrap();
            }
        });
    });
}

criterion_group!(
    benches,
    bench_middleware_chain,
    bench_event_bus,
    bench_middleware_register
);
criterion_main!(benches);
