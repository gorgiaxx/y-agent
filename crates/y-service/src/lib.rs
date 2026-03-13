//! y-service: Business/service layer for y-agent.
//!
//! This crate provides the shared business logic used by all presentation
//! layers (CLI, TUI, future Web API). It follows MVC principles:
//!
//! - **Model**: Domain crates (`y-core`, `y-provider`, `y-session`, etc.)
//! - **Service**: This crate (`y-service`) — orchestration, workflows, cost, diagnostics
//! - **View**: Presentation crates (`y-cli` for CLI/TUI, future `y-web` for API)
//!
//! ## Key Components
//!
//! - [`ServiceContainer`] — DI container that wires all domain services from config
//! - [`ChatService`] — LLM turn lifecycle (context → LLM → tools → diagnostics)
//! - [`CostService`] — Token cost computation and daily summaries
//! - [`DiagnosticsService`] — Trace queries and health checks
//! - [`SystemService`] — System status reporting

pub mod chat;
pub mod config;
pub mod container;
pub mod cost;
pub mod diagnostics;
pub mod skill_evolution;
pub mod skill_ingestion;
pub mod system;
pub mod tool_search_orchestrator;

// Re-export primary types for convenience.
pub use chat::{
    ChatService, PrepareTurnError, PrepareTurnRequest, PreparedTurn, ToolCallRecord, TurnError,
    TurnEvent, TurnEventSender, TurnInput, TurnMetaSummary, TurnResult,
};
pub use config::ServiceConfig;
pub use container::ServiceContainer;
pub use cost::CostService;
pub use diagnostics::{DiagnosticsService, HistoricalEntry};
pub use skill_evolution::{
    CapturedExperience, ExperienceCaptureSubscriber, SkillInjectionTracker,
    SkillUsageAuditSubscriber, UsageMetrics,
};
pub use skill_ingestion::{ImportDecision, ImportError, ImportResult, SkillIngestionService};
pub use system::{
    HealthReport, ProviderInfo, ProviderTestRequest, StatusReport, SystemService,
};
