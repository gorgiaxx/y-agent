//! y-provider: Provider pool — LLM communication, routing, freeze/thaw, metrics.
//!
//! This crate provides the gateway layer for LLM interactions:
//!
//! - [`ProviderPoolImpl`] — implements `ProviderPool` with tag-based routing
//! - [`TagBasedRouter`] — multi-tag matching with preferred model support
//! - [`FreezeManager`] — exponential backoff freeze/thaw lifecycle
//! - [`HealthChecker`] — health probe for frozen provider recovery
//! - [`ProviderMetrics`] — lock-free per-provider request/token counters
//! - [`PriorityScheduler`] — three-tier priority scheduling (Critical/Normal/Idle)
//! - [`OpenAiProvider`] — OpenAI-compatible LLM backend
//! - [`AnthropicProvider`] — Anthropic Messages API backend
//! - [`GeminiProvider`] — Google Gemini API backend
//! - [`OllamaProvider`] — Ollama local LLM backend
//! - [`AzureOpenAiProvider`] — Azure `OpenAI` Service backend
//! - [`LeaseManager`] — request lifecycle tracking with RAII guards
//! - [`metrics_export`] — Prometheus-compatible metrics rendering

pub mod agent_runner;
pub mod config;
pub mod embedding;
pub mod error;
pub mod error_classifier;
pub mod freeze;
pub mod health;
pub mod hook_llm_runner;
pub mod lease;
pub mod metrics;
pub mod metrics_export;
pub mod pool;
pub mod providers;
pub mod router;
pub mod scheduler;

// Re-export primary types.
pub use config::{ProviderConfig, ProviderPoolConfig, ProxySpec};
pub use error::ProviderPoolError;
pub use error_classifier::{classify, classify_provider_error, StandardError};
pub use freeze::FreezeManager;
pub use health::HealthChecker;
pub use hook_llm_runner::ProviderPoolHookLlmRunner;
pub use lease::{LeaseGuard, LeaseId, LeaseManager};
pub use metrics::{MetricsSnapshot, ProviderMetrics};
pub use metrics_export::render_prometheus;
pub use pool::ProviderPoolImpl;
pub use providers::anthropic::AnthropicProvider;
pub use providers::azure::AzureOpenAiProvider;
pub use providers::gemini::GeminiProvider;
pub use providers::ollama::OllamaProvider;
pub use providers::openai::OpenAiProvider;
pub use router::{SelectionStrategy, TagBasedRouter};
pub use scheduler::{PriorityScheduler, SchedulerSnapshot};
pub use agent_runner::SingleTurnRunner;
pub use embedding::{EmbeddingConfig, OpenAiEmbeddingProvider};
