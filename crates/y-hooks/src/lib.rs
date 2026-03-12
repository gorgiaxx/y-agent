//! y-hooks: Hook system, middleware chains, async event bus, hook handlers.
//!
//! This crate provides the extension backbone for y-agent:
//!
//! - [`HookSystem`] — unified facade for the entire hook/middleware/event system
//! - [`MiddlewareChain`] — priority-sorted middleware execution pipeline
//! - [`ChainRunner`] — timeout-guarded per-middleware execution
//! - [`HookRegistry`] — lifecycle hook handler registration and dispatch
//! - [`EventBus`] — async fire-and-forget event delivery (per-subscriber channels)
//!
//! Middleware chains exist for 5 domains: Context, Tool, LLM, Compaction, Memory.
//! Guardrails, file journaling, and context assembly are all middleware.

pub mod chain;
pub mod chain_runner;
pub mod config;
pub mod error;
pub mod event_bus;
#[cfg(feature = "hook_handlers")]
pub mod hook_handler;
pub mod hook_registry;
pub mod hook_system;

// Re-export primary types.
pub use chain::MiddlewareChain;
pub use chain_runner::ChainRunner;
pub use config::HookConfig;
pub use error::HookError;
pub use event_bus::{EventBus, EventBusMetrics, Subscription};
pub use hook_registry::HookRegistry;
pub use hook_system::HookSystem;

// Re-export hook handler types.
#[cfg(feature = "hook_handlers")]
pub use hook_handler::{
    CommandHttpDecision, HookDecision, HookHandlerExecutor, HookHandlerMetrics,
    HookHandlerResult, HookInput, PromptAgentDecision,
};
