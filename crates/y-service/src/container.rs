//! Central dependency container — shared by all presentation layers.
//!
//! Mirrors the former `AppServices` from `y-cli/wire.rs`, but lives in the
//! service layer so CLI, TUI, and future Web API can all construct one.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::sync::{Mutex, RwLock};
use tracing::{info, warn};

use y_agent::{AgentPool, AgentRegistry, DelegationTracker, MultiAgentConfig};
use y_context::{
    BuildSystemPromptProvider, BunVenvPromptInfo, CompactionConfig, CompactionEngine,
    CompactionLlm, ContextPipeline, InjectContextStatus, InjectSkills, InjectTools,
    KnowledgeContextProvider, PruningEngine, PythonVenvPromptInfo, SystemPromptConfig,
    VenvPromptInfo,
};
use y_core::agent::AgentDelegator;
use y_core::permission_types::PermissionMode;

use y_core::types::{SessionId, ToolName};
use y_diagnostics::{DiagnosticsEvent, DiagnosticsSubscriber, TraceStore};
use y_guardrails::GuardrailManager;
use y_hooks::HookSystem;
use y_prompt::{builtin_section_store_with_overrides, default_template, PromptContext};
use y_provider::ProviderPoolImpl;
use y_provider::SingleTurnRunner;
use y_runtime::{RuntimeManager, VenvManager};
use y_session::{ChatCheckpointManager, SessionManager};
use y_skills::{SkillRegistryImpl, SkillSearch};
use y_storage::{
    SqliteChatCheckpointStore, SqliteChatMessageStore, SqliteProviderMetricsStore,
    SqliteScheduleStore, SqliteSessionStore, SqliteWorkflowStore,
};
use y_tools::{ToolActivationSet, ToolRegistryImpl, ToolTaxonomy};

use crate::config::ServiceConfig;

use crate::knowledge_service::KnowledgeService;
use crate::skill_ingestion::{import_skill_from_path, SkillImportOutcome, SkillIngestionService};

use y_mcp::McpConnectionManager;

/// Embedded default taxonomy TOML (compiled into binary).
const DEFAULT_TAXONOMY_TOML: &str = include_str!("../../../config/tool_taxonomy.toml");

/// Default `ToolActivationSet` ceiling.
const ACTIVATION_SET_CEILING: usize = 20;
/// Background scheduler poll interval for long-lived presentation layers.
const BACKGROUND_SCHEDULER_TICK_INTERVAL: Duration = Duration::from_secs(1);

/// All wired application services, constructed from [`ServiceConfig`].
///
/// Fields are logically grouped by domain. A future refactoring may extract
/// these groups into dedicated sub-structs (e.g. `SessionServices`,
/// `ToolServices`).
#[allow(dead_code)]
pub struct ServiceContainer {
    // -- Provider ----------------------------------------------------------
    /// Provider pool for LLM communication.
    /// Wrapped in `RwLock` to support hot-reload of provider configuration.
    provider_pool: RwLock<Arc<ProviderPoolImpl>>,

    // -- Sessions ----------------------------------------------------------
    /// Session manager for session lifecycle.
    pub session_manager: SessionManager,

    /// Chat checkpoint manager for turn-level rollback.
    pub chat_checkpoint_manager: ChatCheckpointManager,

    /// Chat message store for session history tree (Phase 2).
    pub chat_message_store: Arc<SqliteChatMessageStore>,

    // -- Tools -------------------------------------------------------------
    /// Tool registry for tool management.
    pub tool_registry: ToolRegistryImpl,

    /// Session-scoped tool activation set (LRU, ceiling 20).
    pub tool_activation_set: Arc<RwLock<ToolActivationSet>>,

    /// Hierarchical tool taxonomy for prompt-based discovery.
    pub tool_taxonomy: Arc<RwLock<ToolTaxonomy>>,

    /// Skill search index for unified `ToolSearch` capability discovery.
    ///
    /// Pre-loaded from the skills directory at startup. Wrapped in `RwLock`
    /// so it can be refreshed when skills are added/removed.
    pub skill_search: RwLock<SkillSearch>,

    // -- Agents ------------------------------------------------------------
    /// Agent registry for definition management.
    pub agent_registry: Arc<Mutex<AgentRegistry>>,

    /// Agent pool for runtime instance management.
    /// Shared via `Arc` with `MutexPoolDelegator` so that runner upgrades
    /// (via `init_agent_runner`) affect both direct pool access and delegation.
    pub agent_pool: Arc<RwLock<AgentPool>>,

    /// Agent delegator for delegating tasks to agents.
    pub agent_delegator: Arc<dyn AgentDelegator>,

    /// Shared delegation tracker for observability.
    pub delegation_tracker: Arc<DelegationTracker>,

    /// Dynamic text listing user-callable agents for prompt injection.
    pub callable_agents_text: Arc<RwLock<String>>,

    // -- Context -----------------------------------------------------------
    /// Context pipeline for prompt assembly.
    pub context_pipeline: ContextPipeline,

    /// Shared prompt context, updated per-turn by the chat loop.
    pub prompt_context: Arc<RwLock<PromptContext>>,

    /// Pruning engine for failed tool call removal and summarization.
    pub pruning_engine: PruningEngine,

    /// Compaction engine for older history summarization.
    pub compaction_engine: CompactionEngine,

    /// Compaction trigger threshold as a percentage of `context_window`.
    pub compaction_threshold_pct: u32,

    /// Per-session token watermarks for delta-based pruning.
    pub pruning_watermarks: RwLock<HashMap<SessionId, u32>>,

    // -- Diagnostics -------------------------------------------------------
    /// Diagnostics subscriber for trace recording.
    pub diagnostics: Arc<DiagnosticsSubscriber<dyn TraceStore>>,

    /// Broadcast channel for real-time diagnostics events.
    pub diagnostics_broadcast: tokio::sync::broadcast::Sender<DiagnosticsEvent>,

    /// Tool call diagnostics gateway.
    pub tool_gateway: Arc<crate::diagnostics::DiagnosticsToolGateway>,

    /// Provider metrics event log store for persistence across restarts.
    pub provider_metrics_store: SqliteProviderMetricsStore,

    // -- Middleware ---------------------------------------------------------
    /// Unified hook system (registry, event bus, middleware chains).
    pub hook_system: std::sync::RwLock<HookSystem>,

    /// Runtime manager for tool execution environments.
    pub runtime_manager: Arc<RuntimeManager>,

    /// Guardrail manager for security middleware.
    pub guardrail_manager: GuardrailManager,

    // -- Scheduler ---------------------------------------------------------
    /// Workflow store for persistent workflow templates.
    pub workflow_store: SqliteWorkflowStore,

    /// Schedule store for persistent schedule definitions.
    pub schedule_store: SqliteScheduleStore,

    /// Scheduler manager for scheduled task management.
    pub scheduler_manager: y_scheduler::SchedulerManager,

    // -- Knowledge ---------------------------------------------------------
    /// Knowledge base service (ingestion, retrieval, embedding).
    pub knowledge_service: Arc<Mutex<KnowledgeService>>,

    // -- Interactions (HITL) -----------------------------------------------
    /// Pending user-interaction answer channels for `AskUser` tool calls.
    pub pending_interactions: crate::chat::PendingInteractions,

    /// Pending permission-approval channels for HITL permission requests.
    pub pending_permissions: crate::chat::PendingPermissions,

    /// Session-scoped permission overrides.
    pub session_permission_modes: Arc<RwLock<HashMap<SessionId, PermissionMode>>>,

    // -- Bot ---------------------------------------------------------------
    /// Path to the bot persona directory (`~/.config/y-agent/persona/`).
    pub persona_dir: Option<PathBuf>,

    // -- MCP ---------------------------------------------------------------
    /// MCP connection manager for multi-server lifecycle.
    pub mcp_manager: Arc<McpConnectionManager>,

    // -- File History (Rewind) ---------------------------------------------
    /// Per-session file history managers for rewind support.
    pub file_history_managers: crate::rewind::FileHistoryManagers,

    /// Data directory root (parent of the `SQLite` database file).
    /// Used for constructing file-history backup paths.
    pub data_dir: PathBuf,
}

// ---------------------------------------------------------------------------
// Intermediate result types for init helpers
// ---------------------------------------------------------------------------

/// Session infrastructure initialised by [`ServiceContainer::init_sessions`].
struct SessionInit {
    session_manager: SessionManager,
    chat_checkpoint_manager: ChatCheckpointManager,
    chat_message_store: Arc<SqliteChatMessageStore>,
}

/// Tool infrastructure initialised by [`ServiceContainer::init_tools`].
struct ToolInit {
    tool_registry: ToolRegistryImpl,
    tool_taxonomy: Arc<RwLock<ToolTaxonomy>>,
    tool_activation_set: Arc<RwLock<ToolActivationSet>>,
}

/// Context pipeline initialised by [`ServiceContainer::init_context_pipeline`].
struct ContextPipelineInit {
    pipeline: ContextPipeline,
    prompt_context: Arc<RwLock<PromptContext>>,
    skill_search: SkillSearch,
    callable_agents_text: Arc<RwLock<String>>,
}

/// Scheduler infrastructure initialised by [`ServiceContainer::init_scheduler`].
struct SchedulerInit {
    workflow_store: SqliteWorkflowStore,
    schedule_store: SqliteScheduleStore,
    scheduler_manager: y_scheduler::SchedulerManager,
}

/// Diagnostics infrastructure initialised by [`ServiceContainer::init_diagnostics`].
struct DiagnosticsInit {
    diagnostics: Arc<DiagnosticsSubscriber<dyn TraceStore>>,
    broadcast_tx: tokio::sync::broadcast::Sender<DiagnosticsEvent>,
    tool_gateway: Arc<crate::diagnostics::DiagnosticsToolGateway>,
}

impl ServiceContainer {
    /// Wire all services from a validated [`ServiceConfig`].
    ///
    /// This is the sole dependency-wiring entry point. Presentation layers
    /// call this once and pass the resulting `ServiceContainer` to service
    /// methods and command handlers.
    ///
    /// # Panics
    ///
    /// Panics if the fallback tool taxonomy TOML is invalid.
    pub async fn from_config(config: &ServiceConfig) -> Result<Self> {
        // 1. Storage -- SQLite pool + schema compatibility handling.
        let pool = Self::init_storage(config).await?;

        // 2. Session infrastructure.
        let sessions = Self::init_sessions(&pool, config);

        // 3. Provider pool.
        let providers = y_provider::build_providers(&config.providers);
        let provider_pool = Arc::new(ProviderPoolImpl::from_providers(
            providers,
            &config.providers,
        ));

        // 4. Hook system.
        let hook_system = Self::init_hooks(config, &provider_pool);

        // 5. Guardrails.
        let guardrail_manager = GuardrailManager::new(config.guardrails.clone());

        // 6. Knowledge service (early -- needed for tool registration).
        let (knowledge_service, embedding_provider) = Self::init_knowledge_service(config);

        // 7. Tool registry + taxonomy + activation set.
        let tools = Self::init_tools(config, &knowledge_service, embedding_provider.clone()).await;

        // 8. Runtime manager + venvs.
        let (runtime_manager, venv_info) = Self::init_runtime(config).await;

        // 9. Context pipeline.
        let ctx = Self::init_context_pipeline(
            config,
            &tools.tool_registry,
            &tools.tool_activation_set,
            &tools.tool_taxonomy,
            venv_info,
        )
        .await;

        // 10. Workflow + scheduler.
        let sched = Self::init_scheduler_services(&pool).await;

        // 11. Diagnostics.
        let diag = Self::init_diagnostics(&pool);

        // 12. Agent infrastructure.
        let config_dir = config.prompts_dir.as_ref().and_then(|p| p.parent());
        let (agent_registry, agent_pool, agent_delegator, delegation_tracker) =
            Self::init_agent_and_diagnostics(
                config_dir,
                &provider_pool,
                &diag.diagnostics,
                diag.broadcast_tx.clone(),
            );

        // 13. Pruning + compaction.
        let pruning_engine =
            PruningEngine::with_delegator(config.pruning.clone(), Arc::clone(&agent_delegator));
        let compaction_llm: Box<dyn CompactionLlm> =
            Box::new(DelegatingCompactionLlm(Arc::clone(&agent_delegator)));
        let compaction_engine =
            CompactionEngine::with_llm(CompactionConfig::default(), compaction_llm);

        // 14. Provider metrics.
        let provider_metrics_store = SqliteProviderMetricsStore::new(pool.clone());
        {
            let receivers = provider_pool.attach_event_senders();
            let pms = provider_metrics_store.clone();
            tokio::spawn(async move {
                Self::run_metrics_event_consumers(receivers, pms).await;
            });
        }

        // 15. MCP connection manager (connections started later via
        //     start_background_services).
        let mcp_manager = Arc::new(McpConnectionManager::new(None));

        Ok(Self {
            provider_pool: RwLock::new(provider_pool),
            session_manager: sessions.session_manager,
            hook_system: std::sync::RwLock::new(hook_system),
            tool_registry: tools.tool_registry,
            runtime_manager,
            context_pipeline: ctx.pipeline,
            guardrail_manager,
            agent_registry,
            agent_pool,
            agent_delegator,
            delegation_tracker,
            workflow_store: sched.workflow_store,
            schedule_store: sched.schedule_store,
            scheduler_manager: sched.scheduler_manager,
            prompt_context: ctx.prompt_context,
            diagnostics: diag.diagnostics,
            diagnostics_broadcast: diag.broadcast_tx,
            tool_gateway: diag.tool_gateway,
            chat_checkpoint_manager: sessions.chat_checkpoint_manager,
            tool_activation_set: tools.tool_activation_set,
            tool_taxonomy: tools.tool_taxonomy,
            chat_message_store: sessions.chat_message_store,
            knowledge_service,
            pruning_engine,
            compaction_engine,
            compaction_threshold_pct: config.session.compaction_threshold_pct,
            pruning_watermarks: RwLock::new(HashMap::new()),
            provider_metrics_store,
            skill_search: RwLock::new(ctx.skill_search),
            callable_agents_text: ctx.callable_agents_text,
            pending_interactions: std::sync::Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            pending_permissions: std::sync::Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            session_permission_modes: Arc::new(RwLock::new(HashMap::new())),
            persona_dir: config.persona_dir.clone(),
            mcp_manager,
            file_history_managers: crate::rewind::create_file_history_managers(),
            data_dir: {
                let db_path = std::path::Path::new(&config.storage.db_path);
                // For :memory: databases, use transcript_dir's parent as data_dir
                // to ensure file-history is created in a temp directory during tests.
                if db_path.parent().is_none_or(|p| p.as_os_str().is_empty()) {
                    config
                        .storage
                        .transcript_dir
                        .parent()
                        .unwrap_or(std::path::Path::new("."))
                        .to_path_buf()
                } else {
                    db_path.parent().unwrap().to_path_buf()
                }
            },
        })
    }

    // -- Init helpers (private) --------------------------------------------

    /// Create a `SQLite` pool and reconcile the on-disk schema if needed.
    async fn init_storage(config: &ServiceConfig) -> Result<y_storage::SqlitePool> {
        y_storage::migration::prepare_database(&config.storage)
            .await
            .context("failed to prepare SQLite database")?;
        let pool = y_storage::create_pool(&config.storage)
            .await
            .context("failed to create SQLite pool")?;
        y_storage::migration::run_embedded_migrations(&pool)
            .await
            .context("failed to initialize SQLite schema")?;
        Ok(pool)
    }

    /// Construct session manager, checkpoint manager, and message store.
    fn init_sessions(pool: &y_storage::SqlitePool, config: &ServiceConfig) -> SessionInit {
        let session_store: Arc<dyn y_core::session::SessionStore> =
            Arc::new(SqliteSessionStore::new(pool.clone()));
        let transcript_store: Arc<dyn y_core::session::TranscriptStore> = Arc::new(
            y_storage::JsonlTranscriptStore::new(&config.storage.transcript_dir),
        );
        let display_transcript_store: Arc<dyn y_core::session::DisplayTranscriptStore> = Arc::new(
            y_storage::JsonlDisplayTranscriptStore::new(&config.storage.transcript_dir),
        );
        let session_manager = SessionManager::new(
            Arc::clone(&session_store),
            Arc::clone(&transcript_store),
            Arc::clone(&display_transcript_store),
            config.session.clone(),
        );
        let chat_checkpoint_store = Arc::new(SqliteChatCheckpointStore::new(pool.clone()));
        let chat_checkpoint_manager = ChatCheckpointManager::new(
            Arc::clone(&transcript_store),
            Arc::clone(&display_transcript_store),
            chat_checkpoint_store,
            Arc::clone(&session_store),
        );
        let chat_message_store = Arc::new(SqliteChatMessageStore::new(pool.clone()));

        SessionInit {
            session_manager,
            chat_checkpoint_manager,
            chat_message_store,
        }
    }

    /// Create hook system with optional LLM runner injection.
    fn init_hooks(config: &ServiceConfig, _provider_pool: &Arc<ProviderPoolImpl>) -> HookSystem {
        #[allow(unused_mut)]
        let mut hook_system = HookSystem::new(&config.hooks);
        #[cfg(all(feature = "hook_handlers", feature = "llm_hooks"))]
        {
            use y_core::provider::ProviderPool as _;
            let llm_runner = Arc::new(y_provider::ProviderPoolHookLlmRunner::new(Arc::new(
                _provider_pool.clone(),
            )
                as Arc<dyn y_core::provider::ProviderPool>));
            hook_system.set_llm_runner(llm_runner);
            info!("Prompt hook LLM runner injected");
        }
        hook_system
    }

    /// Initialise tool registry, taxonomy, and activation set.
    async fn init_tools(
        config: &ServiceConfig,
        knowledge_service: &Arc<Mutex<KnowledgeService>>,
        embedding_provider: Option<Arc<dyn y_core::embedding::EmbeddingProvider>>,
    ) -> ToolInit {
        let tool_registry = ToolRegistryImpl::new(config.tools.clone());
        let kb_handle = {
            let ks = knowledge_service.lock().await;
            ks.knowledge_handle()
        };
        y_tools::builtin::register_builtin_tools(
            &tool_registry,
            config.browser.clone(),
            Some(kb_handle),
            embedding_provider,
        )
        .await;

        let tool_taxonomy = Arc::new(RwLock::new(
            ToolTaxonomy::from_toml(DEFAULT_TAXONOMY_TOML).unwrap_or_else(|e| {
                warn!(error = %e, "failed to load tool taxonomy; using empty");
                ToolTaxonomy::from_toml(
                    r#"
[categories.meta]
label = "Meta"
description = "Tool management"
tools = ["ToolSearch"]
"#,
                )
                .expect("fallback taxonomy")
            }),
        ));
        let tool_activation_set =
            Arc::new(RwLock::new(ToolActivationSet::new(ACTIVATION_SET_CEILING)));
        pre_activate_core_tools(&tool_registry, &tool_activation_set).await;

        ToolInit {
            tool_registry,
            tool_taxonomy,
            tool_activation_set,
        }
    }

    /// Initialise runtime manager and build venv prompt info.
    async fn init_runtime(config: &ServiceConfig) -> (Arc<RuntimeManager>, VenvPromptInfo) {
        let runtime_manager = Arc::new(RuntimeManager::new(config.runtime.clone(), None));
        let venv_report = VenvManager::init_all(&config.runtime).await;
        let venv_info = VenvPromptInfo {
            python: venv_report.python.as_ref().and_then(|s| {
                if s.ready {
                    Some(PythonVenvPromptInfo {
                        uv_path: s.binary_path.clone(),
                        python_version: config.runtime.python_venv.python_version.clone(),
                        venv_dir: config.runtime.python_venv.venv_dir.clone(),
                        working_dir: config.runtime.python_venv.working_dir.clone(),
                    })
                } else {
                    warn!(msg = %s.message, "Python venv not ready");
                    None
                }
            }),
            bun: venv_report.bun.as_ref().and_then(|s| {
                if s.ready {
                    Some(BunVenvPromptInfo {
                        bun_path: s.binary_path.clone(),
                        bun_version: config.runtime.bun_venv.bun_version.clone(),
                        working_dir: config.runtime.bun_venv.working_dir.clone(),
                    })
                } else {
                    warn!(msg = %s.message, "Bun venv not ready");
                    None
                }
            }),
        };
        (runtime_manager, venv_info)
    }

    /// Build the context pipeline with all providers.
    async fn init_context_pipeline(
        config: &ServiceConfig,
        tool_registry: &ToolRegistryImpl,
        tool_activation_set: &Arc<RwLock<ToolActivationSet>>,
        tool_taxonomy: &Arc<RwLock<ToolTaxonomy>>,
        venv_info: VenvPromptInfo,
    ) -> ContextPipelineInit {
        let prompt_context = Arc::new(RwLock::new(PromptContext::default()));
        let mut pipeline = ContextPipeline::new();

        // Clone venv_info before it is consumed by the system prompt provider,
        // so we can also pass it to InjectSkills for template variable expansion.
        let venv_info_for_skills = venv_info.clone();

        let callable_agents_text;
        {
            let mut sys_prompt_provider = BuildSystemPromptProvider::with_venv_info(
                default_template(),
                builtin_section_store_with_overrides(
                    config.prompts_dir.as_deref(),
                    &config.runtime.default_backend,
                ),
                Arc::clone(&prompt_context),
                SystemPromptConfig::default(),
                venv_info,
                config.runtime.default_backend.clone(),
            );
            sys_prompt_provider.set_prompts_dir(config.prompts_dir.clone());
            callable_agents_text = sys_prompt_provider.callable_agents_handle();
            pipeline.register(Box::new(sys_prompt_provider));
        }
        pipeline.register(Box::new(InjectContextStatus::new(4096)));

        let core_tools_summary = {
            let set = tool_activation_set.read().await;
            build_core_tools_summary(&set)
        };
        let tool_names: Vec<String> = tool_registry
            .get_all_definitions()
            .await
            .iter()
            .map(|d| d.name.as_str().to_string())
            .collect();
        pipeline.register(Box::new(InjectTools::dynamic(
            tool_names,
            tool_taxonomy.read().await.root_summary(),
            core_tools_summary,
            Arc::clone(&prompt_context),
        )));

        let skill_search = Self::build_skill_search_index(config.skills_dir.as_deref());
        if let Some(ref skills_dir) = config.skills_dir {
            pipeline.register(Box::new(InjectSkills::new(
                Arc::clone(&prompt_context),
                skills_dir.clone(),
                venv_info_for_skills,
            )));
        }
        pipeline.register(Box::new(KnowledgeContextProvider::new()));

        ContextPipelineInit {
            pipeline,
            prompt_context,
            skill_search,
            callable_agents_text,
        }
    }

    /// Initialise workflow store, schedule store, and scheduler manager.
    async fn init_scheduler_services(pool: &y_storage::SqlitePool) -> SchedulerInit {
        let workflow_store = SqliteWorkflowStore::new(pool.clone());
        let schedule_store = SqliteScheduleStore::new(pool.clone());
        let scheduler_manager = crate::scheduler_service::SchedulerService::create_manager();
        crate::scheduler_service::SchedulerService::attach_persistence(
            &scheduler_manager,
            schedule_store.clone(),
        )
        .await;
        if let Err(e) = crate::scheduler_service::SchedulerService::load_schedules_from_db(
            &scheduler_manager,
            &schedule_store,
        )
        .await
        {
            warn!(error = %e, "Failed to load persisted schedules; starting with empty store");
        }
        if let Err(e) = crate::scheduler_service::SchedulerService::load_executions_from_db(
            &scheduler_manager,
            &schedule_store,
        )
        .await
        {
            warn!(error = %e, "Failed to load persisted schedule executions; starting with empty history");
        }
        SchedulerInit {
            workflow_store,
            schedule_store,
            scheduler_manager,
        }
    }

    /// Initialise diagnostics subscriber, broadcast channel, and tool gateway.
    fn init_diagnostics(pool: &y_storage::SqlitePool) -> DiagnosticsInit {
        let sqlite_trace_store = y_diagnostics::SqliteTraceStore::new(pool.clone());
        let trace_store_dyn: Arc<dyn TraceStore> = Arc::new(sqlite_trace_store);
        let diagnostics = Arc::new(DiagnosticsSubscriber::new(trace_store_dyn));
        let (broadcast_tx, _) = tokio::sync::broadcast::channel::<DiagnosticsEvent>(256);
        let tool_gateway = Arc::new(crate::diagnostics::DiagnosticsToolGateway::new(
            Arc::clone(&diagnostics),
            broadcast_tx.clone(),
        ));
        DiagnosticsInit {
            diagnostics,
            broadcast_tx,
            tool_gateway,
        }
    }

    /// Initialise the knowledge service and optional embedding provider.
    fn init_knowledge_service(
        config: &ServiceConfig,
    ) -> (
        Arc<Mutex<KnowledgeService>>,
        Option<Arc<dyn y_core::embedding::EmbeddingProvider>>,
    ) {
        let knowledge_data_dir = {
            let db_path = std::path::Path::new(&config.storage.db_path);
            // For :memory: databases, use transcript_dir's parent as base
            // to ensure knowledge data is created in a temp directory during tests.
            let base_dir = if db_path.parent().is_none_or(|p| p.as_os_str().is_empty()) {
                config
                    .storage
                    .transcript_dir
                    .parent()
                    .unwrap_or(std::path::Path::new("."))
            } else {
                db_path.parent().unwrap()
            };
            base_dir.join("knowledge")
        };

        let embedding_provider: Option<Arc<dyn y_core::embedding::EmbeddingProvider>> = if config
            .knowledge
            .embedding_enabled
        {
            let embedding_config = y_provider::EmbeddingConfig {
                enabled: true,
                model: config.knowledge.embedding_model.clone(),
                dimensions: config.knowledge.embedding_dimensions,
                base_url: config.knowledge.embedding_base_url.clone(),
                api_key_env: config.knowledge.embedding_api_key_env.clone(),
                api_key: config.knowledge.embedding_api_key.clone(),
                ..Default::default()
            };
            match y_provider::OpenAiEmbeddingProvider::from_config(&embedding_config) {
                Ok(provider) => {
                    info!(
                        model = %embedding_config.model,
                        dimensions = embedding_config.dimensions,
                        "Embedding provider initialized"
                    );
                    Some(Arc::new(provider))
                }
                Err(e) => {
                    warn!(error = %e, "Failed to initialize embedding provider; knowledge will use keyword-only search");
                    None
                }
            }
        } else {
            None
        };

        let mut builder =
            KnowledgeService::builder(config.knowledge.clone()).data_dir(knowledge_data_dir);
        if let Some(ref provider) = embedding_provider {
            builder = builder.embedding_provider(Arc::clone(provider));
        }
        let knowledge_service = builder.build();

        (Arc::new(Mutex::new(knowledge_service)), embedding_provider)
    }

    /// Build a [`SkillSearch`] index from the filesystem skill store.
    ///
    /// Loads all skill manifests from `skills_dir` and indexes them for
    /// keyword search. Returns an empty index if the directory is missing
    /// or unreadable.
    fn build_skill_search_index(skills_dir: Option<&std::path::Path>) -> SkillSearch {
        let mut index = SkillSearch::new();

        let Some(dir) = skills_dir else {
            return index;
        };

        if !dir.exists() {
            return index;
        }

        let store = match y_skills::FilesystemSkillStore::new(dir) {
            Ok(s) => s,
            Err(e) => {
                warn!(error = %e, "failed to open skill store for search index");
                return index;
            }
        };

        match store.load_all() {
            Ok(manifests) => {
                let count = manifests.len();
                for manifest in manifests {
                    index.index(manifest);
                }
                if count > 0 {
                    info!(skills = count, "skill search index built");
                }
            }
            Err(e) => {
                warn!(error = %e, "failed to load skills for search index");
            }
        }

        index
    }
}

// ---------------------------------------------------------------------------
// RwLockPoolDelegator -- shared-pool delegation adapter
// ---------------------------------------------------------------------------

/// Thin wrapper: implements `AgentDelegator` by locking a shared
/// `Arc<RwLock<AgentPool>>` so that runner swaps propagate to all
/// delegation call sites.
struct RwLockPoolDelegator(Arc<RwLock<AgentPool>>);

impl std::fmt::Debug for RwLockPoolDelegator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RwLockPoolDelegator")
            .finish_non_exhaustive()
    }
}

#[async_trait::async_trait]
impl AgentDelegator for RwLockPoolDelegator {
    async fn delegate(
        &self,
        agent_name: &str,
        input: serde_json::Value,
        context_strategy: y_core::agent::ContextStrategyHint,
        session_id: Option<uuid::Uuid>,
    ) -> Result<y_core::agent::DelegationOutput, y_core::agent::DelegationError> {
        let pool = self.0.read().await;
        pool.delegate(agent_name, input, context_strategy, session_id)
            .await
    }
}

// ---------------------------------------------------------------------------
// DelegatingCompactionLlm -- bridges CompactionLlm with AgentDelegator
// ---------------------------------------------------------------------------

/// Adapter: implements [`CompactionLlm`] by delegating to the
/// `compaction-summarizer` built-in agent via [`AgentDelegator`].
///
/// The prompt text is wrapped in a JSON object `{ "prompt": "..." }` and
/// sent as the delegation input. The agent's text response is returned
/// as the summary.
struct DelegatingCompactionLlm(Arc<dyn AgentDelegator>);

#[async_trait::async_trait]
impl CompactionLlm for DelegatingCompactionLlm {
    async fn summarize(&self, prompt: &str) -> Result<String, String> {
        let input = serde_json::json!({ "prompt": prompt });
        match self
            .0
            .delegate(
                "compaction-summarizer",
                input,
                y_core::agent::ContextStrategyHint::None,
                None,
            )
            .await
        {
            Ok(output) if !output.text.trim().is_empty() => Ok(output.text),
            Ok(_) => Err("compaction-summarizer returned empty response".to_string()),
            Err(e) => Err(format!("compaction-summarizer delegation failed: {e}")),
        }
    }
}

/// Result of agent sub-system initialisation.
type AgentInitResult = (
    Arc<Mutex<AgentRegistry>>,
    Arc<RwLock<AgentPool>>,
    Arc<dyn AgentDelegator>,
    Arc<DelegationTracker>,
);

impl ServiceContainer {
    /// Initialise agent registry, pool, delegator, and wrap the delegator with diagnostics.
    ///
    /// Uses a SINGLE shared `AgentPool` behind `tokio::sync::RwLock` so that
    /// both `self.agent_pool` and `self.agent_delegator` share the same pool.
    /// When `init_agent_runner()` swaps the runner to `ServiceAgentRunner`,
    /// the change automatically affects the delegation path.
    fn init_agent_and_diagnostics(
        config_dir: Option<&std::path::Path>,
        provider_pool: &Arc<ProviderPoolImpl>,
        diagnostics: &Arc<DiagnosticsSubscriber<dyn TraceStore>>,
        broadcast_tx: tokio::sync::broadcast::Sender<DiagnosticsEvent>,
    ) -> AgentInitResult {
        let agents_dir = config_dir.map(|p| p.join("agents"));
        let mut registry = AgentRegistry::new_with_user_agents(agents_dir.as_deref());
        registry.add_template_var(
            "{{TRANSLATE_TARGET_LANGUAGE}}".to_string(),
            "English".to_string(),
        );
        let agent_registry = Arc::new(Mutex::new(registry));
        let mut agent_pool =
            AgentPool::with_registry(MultiAgentConfig::default(), Arc::clone(&agent_registry));

        let runner = Arc::new(SingleTurnRunner::new(
            Arc::clone(provider_pool) as Arc<dyn y_core::provider::ProviderPool>
        ));
        agent_pool.set_runner(runner);

        let delegation_tracker = Arc::clone(agent_pool.delegation_tracker());

        // Wrap the pool in Arc<RwLock<..>> so both self.agent_pool and the
        // delegator share the same pool instance.
        let shared_pool = Arc::new(RwLock::new(agent_pool));

        let agent_delegator: Arc<dyn AgentDelegator> =
            Arc::new(RwLockPoolDelegator(Arc::clone(&shared_pool)));
        let agent_delegator: Arc<dyn AgentDelegator> =
            Arc::new(crate::diagnostics::DiagnosticsAgentDelegator::new(
                agent_delegator,
                Arc::clone(diagnostics),
                broadcast_tx,
            ));

        (
            agent_registry,
            shared_pool,
            agent_delegator,
            delegation_tracker,
        )
    }

    /// Get a snapshot of the current provider pool.
    ///
    /// Callers receive an `Arc` clone so the pool remains valid even if
    /// a concurrent reload swaps in a new pool.
    pub async fn provider_pool(&self) -> Arc<ProviderPoolImpl> {
        Arc::clone(&*self.provider_pool.read().await)
    }

    /// Hot-reload the provider pool from a new configuration.
    ///
    /// This rebuilds all LLM provider instances and atomically swaps the
    /// pool. In-flight requests using the old `Arc` remain unaffected.
    pub async fn reload_providers(&self, pool_config: &y_provider::ProviderPoolConfig) {
        let providers = y_provider::build_providers(pool_config);
        let new_pool = Arc::new(ProviderPoolImpl::from_providers(providers, pool_config));
        let mut guard = self.provider_pool.write().await;
        *guard = new_pool;

        // The `tool_calling.prompt_based` flag is now set per-request in
        // agent_service.rs based on the provider selected for that turn.
        // No static sync needed here.

        info!(
            providers = pool_config.providers.len(),
            "Provider pool hot-reloaded"
        );
    }

    /// Hot-reload the guardrail configuration.
    ///
    /// Atomically replaces the `GuardrailManager` config so that subsequent
    /// turns use the new values (e.g. `max_tool_iterations`).
    pub fn reload_guardrails(&self, new_config: y_guardrails::GuardrailConfig) {
        self.guardrail_manager.reload_config(new_config);
    }

    /// Hot-reload the session configuration.
    pub fn reload_session(&self, new_config: y_session::SessionConfig) {
        self.session_manager.reload_config(new_config);
    }

    /// Hot-reload the runtime configuration.
    pub fn reload_runtime(&self, new_config: y_runtime::RuntimeConfig) {
        self.runtime_manager.reload_config(new_config);
    }

    /// Hot-reload the tool registry configuration.
    pub fn reload_tools(&self, new_config: y_tools::ToolRegistryConfig) {
        self.tool_registry.reload_config(new_config);
    }

    /// Hot-reload the browser tool configuration.
    ///
    /// Looks up the registered `browser` tool by name and calls its
    /// `reload_config` method. No-op if the browser tool is not registered.
    pub async fn reload_browser(&self, new_config: y_browser::BrowserConfig) {
        if let Some(tool) = self
            .tool_registry
            .get_tool(&y_core::types::ToolName::from_string("Browser"))
            .await
        {
            // Downcast the Arc<dyn Tool> to BrowserTool.
            if let Some(bt) = tool.as_any().downcast_ref::<y_browser::BrowserTool>() {
                bt.reload_config(new_config);
            } else {
                info!("browser tool found but downcast failed; skipping browser config reload");
            }
        }
    }

    /// Hot-reload the hook system configuration.
    ///
    /// Rebuilds the handler executor, updates timeouts and event bus capacity.
    /// Existing middleware registrations are preserved.
    ///
    /// # Panics
    ///
    /// Panics if the internal `hook_system` `RwLock` is poisoned (a prior
    /// holder panicked while holding the write lock).
    pub fn reload_hooks(&self, new_config: &y_hooks::HookConfig) {
        let mut hs = self
            .hook_system
            .write()
            .expect("hook_system RwLock poisoned");
        hs.reload_config(new_config);
    }

    /// Release all in-memory state associated with a session.
    ///
    /// Called when a session is deleted so that per-session `HashMaps` do not
    /// grow unboundedly over the application lifetime.
    pub async fn cleanup_session_state(&self, session_id: &SessionId) {
        self.pruning_watermarks.write().await.remove(session_id);
        self.session_permission_modes
            .write()
            .await
            .remove(session_id);
        crate::rewind::RewindService::cleanup_session(&self.file_history_managers, session_id)
            .await;
        {
            let mut interactions = self.pending_interactions.lock().await;
            interactions.remove(&session_id.0);
        }
        {
            let mut permissions = self.pending_permissions.lock().await;
            permissions.remove(&session_id.0);
        }
        info!(session = %session_id.0, "cleaned up in-memory state for deleted session");
    }

    // -- Agent management (delegated to AgentManagementService) ----------------

    /// Hot-reload agent definitions from the agents directory.
    ///
    /// Returns `(loaded, errored)` counts.
    pub async fn reload_agents(&self) -> (usize, usize) {
        crate::agent_management::AgentManagementService::reload_agents(self).await
    }

    /// Register a single agent from raw TOML content at runtime.
    ///
    /// Returns the registered agent's ID on success.
    pub async fn register_agent_from_toml(&self, toml_content: &str) -> Result<String, String> {
        crate::agent_management::AgentManagementService::register_agent_from_toml(
            self,
            toml_content,
        )
        .await
    }

    /// Populate the callable agents text at startup.
    pub async fn init_callable_agents_text(&self) {
        crate::agent_management::AgentManagementService::init_callable_agents_text(self).await;
    }

    /// Save an agent definition from raw TOML content to the agents directory.
    pub async fn save_agent(&self, id: &str, toml_content: &str) -> Result<(), String> {
        crate::agent_management::AgentManagementService::save_agent(self, id, toml_content).await
    }

    /// Reset an overridden built-in agent to its original definition.
    pub async fn reset_agent(&self, id: &str) -> Result<(), String> {
        crate::agent_management::AgentManagementService::reset_agent(self, id).await
    }

    /// Read the raw TOML source for an agent definition.
    ///
    /// Returns `(path, content, is_user_file)`.
    pub async fn get_agent_source(&self, id: &str) -> Result<(String, String, bool), String> {
        crate::agent_management::AgentManagementService::get_agent_source(self, id).await
    }

    /// Hot-reload prompt section files from disk.
    ///
    /// Looks up the `BuildSystemPromptProvider` in the context pipeline
    /// and triggers a store reload so that saved prompt edits take effect
    /// immediately (without restarting the application).
    pub async fn reload_prompts(&self) {
        if let Some(provider) = self
            .context_pipeline
            .get_provider::<BuildSystemPromptProvider>("build_system_prompt")
        {
            provider.reload_store().await;
        } else {
            warn!("BuildSystemPromptProvider not found in context pipeline; prompt reload skipped");
        }
    }

    /// Hot-reload the knowledge service configuration.
    ///
    /// Acquires the `KnowledgeService` mutex and replaces its config so
    /// that subsequent ingestion operations use the new parameters.
    pub async fn reload_knowledge(&self, new_config: y_knowledge::config::KnowledgeConfig) {
        let mut ks = self.knowledge_service.lock().await;
        ks.reload_config(new_config);
    }

    /// Construct a [`SkillIngestionService`] wired to this container's
    /// agent delegator.
    ///
    /// The caller supplies the skill registry; the delegator comes from
    /// the container.
    pub fn skill_ingestion_service(
        &self,
        registry: Arc<RwLock<SkillRegistryImpl>>,
    ) -> SkillIngestionService {
        SkillIngestionService::new(Arc::clone(&self.agent_delegator), registry)
    }

    /// Import a skill from a source path using the service-layer workflow.
    pub async fn import_skill_from_path(
        &self,
        store_path: &std::path::Path,
        source_path: &std::path::Path,
        sanitize: bool,
    ) -> Result<SkillImportOutcome, String> {
        import_skill_from_path(
            Arc::clone(&self.agent_delegator),
            store_path,
            source_path,
            sanitize,
        )
        .await
    }

    /// Two-phase initialisation: swap the agent runner from `SingleTurnRunner`
    /// to `ServiceAgentRunner` so that sub-agents use the unified
    /// `AgentService::execute()` loop with multi-turn tool calling.
    ///
    /// Because `agent_pool` is shared (via `RwLockPoolDelegator`) with
    /// `agent_delegator`, this single swap upgrades both delegation and
    /// direct pool access paths.
    ///
    /// Must be called **after** the container has been wrapped in `Arc`.
    pub async fn init_agent_runner(self: &Arc<Self>) {
        let runner = Arc::new(crate::agent_service::ServiceAgentRunner::new(Arc::clone(
            self,
        )));
        // The agent_pool held by the container is behind a tokio::sync::RwLock.
        // Since the delegator shares this same pool (via RwLockPoolDelegator),
        // the runner swap automatically takes effect for all delegation calls.
        self.agent_pool.write().await.set_runner(runner);
        tracing::info!("ServiceAgentRunner initialised for sub-agent delegation");
    }

    /// Two-phase initialisation: inject a `WorkflowDispatcher` into the
    /// `SchedulerManager` so that fired triggers and manual workflow
    /// executions run real DAG-based workflows.
    ///
    /// Uses the same pattern as [`init_agent_runner`](Self::init_agent_runner):
    /// must be called **after** the container has been wrapped in `Arc`.
    pub async fn init_workflow_dispatcher(self: &Arc<Self>) {
        let dispatcher = Arc::new(crate::orchestrator_dispatcher::OrchestratorDispatcher::new(
            Arc::clone(self),
        ));
        self.scheduler_manager.set_dispatcher(dispatcher).await;
        tracing::info!("WorkflowDispatcher initialised for real workflow execution");
    }

    /// Start the background scheduler loop used by GUI/TUI/server runtimes.
    ///
    /// This is intentionally separate from [`from_config`](Self::from_config)
    /// because short-lived commands should not keep background tasks alive.
    pub async fn init_scheduler(self: &Arc<Self>) {
        self.scheduler_manager
            .start(BACKGROUND_SCHEDULER_TICK_INTERVAL)
            .await;
        tracing::info!(
            tick_interval_ms = BACKGROUND_SCHEDULER_TICK_INTERVAL.as_millis(),
            "SchedulerManager started for background automation"
        );
    }

    /// Start all background services required by long-lived frontends.
    ///
    /// GUI, TUI, and the embedded HTTP server share the same lifecycle:
    /// upgrade the agent runner, inject the workflow dispatcher, start the
    /// scheduler loop, then refresh callable-agent prompt text.
    pub async fn start_background_services(self: &Arc<Self>) {
        self.init_agent_runner().await;
        self.init_workflow_dispatcher().await;
        self.init_scheduler().await;
        self.init_callable_agents_text().await;
        self.init_knowledge_llm_services().await;
        crate::mcp_service::McpService::init_mcp_connections(self).await;
        crate::mcp_service::McpService::register_mcp_tools(self).await;
        crate::mcp_service::McpService::start_mcp_event_consumer(self).await;
    }

    /// Wire LLM-backed knowledge services (tag generator, metadata
    /// extractor, summary generator) into the `KnowledgeService`.
    ///
    /// Must be called **after** `init_agent_runner` so that the delegator
    /// uses the full `ServiceAgentRunner` (required for the summarizer
    /// agent which uses multi-turn `FileRead` tool calling).
    async fn init_knowledge_llm_services(self: &Arc<Self>) {
        use crate::knowledge_service::{
            AgentMetadataExtractor, AgentSummaryGenerator, AgentTagGenerator,
        };

        let delegator = Arc::clone(&self.agent_delegator);

        let tag_gen: Arc<dyn y_knowledge::tagger::TagGenerator> =
            Arc::new(AgentTagGenerator::new(Arc::clone(&delegator)));
        let meta_ext: Arc<dyn y_knowledge::tagger::MetadataExtractor> =
            Arc::new(AgentMetadataExtractor::new(Arc::clone(&delegator)));
        let summary_gen: Arc<dyn y_knowledge::tagger::SummaryGenerator> =
            Arc::new(AgentSummaryGenerator::new(delegator));

        let mut ks = self.knowledge_service.lock().await;
        ks.set_tag_generator(tag_gen);
        ks.set_metadata_extractor(meta_ext);
        ks.set_summary_generator(summary_gen);

        tracing::info!("Knowledge LLM services wired (tagger, metadata, summarizer)");
    }

    /// Spawn per-provider tasks that drain metrics events and persist them.
    ///
    /// Each provider gets its own channel; we spawn one task per provider
    /// that loops until the sender is dropped.
    async fn run_metrics_event_consumers(
        receivers: Vec<(
            String,
            String,
            tokio::sync::mpsc::UnboundedReceiver<y_provider::MetricsEvent>,
        )>,
        store: y_storage::SqliteProviderMetricsStore,
    ) {
        let mut handles = Vec::with_capacity(receivers.len());
        for (provider_id, model, mut rx) in receivers {
            let store = store.clone();
            let pid = provider_id;
            let mdl = model;
            handles.push(tokio::spawn(async move {
                while let Some(event) = rx.recv().await {
                    let db_event = y_storage::ProviderMetricsEvent {
                        provider_id: pid.clone(),
                        model: mdl.clone(),
                        is_error: event.is_error,
                        input_tokens: u64::from(event.input_tokens),
                        output_tokens: u64::from(event.output_tokens),
                        cost_micros: event.cost_micros,
                    };
                    if let Err(e) = store.record_event(&db_event).await {
                        tracing::warn!(
                            provider_id = %pid,
                            error = %e,
                            "failed to persist metrics event"
                        );
                    }
                }
            }));
        }
        // Wait for all consumers to exit (happens when provider pool is dropped).
        for h in handles {
            let _ = h.await;
        }
    }
}

// ---------------------------------------------------------------------------
// Core-tools pre-activation and summary generation
// ---------------------------------------------------------------------------

/// Tools always included in `ChatRequest.tools` -- every LLM call has these schemas.
///
/// These are the most frequently used tools; including them avoids an extra
/// `ToolSearch` round-trip for common operations.
pub(crate) const ESSENTIAL_TOOL_NAMES: &[&str] = &[
    "ToolSearch",
    "FileRead",
    "FileWrite",
    "ShellExec",
    "Task",
    "WebFetch",
    "AskUser",
];

/// Tools pre-activated as always-active (never LRU-evicted) but NOT in
/// `ChatRequest.tools` by default. The LLM sees them in "Available Tools"
/// and can use `ToolSearch` to load full schemas on demand.
const DISCOVERABLE_TOOL_NAMES: &[&str] = &["Browser"];

/// Pre-activate essential and discoverable tools as always-active in the
/// activation set. Both groups are never LRU-evicted; the difference is
/// that only `ESSENTIAL_TOOL_NAMES` have schemas sent in every API call.
async fn pre_activate_core_tools(
    registry: &ToolRegistryImpl,
    activation_set: &Arc<RwLock<ToolActivationSet>>,
) {
    let mut set = activation_set.write().await;
    for &name in ESSENTIAL_TOOL_NAMES
        .iter()
        .chain(DISCOVERABLE_TOOL_NAMES.iter())
    {
        if let Some(def) = registry.get_definition(&ToolName::from_string(name)).await {
            set.activate(def);
            set.set_always_active(&ToolName::from_string(name));
        }
    }
}

/// Build a Markdown tool summary table from a slice of definitions.
///
/// Shared implementation backing both [`build_core_tools_summary`] and
/// [`build_agent_tools_summary`]. Callers supply a header preamble
/// (lines before the table) and a footer reminder (line after the table).
fn build_tool_summary_table(
    defs: &[&y_core::tool::ToolDefinition],
    header_lines: &[&str],
    footer: &str,
) -> String {
    let mut sorted = defs.to_vec();
    sorted.sort_by_key(|d| d.name.as_str().to_string());

    let mut lines: Vec<String> = header_lines.iter().map(|s| (*s).to_string()).collect();
    lines.push("| Tool | Description | Usage |".to_string());
    lines.push("|------|-------------|-------|".to_string());

    for def in &sorted {
        let desc = def
            .description
            .split('.')
            .next()
            .unwrap_or(&def.description)
            .trim();
        let usage = extract_usage_hint(&def.parameters);
        lines.push(format!("| {} | {} | {} |", def.name.as_str(), desc, usage));
    }

    lines.push(String::new());
    lines.push(footer.to_string());
    lines.join("\n")
}

/// Generate a compact summary of essential tools for prompt injection.
///
/// Produces a Markdown table with tool name, first-sentence description,
/// and a usage hint (required args or available params), followed by a
/// usage reminder. Only includes `ESSENTIAL_TOOL_NAMES` -- the tools whose
/// full schemas are always sent in the API call. Called once at startup.
fn build_core_tools_summary(set: &ToolActivationSet) -> String {
    let essential: std::collections::HashSet<&str> = ESSENTIAL_TOOL_NAMES.iter().copied().collect();
    let defs: Vec<&y_core::tool::ToolDefinition> = set
        .always_active_definitions()
        .into_iter()
        .filter(|d| essential.contains(d.name.as_str()))
        .collect();

    build_tool_summary_table(
        &defs,
        &[
            "## Core Tools (always available)\n",
            "You can call these tools directly without searching:\n",
        ],
        "IMPORTANT: Use ONLY these exact tool names. \
         Do NOT invent tool names like 'ls', 'cat', 'grep', or 'mkdir'. \
         For shell operations not covered above, use ShellExec.",
    )
}

/// Generate a compact tools summary for a sub-agent from filtered definitions.
///
/// Same Markdown table format as [`build_core_tools_summary`] but operates on
/// an arbitrary slice of [`ToolDefinition`]s (typically the agent's allowed
/// tools after filtering). Returns an empty string when `defs` is empty.
pub(crate) fn build_agent_tools_summary(defs: &[y_core::tool::ToolDefinition]) -> String {
    if defs.is_empty() {
        return String::new();
    }
    let refs: Vec<&y_core::tool::ToolDefinition> = defs.iter().collect();
    build_tool_summary_table(
        &refs,
        &["## Available Tools\n"],
        "Use ONLY these tool names. Do NOT invent tool names.",
    )
}

/// Extract a compact usage hint from a JSON Schema `parameters` value.
///
/// When the schema has an explicit `"required"` array, the hint shows only
/// those params with placeholders derived from descriptions, e.g.
/// `{"path": "<File_path_to>"}`.
///
/// When no required params exist (common for multi-mode tools like
/// `ToolSearch` and `WebFetch`), the hint falls back to listing all
/// available properties so the LLM still knows what it can pass, e.g.
/// `Pass one of: query, category, tool`.
fn extract_usage_hint(params: &serde_json::Value) -> String {
    let required: Vec<&str> = params
        .get("required")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();

    let properties = params.get("properties");

    // Case 1: explicit required args -- show compact placeholders.
    if !required.is_empty() {
        let pairs: Vec<String> = required
            .iter()
            .map(|name| {
                let placeholder = properties
                    .and_then(|p| p.get(*name))
                    .and_then(|prop| {
                        prop.get("description").and_then(|d| d.as_str()).map(|d| {
                            let hint: String =
                                d.split_whitespace().take(3).collect::<Vec<_>>().join("_");
                            format!("<{hint}>")
                        })
                    })
                    .unwrap_or_else(|| "<value>".to_string());
                format!("\"{name}\": \"{placeholder}\"")
            })
            .collect();
        return format!("{{{{{}}}}}", pairs.join(", "));
    }

    // Case 2: no required args -- list available properties as hints.
    if let Some(props) = properties.and_then(|p| p.as_object()) {
        if props.is_empty() {
            return "{}".to_string();
        }
        let names: Vec<&str> = props.keys().map(String::as_str).collect();
        return format!("Pass one of: {}", names.join(", "));
    }

    "{}".to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_container_creates_all_services() {
        let mut config = ServiceConfig::default();
        config.storage = y_storage::StorageConfig::in_memory();

        let result = ServiceContainer::from_config(&config).await;
        assert!(result.is_ok(), "wiring with default config should succeed");

        let sc = result.unwrap();
        let _ = sc.provider_pool().await;
        let _ = &sc.session_manager;
        let _ = &sc.hook_system;
        let _ = &sc.tool_registry;
        let _ = &sc.runtime_manager;
        let _ = &sc.context_pipeline;
        let _ = &sc.guardrail_manager;
        let _ = &sc.agent_pool;
        let _ = &sc.prompt_context;
    }

    #[tokio::test]
    async fn test_container_registers_middleware() {
        let mut config = ServiceConfig::default();
        config.storage = y_storage::StorageConfig::in_memory();

        let sc = ServiceContainer::from_config(&config).await.unwrap();
        let _tool_guard = sc.guardrail_manager.tool_guard();
        let _loop_detector = sc.guardrail_manager.loop_detector();
        let _llm_guard = sc.guardrail_manager.llm_guard();
    }

    #[test]
    fn test_build_providers_skips_missing_key() {
        let pool_config = y_provider::config::ProviderPoolConfig {
            providers: vec![y_provider::config::ProviderConfig {
                id: "test-no-key".into(),
                provider_type: "openai".into(),
                model: "gpt-4".into(),
                enabled: true,
                tags: vec![],
                capabilities: vec![],
                max_concurrency: 5,
                context_window: 128_000,
                cost_per_1k_input: 0.0,
                cost_per_1k_output: 0.0,
                api_key: None,
                api_key_env: Some("Y_AGENT_NONEXISTENT_KEY_12345".into()),
                base_url: None,
                headers: std::collections::HashMap::new(),
                http_protocol: y_provider::config::HttpProtocol::Http1,
                temperature: None,
                top_p: None,
                tool_calling_mode: None,
                icon: None,
            }],
            ..Default::default()
        };
        let providers = y_provider::build_providers(&pool_config);
        assert!(providers.is_empty());
    }

    #[test]
    fn test_build_providers_skips_unsupported_type() {
        temp_env::with_var("Y_AGENT_TEST_SVC_KEY", Some("test-key"), || {
            let pool_config = y_provider::config::ProviderPoolConfig {
                providers: vec![y_provider::config::ProviderConfig {
                    id: "test-unsupported".into(),
                    provider_type: "unsupported_backend".into(),
                    model: "some-model".into(),
                    enabled: true,
                    tags: vec![],
                    capabilities: vec![],
                    max_concurrency: 5,
                    context_window: 128_000,
                    cost_per_1k_input: 0.0,
                    cost_per_1k_output: 0.0,
                    api_key: None,
                    api_key_env: Some("Y_AGENT_TEST_SVC_KEY".into()),
                    base_url: None,
                    headers: std::collections::HashMap::new(),
                    http_protocol: y_provider::config::HttpProtocol::Http1,
                    temperature: None,
                    top_p: None,
                    tool_calling_mode: None,
                    icon: None,
                }],
                ..Default::default()
            };
            let providers = y_provider::build_providers(&pool_config);
            assert!(providers.is_empty());
        });
    }

    #[test]
    fn test_build_providers_openai_compat_alias() {
        temp_env::with_var("Y_AGENT_TEST_COMPAT_KEY", Some("sk-test"), || {
            let pool_config = y_provider::config::ProviderPoolConfig {
                providers: vec![y_provider::config::ProviderConfig {
                    id: "my-compat".into(),
                    provider_type: "openai-compat".into(),
                    model: "local-model".into(),
                    enabled: true,
                    tags: vec![],
                    capabilities: vec![],
                    max_concurrency: 2,
                    context_window: 32_000,
                    cost_per_1k_input: 0.0,
                    cost_per_1k_output: 0.0,
                    api_key: None,
                    api_key_env: Some("Y_AGENT_TEST_COMPAT_KEY".into()),
                    base_url: Some("http://localhost:8080/v1".into()),
                    headers: std::collections::HashMap::new(),
                    http_protocol: y_provider::config::HttpProtocol::Http1,
                    temperature: None,
                    top_p: None,
                    tool_calling_mode: None,
                    icon: None,
                }],
                ..Default::default()
            };
            let providers = y_provider::build_providers(&pool_config);
            assert_eq!(
                providers.len(),
                1,
                "openai-compat should build exactly one provider"
            );
        });
    }

    #[test]
    fn test_build_providers_deepseek_alias() {
        temp_env::with_var("Y_AGENT_TEST_DEEPSEEK_KEY", Some("sk-ds-test"), || {
            let pool_config = y_provider::config::ProviderPoolConfig {
                providers: vec![y_provider::config::ProviderConfig {
                    id: "deepseek-chat".into(),
                    provider_type: "deepseek".into(),
                    model: "deepseek-chat".into(),
                    enabled: true,
                    tags: vec![],
                    capabilities: vec![],
                    max_concurrency: 3,
                    context_window: 64_000,
                    cost_per_1k_input: 0.0,
                    cost_per_1k_output: 0.0,
                    api_key: None,
                    api_key_env: Some("Y_AGENT_TEST_DEEPSEEK_KEY".into()),
                    base_url: None,
                    headers: std::collections::HashMap::new(),
                    http_protocol: y_provider::config::HttpProtocol::Http1,
                    temperature: None,
                    top_p: None,
                    tool_calling_mode: None,
                    icon: None,
                }],
                ..Default::default()
            };
            let providers = y_provider::build_providers(&pool_config);
            assert_eq!(
                providers.len(),
                1,
                "deepseek should build exactly one provider"
            );
        });
    }

    #[tokio::test]
    async fn test_container_registers_context_providers() {
        let mut config = ServiceConfig::default();
        config.storage = y_storage::StorageConfig::in_memory();

        let sc = ServiceContainer::from_config(&config).await.unwrap();
        assert_eq!(sc.context_pipeline.provider_count(), 4);
    }

    #[tokio::test]
    async fn test_container_initializes_embedding_enabled_knowledge_wiring() {
        let mut config = ServiceConfig::default();
        config.storage = y_storage::StorageConfig::in_memory();
        config.knowledge.embedding_enabled = true;
        config.knowledge.embedding_api_key = "test-key".to_string();

        let sc = ServiceContainer::from_config(&config).await.unwrap();
        let definitions = sc.tool_registry.get_all_definitions().await;
        assert!(definitions
            .iter()
            .any(|def| def.name.as_str() == "KnowledgeSearch"));

        let knowledge_service = sc.knowledge_service.lock().await;
        assert!(knowledge_service.embedding_provider().is_some());
    }

    #[tokio::test]
    async fn test_skill_ingestion_service_factory() {
        let mut config = ServiceConfig::default();
        config.storage = y_storage::StorageConfig::in_memory();

        let sc = ServiceContainer::from_config(&config).await.unwrap();
        let registry = Arc::new(RwLock::new(y_skills::SkillRegistryImpl::new()));
        let _service = sc.skill_ingestion_service(registry);
        // Construction succeeds -- delegator is correctly wired.
    }

    #[tokio::test]
    async fn test_start_background_services_starts_scheduler() {
        let mut config = ServiceConfig::default();
        config.storage = y_storage::StorageConfig::in_memory();

        let sc = Arc::new(ServiceContainer::from_config(&config).await.unwrap());
        assert!(!sc.scheduler_manager.is_running());

        sc.start_background_services().await;
        assert!(sc.scheduler_manager.is_running());

        sc.scheduler_manager.stop().await;
        assert!(!sc.scheduler_manager.is_running());
    }

    // -- build_agent_tools_summary tests --

    fn make_tool_def(
        name: &str,
        desc: &str,
        params: serde_json::Value,
    ) -> y_core::tool::ToolDefinition {
        y_core::tool::ToolDefinition {
            name: y_core::types::ToolName::from_string(name),
            description: desc.to_string(),
            help: None,
            parameters: params,
            result_schema: None,
            category: y_core::tool::ToolCategory::Shell,
            tool_type: y_core::tool::ToolType::BuiltIn,
            capabilities: Default::default(),
            is_dangerous: false,
        }
    }

    #[test]
    fn test_build_agent_tools_summary_empty() {
        let summary = super::build_agent_tools_summary(&[]);
        assert!(summary.is_empty());
    }

    #[test]
    fn test_build_agent_tools_summary_single_tool() {
        let defs = vec![make_tool_def(
            "ShellExec",
            "Execute a shell command. Runs in sandbox.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {"type": "string", "description": "Shell command to run"}
                },
                "required": ["command"]
            }),
        )];
        let summary = super::build_agent_tools_summary(&defs);
        assert!(summary.contains("## Available Tools"));
        assert!(summary.contains("| ShellExec |"));
        assert!(summary.contains("Execute a shell command"));
        assert!(summary.contains("Use ONLY these tool names"));
    }

    #[test]
    fn test_build_agent_tools_summary_sorted() {
        let defs = vec![
            make_tool_def("FileWrite", "Write a file.", serde_json::json!({})),
            make_tool_def("Browser", "Open a browser.", serde_json::json!({})),
            make_tool_def("ShellExec", "Execute shell.", serde_json::json!({})),
        ];
        let summary = super::build_agent_tools_summary(&defs);
        let browser_pos = summary.find("Browser").unwrap();
        let file_write_pos = summary.find("FileWrite").unwrap();
        let shell_exec_pos = summary.find("ShellExec").unwrap();
        assert!(browser_pos < file_write_pos);
        assert!(file_write_pos < shell_exec_pos);
    }
}
