//! y-provider: Provider pool — LLM communication, routing, freeze/thaw, metrics.
//!
//! This crate provides the gateway layer for LLM interactions:
//!
//! - [`ProviderPoolImpl`] — implements `ProviderPool` with tag-based routing
//! - [`TagBasedRouter`] — multi-tag matching with preferred model support
//! - [`FreezeManager`] — exponential backoff freeze/thaw lifecycle
//! - [`HealthChecker`] — health probe for frozen provider recovery
//! - [`ProviderMetrics`] — lock-free per-provider request/token counters
//! - [`OpenAiProvider`] — OpenAI-compatible LLM backend

pub mod config;
pub mod error;
pub mod freeze;
pub mod health;
pub mod metrics;
pub mod pool;
pub mod providers;
pub mod router;

// Re-export primary types.
pub use config::{ProviderConfig, ProviderPoolConfig};
pub use error::ProviderPoolError;
pub use freeze::FreezeManager;
pub use health::HealthChecker;
pub use metrics::{MetricsSnapshot, ProviderMetrics};
pub use pool::ProviderPoolImpl;
pub use providers::openai::OpenAiProvider;
pub use router::TagBasedRouter;
