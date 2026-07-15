//! Diagnostics subsystem: query service, agent delegator, and gateways.
//!
//! Decouples diagnostics recording from business logic. LLM calls and
//! tool calls are automatically captured at the gateway level.

mod adaptation_metrics;
pub mod agent_delegator;
pub mod provider_pool;
pub mod service;
pub mod tool_gateway;

// Re-exports for convenient access.
pub use adaptation_metrics::{
    DynamicAgentRegressionFinding, DynamicAgentVersionMetrics, OrchestrationModeMetrics,
};
pub use agent_delegator::DiagnosticsAgentDelegator;
pub use provider_pool::DiagnosticsProviderPool;
pub use service::{DiagnosticsService, HealthCheckResult, HistoricalEntry};
pub use tool_gateway::DiagnosticsToolGateway;
