//! y-hooks: Hook system, middleware chains, async event bus, plugin loading.
//!
//! This crate provides the extension backbone for y-agent:
//!
//! - [`MiddlewareChain`] — priority-sorted middleware execution pipeline
//! - [`ChainRunner`] — timeout-guarded per-middleware execution
//! - [`HookRegistry`] — lifecycle hook handler registration and dispatch
//! - [`EventBus`] — async fire-and-forget event delivery via broadcast
//! - [`PluginLoader`] — dynamic middleware loading (Phase 4 skeleton)
//!
//! Middleware chains exist for 5 domains: Context, Tool, LLM, Compaction, Memory.
//! Guardrails, file journaling, and context assembly are all middleware.

pub mod chain;
pub mod chain_runner;
pub mod config;
pub mod error;
pub mod event_bus;
pub mod hook_registry;
pub mod plugin;

// Re-export primary types.
pub use chain::MiddlewareChain;
pub use chain_runner::ChainRunner;
pub use config::HookConfig;
pub use error::HookError;
pub use event_bus::{EventBus, Subscription};
pub use hook_registry::HookRegistry;
pub use plugin::PluginLoader;
