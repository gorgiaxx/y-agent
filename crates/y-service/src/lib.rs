//! y-service: Business/service layer for y-agent.
//!
//! This crate provides the shared business logic used by all presentation
//! layers (CLI, TUI, future Web API). It follows MVC principles:
//!
//! - **Model**: Domain crates (`y-core`, `y-provider`, `y-session`, etc.)
//! - **Service**: This crate (`y-service`) -- orchestration, workflows, cost, diagnostics
//! - **View**: Presentation crates (`y-cli` for CLI/TUI, future `y-web` for API)
//!
//! ## Key Components
//!
//! - [`ServiceContainer`] -- DI container that wires all domain services from config
//! - [`ChatService`] -- LLM turn lifecycle (context -> LLM -> tools -> diagnostics)
//! - [`CostService`] -- Token cost computation and daily summaries
//! - [`DiagnosticsService`] -- Trace queries and health checks
//! - [`SystemService`] -- System status reporting

pub mod agent_management;
pub mod agent_service;
pub mod bot;
pub mod chat;
pub mod chat_types;
pub mod chat_worker;
pub mod config;
pub mod container;
pub mod context_optimization;
pub mod cost;
pub mod diagnostics;
pub mod event_sink;
pub mod init;
pub mod knowledge_service;
pub mod mcp_service;
pub mod message_builder;
pub mod observability;
pub mod orchestrator_dispatcher;
pub mod plan_orchestrator;
pub mod rewind;
pub mod scheduler_service;
pub mod skill_evolution;
pub mod skill_ingestion;
pub mod skill_service;
pub mod system;
pub mod task_delegation_orchestrator;
pub mod tool_search_orchestrator;
pub mod user_interaction_orchestrator;
pub mod workflow_executors;
pub mod workflow_orchestrator;
pub mod workflow_service;
pub mod workspace;

// Re-export primary types for convenience.
pub use agent_management::AgentManagementService;
pub use agent_service::{
    AgentExecutionConfig, AgentExecutionError, AgentExecutionResult, AgentService,
    ServiceAgentRunner,
};
pub use bot::{BotConfig, BotPersona, BotService, BotServiceError};
pub use chat::{
    ChatService, PendingInteractions, PendingPermissions, PermissionPromptResponse,
    PrepareTurnError, PrepareTurnRequest, PreparedTurn, ResendTurnError, ResendTurnRequest,
    ToolCallRecord, TurnError, TurnEvent, TurnEventSender, TurnInput, TurnMetaSummary, TurnResult,
};
pub use config::ServiceConfig;
pub use container::ServiceContainer;
pub use cost::CostService;
pub use diagnostics::{DiagnosticsAgentDelegator, DiagnosticsService, HistoricalEntry};
pub use event_sink::EventSink;
pub use mcp_service::McpService;
pub use observability::{
    AgentInstanceSnapshot, AgentPoolSnapshot, ObservabilityService, ProviderSnapshot,
    SchedulerQueueSnapshot, SystemSnapshot,
};
pub use rewind::{FileHistoryManagers, RewindError, RewindPointInfo, RewindResult, RewindService};
pub use scheduler_service::{
    CreateScheduleRequest, ExecutionSummary, ScheduleSummary, SchedulerService,
    SchedulerServiceError, UpdateScheduleRequest,
};
pub use skill_evolution::{
    CapturedExperience, ExperienceCaptureSubscriber, SkillInjectionTracker,
    SkillUsageAuditSubscriber, UsageMetrics,
};
pub use skill_ingestion::{ImportDecision, ImportError, ImportResult, SkillIngestionService};
pub use skill_service::{SkillDetail, SkillInfo, SkillService};
pub use system::{HealthReport, ProviderInfo, ProviderTestRequest, StatusReport, SystemService};
pub use workflow_service::{
    CreateWorkflowRequest, DagEdge, DagNode, DagVisualization, UpdateWorkflowRequest,
    ValidationResult, WorkflowService, WorkflowServiceError,
};
pub use workspace::{WorkspaceRecord, WorkspaceService};

// ---------------------------------------------------------------------------
// Re-exports: infrastructure config types for presentation layers
// ---------------------------------------------------------------------------
// These re-exports allow presentation crates (y-cli, y-gui) to import config
// types from `y_service` rather than depending on each infrastructure crate
// directly, keeping the dependency graph lean.

/// Config types re-exported from infrastructure crates.
pub mod config_types {
    pub use y_browser::BrowserConfig;
    pub use y_context::PruningConfig;
    pub use y_guardrails::GuardrailConfig;
    pub use y_hooks::HookConfig;
    pub use y_knowledge::config::KnowledgeConfig;
    pub use y_provider::ProviderPoolConfig;
    pub use y_runtime::RuntimeConfig;
    pub use y_session::SessionConfig;
    pub use y_storage::StorageConfig;
    pub use y_tools::ToolRegistryConfig;
}

// Flat re-exports for backward compatibility.
// Prefer importing from `config_types::` module instead.
#[deprecated(note = "import from y_service::config_types::BrowserConfig instead")]
pub use y_browser::BrowserConfig;
#[deprecated(note = "import from y_service::config_types::PruningConfig instead")]
pub use y_context::PruningConfig;
#[deprecated(note = "import from y_service::config_types::GuardrailConfig instead")]
pub use y_guardrails::GuardrailConfig;
#[deprecated(note = "import from y_service::config_types::HookConfig instead")]
pub use y_hooks::HookConfig;
#[deprecated(note = "import from y_service::config_types::KnowledgeConfig instead")]
pub use y_knowledge::config::KnowledgeConfig;
#[deprecated(note = "import from y_service::config_types::ProviderPoolConfig instead")]
pub use y_provider::ProviderPoolConfig;
#[deprecated(note = "import from y_service::config_types::RuntimeConfig instead")]
pub use y_runtime::RuntimeConfig;
#[deprecated(note = "import from y_service::config_types::SessionConfig instead")]
pub use y_session::SessionConfig;
#[deprecated(note = "import from y_service::config_types::StorageConfig instead")]
pub use y_storage::StorageConfig;
#[deprecated(note = "import from y_service::config_types::ToolRegistryConfig instead")]
pub use y_tools::ToolRegistryConfig;

// Re-export context assembly types (used in tests by presentation layers).
pub use y_context::{AssembledContext, ContextCategory, ContextItem};

// Re-export knowledge tool param types (used by CLI `kb` command).
pub use y_knowledge::tools::{KnowledgeIngestParams, KnowledgeSearchParams};

// Re-export provider config sub-types (used in test code by presentation layers).
pub use y_provider::config::ProviderConfig;

// Re-export prompt types (used by CLI chat command and init).
pub use y_prompt::{PromptContext, BUILTIN_PROMPT_FILES};

// Re-export storage functions (used by CLI `init` command for DB setup).
pub use y_storage::create_pool;

/// Storage migration helpers re-exported for presentation layers.
pub mod migration {
    pub use y_storage::migration::run_embedded_migrations;
}

// Re-export workflow store types (used by CLI `workflow` command and workflow service).
pub use y_storage::workflow_store::WorkflowRow;

// Re-export scheduler types (used by scheduler service and REST routes).
pub use y_scheduler::{
    Schedule, SchedulePolicies, SchedulerConfig, SchedulerManager, TriggerConfig,
};
