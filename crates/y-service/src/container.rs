//! Central dependency container — shared by all presentation layers.
//!
//! Mirrors the former `AppServices` from `y-cli/wire.rs`, but lives in the
//! service layer so CLI, TUI, and future Web API can all construct one.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::sync::{Mutex, RwLock};
use tracing::{info, warn};

use y_agent::{AgentPool, AgentRegistry, DelegationTracker, MultiAgentConfig};
use y_context::{
    BuildSystemPromptProvider, BunVenvPromptInfo, CompactionEngine, ContextPipeline,
    InjectContextStatus, InjectSkills, InjectTools, KnowledgeContextProvider, PruningEngine,
    PythonVenvPromptInfo, SystemPromptConfig, VenvPromptInfo,
};
use y_core::agent::AgentDelegator;
use y_core::provider::LlmProvider;
use y_core::types::{SessionId, ToolName};
use y_diagnostics::{DiagnosticsSubscriber, TraceStore};
use y_guardrails::GuardrailManager;
use y_hooks::HookSystem;
use y_prompt::{builtin_section_store_with_overrides, default_template, PromptContext};
use y_provider::providers::anthropic::AnthropicProvider;
use y_provider::providers::azure::AzureOpenAiProvider;
use y_provider::providers::gemini::GeminiProvider;
use y_provider::providers::ollama::OllamaProvider;
use y_provider::providers::openai::OpenAiProvider;
use y_provider::ProviderPoolImpl;
use y_provider::SingleTurnRunner;
use y_runtime::{RuntimeManager, VenvManager};
use y_session::{ChatCheckpointManager, SessionManager};
use y_skills::{SkillRegistryImpl, SkillSearch};
use y_storage::{
    SqliteChatCheckpointStore, SqliteChatMessageStore, SqliteProviderMetricsStore,
    SqliteSessionStore, SqliteWorkflowStore,
};
use y_tools::{ToolActivationSet, ToolRegistryImpl, ToolTaxonomy};

use crate::config::ServiceConfig;

use crate::knowledge_service::KnowledgeService;
use crate::skill_ingestion::SkillIngestionService;

/// Embedded default taxonomy TOML (compiled into binary).
const DEFAULT_TAXONOMY_TOML: &str = include_str!("../../../config/tool_taxonomy.toml");

/// Default `ToolActivationSet` ceiling.
const ACTIVATION_SET_CEILING: usize = 20;

/// All wired application services, constructed from [`ServiceConfig`].
///
/// Some fields (e.g., `runtime_manager`, `hook_system`, `guardrail_manager`)
/// are not yet consumed by `ChatService` but are reserved for middleware
/// pipeline integration (see TODO(middleware) comments in `chat.rs`).
#[allow(dead_code)]
pub struct ServiceContainer {
    /// Provider pool for LLM communication.
    /// Wrapped in `RwLock` to support hot-reload of provider configuration.
    provider_pool: RwLock<Arc<ProviderPoolImpl>>,

    /// Session manager for session lifecycle.
    pub session_manager: SessionManager,

    /// Unified hook system (registry, event bus, middleware chains, handler executor).
    pub hook_system: HookSystem,

    /// Tool registry for tool management.
    pub tool_registry: ToolRegistryImpl,

    /// Runtime manager for tool execution environments.
    pub runtime_manager: Arc<RuntimeManager>,

    /// Context pipeline for prompt assembly.
    pub context_pipeline: ContextPipeline,

    /// Guardrail manager for security middleware.
    pub guardrail_manager: GuardrailManager,

    /// Agent registry for definition management.
    pub agent_registry: Mutex<AgentRegistry>,

    /// Agent pool for runtime instance management.
    /// Shared via `Arc` with `MutexPoolDelegator` so that runner upgrades
    /// (via `init_agent_runner`) affect both direct pool access and delegation.
    pub agent_pool: Arc<Mutex<AgentPool>>,

    /// Agent delegator for delegating tasks to agents (wired through `AgentPool` + `SingleTurnRunner`).
    /// Once `init_agent_runner` is called post-construction, sub-agents use `ServiceAgentRunner`.
    pub agent_delegator: Arc<dyn AgentDelegator>,

    /// Shared delegation tracker: records active delegations from `agent_delegator`
    /// so observability can see them even though they bypass pool instance tracking.
    pub delegation_tracker: Arc<DelegationTracker>,

    /// Workflow store for persistent workflow templates.
    pub workflow_store: SqliteWorkflowStore,

    /// Shared prompt context, updated per-turn by the chat loop.
    pub prompt_context: Arc<RwLock<PromptContext>>,

    /// Diagnostics subscriber for trace recording.
    pub diagnostics: Arc<DiagnosticsSubscriber<dyn TraceStore>>,

    /// Chat checkpoint manager for turn-level rollback.
    pub chat_checkpoint_manager: ChatCheckpointManager,

    /// Session-scoped tool activation set (LRU, ceiling 20).
    pub tool_activation_set: Arc<RwLock<ToolActivationSet>>,

    /// Hierarchical tool taxonomy for prompt-based discovery.
    pub tool_taxonomy: Arc<ToolTaxonomy>,

    /// Chat message store for session history tree (Phase 2).
    pub chat_message_store: Arc<SqliteChatMessageStore>,

    /// Knowledge base service (ingestion, retrieval, embedding).
    ///
    /// Uses `tokio::sync::Mutex` so the GUI layer can share this `Arc` and
    /// hold the lock across `.await` points (e.g. `ingest().await`).
    pub knowledge_service: Arc<Mutex<KnowledgeService>>,

    /// Pruning engine — removes failed tool call branches and summarizes
    /// completed multi-step sequences. Wired with the `agent_delegator`
    /// so progressive pruning can delegate to the `pruning-summarizer` agent.
    pub pruning_engine: PruningEngine,

    /// Compaction engine — summarizes older history to reclaim context space.
    pub compaction_engine: CompactionEngine,

    /// Compaction trigger threshold as a percentage of `context_window`
    /// (e.g. 85 = compact when usage exceeds 85% of model context window).
    pub compaction_threshold_pct: u32,

    /// Per-session token watermarks for delta-based pruning.
    /// Tracks the total token count at the time pruning last ran.
    /// Pruning only triggers when `current_tokens - watermark >= token_threshold`.
    pub pruning_watermarks: RwLock<HashMap<SessionId, u32>>,

    /// Provider metrics event log store for persistence across restarts.
    pub provider_metrics_store: SqliteProviderMetricsStore,

    /// Skill search index for unified `tool_search` capability discovery.
    ///
    /// Pre-loaded from the skills directory at startup. Wrapped in `RwLock`
    /// so it can be refreshed when skills are added/removed.
    pub skill_search: RwLock<SkillSearch>,
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
        // 1. Storage layer — SQLite pool.
        let pool = y_storage::create_pool(&config.storage)
            .await
            .context("failed to create SQLite pool")?;

        // 1b. Run embedded SQLite migrations.
        y_storage::migration::run_embedded_migrations(&pool)
            .await
            .context("failed to run SQLite migrations")?;

        // 2. Session store from pool.
        let session_store: Arc<dyn y_core::session::SessionStore> =
            Arc::new(SqliteSessionStore::new(pool.clone()));

        // 3. Provider pool.
        let providers = build_providers_from_config(&config.providers);
        let provider_pool = Arc::new(ProviderPoolImpl::from_providers(
            providers,
            &config.providers,
        ));

        // 4. Transcript stores.
        let transcript_store: Arc<dyn y_core::session::TranscriptStore> = Arc::new(
            y_storage::JsonlTranscriptStore::new(&config.storage.transcript_dir),
        );
        let display_transcript_store: Arc<dyn y_core::session::DisplayTranscriptStore> = Arc::new(
            y_storage::JsonlDisplayTranscriptStore::new(&config.storage.transcript_dir),
        );

        // 5. Session manager.
        let session_manager = SessionManager::new(
            Arc::clone(&session_store),
            Arc::clone(&transcript_store),
            Arc::clone(&display_transcript_store),
            config.session.clone(),
        );

        // 5b. Chat checkpoint manager.
        let chat_checkpoint_store = Arc::new(SqliteChatCheckpointStore::new(pool.clone()));
        let chat_checkpoint_manager = ChatCheckpointManager::new(
            Arc::clone(&transcript_store),
            Arc::clone(&display_transcript_store),
            chat_checkpoint_store,
            Arc::clone(&session_store),
        );

        // 5c. Chat message store (Phase 2 — session history tree).
        let chat_message_store = Arc::new(SqliteChatMessageStore::new(pool.clone()));

        // 6. Hook system.
        #[allow(unused_mut)]
        let mut hook_system = HookSystem::new(&config.hooks);

        // 6b. Inject LLM runner for prompt hooks (feature-gated).
        #[cfg(all(feature = "hook_handlers", feature = "llm_hooks"))]
        {
            use y_core::provider::ProviderPool as _;
            let llm_runner = Arc::new(y_provider::ProviderPoolHookLlmRunner::new(Arc::new(
                provider_pool.clone(),
            )
                as Arc<dyn y_core::provider::ProviderPool>));
            hook_system.set_llm_runner(llm_runner);
            info!("Prompt hook LLM runner injected");
        }

        // 7. Guardrails.
        let guardrail_manager = GuardrailManager::new(config.guardrails.clone());

        // 7b. Knowledge service + embedding provider (initialised early so the
        //     knowledge_search tool can be registered as part of step 8).
        let (knowledge_service, embedding_provider) = Self::init_knowledge_service(config);

        // 8. Tool registry.
        let tool_registry = ToolRegistryImpl::new(config.tools.clone());

        // Knowledge handle for the knowledge_search tool.
        // Built here so the tool is registered in the service layer (not each
        // presentation layer). The same handle is shared with
        // KnowledgeContextProvider below.
        let kb_handle = {
            let ks = knowledge_service.lock().await;
            ks.knowledge_handle()
        };
        y_tools::builtin::register_builtin_tools(
            &tool_registry,
            config.browser.clone(),
            Some(kb_handle),
        )
        .await;

        // 8b. Tool taxonomy (loaded from embedded TOML).
        let tool_taxonomy = Arc::new(
            ToolTaxonomy::from_toml(DEFAULT_TAXONOMY_TOML).unwrap_or_else(|e| {
                warn!(error = %e, "failed to load tool taxonomy; using empty");
                ToolTaxonomy::from_toml(
                    r#"
[categories.meta]
label = "Meta"
description = "Tool management"
tools = ["tool_search"]
"#,
                )
                .expect("fallback taxonomy")
            }),
        );

        // 8c. Tool activation set.
        let tool_activation_set =
            Arc::new(RwLock::new(ToolActivationSet::new(ACTIVATION_SET_CEILING)));

        // Pre-activate all built-in tools as always-active.
        // The core-tools summary is generated from these definitions so that
        // the prompt is never out of sync with the actual tool registry.
        pre_activate_core_tools(&tool_registry, &tool_activation_set).await;

        // 9. Runtime manager.
        let runtime_manager = Arc::new(RuntimeManager::new(config.runtime.clone(), None));

        // 9b. Initialise virtual environments (uv / bun).
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

        // 10. Context pipeline.
        let prompt_context = Arc::new(RwLock::new(PromptContext::default()));
        let mut context_pipeline = ContextPipeline::new();
        {
            let mut sys_prompt_provider = BuildSystemPromptProvider::with_venv_info(
                default_template(),
                builtin_section_store_with_overrides(config.prompts_dir.as_deref()),
                Arc::clone(&prompt_context),
                SystemPromptConfig::default(),
                venv_info,
            );
            sys_prompt_provider.set_prompts_dir(config.prompts_dir.clone());
            context_pipeline.register(Box::new(sys_prompt_provider));
        }
        context_pipeline.register(Box::new(InjectContextStatus::new(4096)));

        // 10b. Register InjectTools (PromptBased mode) with taxonomy + core tools.
        // Core-tools summary is generated from always-active definitions so the
        // prompt is never out of sync with the actual tool registry.
        let core_tools_summary = {
            let set = tool_activation_set.read().await;
            build_core_tools_summary(&set)
        };
        context_pipeline.register(Box::new(InjectTools::with_taxonomy_and_core_tools(
            tool_taxonomy.root_summary(),
            core_tools_summary,
        )));

        // 10c. Register InjectSkills (dynamic -- reads active_skills from PromptContext).
        //      Also build the SkillSearch index for tool_search.
        let skill_search = Self::build_skill_search_index(config.skills_dir.as_deref());
        if let Some(ref skills_dir) = config.skills_dir {
            context_pipeline.register(Box::new(InjectSkills::new(
                Arc::clone(&prompt_context),
                skills_dir.clone(),
            )));
        }

        // 10d. Register KnowledgeContextProvider in context pipeline.
        //      (knowledge_service was initialised in step 7b above.)
        {
            let ks = knowledge_service.lock().await;
            let knowledge_handle = ks.knowledge_handle();
            if let Some(ref provider) = embedding_provider {
                context_pipeline.register(Box::new(KnowledgeContextProvider::with_embedding(
                    knowledge_handle,
                    Arc::clone(provider),
                )));
            } else {
                context_pipeline
                    .register(Box::new(KnowledgeContextProvider::new(knowledge_handle)));
            }
        }

        // 11. Workflow store.
        let workflow_store = SqliteWorkflowStore::new(pool.clone());

        // 12. Diagnostics -- SQLite-backed for persistence.
        let sqlite_trace_store = y_diagnostics::SqliteTraceStore::new(pool.clone());
        let trace_store_dyn: Arc<dyn TraceStore> = Arc::new(sqlite_trace_store);
        let diagnostics = Arc::new(DiagnosticsSubscriber::new(trace_store_dyn));

        // 13. Agent infrastructure.
        let (agent_registry, agent_pool_for_services, agent_delegator, delegation_tracker) =
            Self::init_agent_and_diagnostics(&provider_pool, &diagnostics);

        // 14. Pruning engine -- wired with agent_delegator for progressive pruning.
        let pruning_engine =
            PruningEngine::with_delegator(config.pruning.clone(), Arc::clone(&agent_delegator));

        // 15. Compaction engine (default config, no LLM backend yet).
        let compaction_engine = CompactionEngine::new();

        // Default compaction threshold from session config (percentage of context window).
        let compaction_threshold_pct = config.session.compaction_threshold_pct;

        // 16. Provider metrics store (observability persistence).
        let provider_metrics_store = SqliteProviderMetricsStore::new(pool.clone());

        // Wire metrics event senders so each request is logged to SQLite.
        {
            let receivers = provider_pool.attach_event_senders();
            let pms = provider_metrics_store.clone();
            tokio::spawn(async move {
                Self::run_metrics_event_consumers(receivers, pms).await;
            });
        }

        Ok(Self {
            provider_pool: RwLock::new(provider_pool),
            session_manager,
            hook_system,
            tool_registry,
            runtime_manager,
            context_pipeline,
            guardrail_manager,
            agent_registry,
            agent_pool: agent_pool_for_services,
            agent_delegator,
            delegation_tracker,
            workflow_store,
            prompt_context,
            diagnostics,
            chat_checkpoint_manager,
            tool_activation_set,
            tool_taxonomy,
            chat_message_store,
            knowledge_service,
            pruning_engine,
            compaction_engine,
            compaction_threshold_pct,
            pruning_watermarks: RwLock::new(HashMap::new()),
            provider_metrics_store,
            skill_search: RwLock::new(skill_search),
        })
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
            db_path
                .parent()
                .unwrap_or(std::path::Path::new("."))
                .join("knowledge")
        };
        let mut knowledge_service =
            KnowledgeService::with_data_dir(config.knowledge.clone(), knowledge_data_dir);

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

        if let Some(ref provider) = embedding_provider {
            knowledge_service.set_embedding_provider(Arc::clone(provider));
        }

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
// MutexPoolDelegator -- shared-pool delegation adapter
// ---------------------------------------------------------------------------

/// Thin wrapper: implements `AgentDelegator` by locking a shared
/// `Arc<Mutex<AgentPool>>` so that runner swaps propagate to all
/// delegation call sites.
struct MutexPoolDelegator(Arc<Mutex<AgentPool>>);

impl std::fmt::Debug for MutexPoolDelegator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MutexPoolDelegator").finish_non_exhaustive()
    }
}

#[async_trait::async_trait]
impl AgentDelegator for MutexPoolDelegator {
    async fn delegate(
        &self,
        agent_name: &str,
        input: serde_json::Value,
        context_strategy: y_core::agent::ContextStrategyHint,
        session_id: Option<uuid::Uuid>,
    ) -> Result<y_core::agent::DelegationOutput, y_core::agent::DelegationError> {
        let pool = self.0.lock().await;
        pool.delegate(agent_name, input, context_strategy, session_id)
            .await
    }
}

/// Result of agent sub-system initialisation.
type AgentInitResult = (
    Mutex<AgentRegistry>,
    Arc<Mutex<AgentPool>>,
    Arc<dyn AgentDelegator>,
    Arc<DelegationTracker>,
);

impl ServiceContainer {
    /// Initialise agent registry, pool, delegator, and wrap the delegator with diagnostics.
    ///
    /// Uses a SINGLE shared `AgentPool` behind `tokio::sync::Mutex` so that
    /// both `self.agent_pool` and `self.agent_delegator` share the same pool.
    /// When `init_agent_runner()` swaps the runner to `ServiceAgentRunner`,
    /// the change automatically affects the delegation path.
    fn init_agent_and_diagnostics(
        provider_pool: &Arc<ProviderPoolImpl>,
        diagnostics: &Arc<DiagnosticsSubscriber<dyn TraceStore>>,
    ) -> AgentInitResult {
        let agent_registry = Mutex::new(AgentRegistry::new());
        let mut agent_pool = AgentPool::new(MultiAgentConfig::default());

        let runner = Arc::new(SingleTurnRunner::new(
            Arc::clone(provider_pool) as Arc<dyn y_core::provider::ProviderPool>
        ));
        agent_pool.set_runner(runner);

        let delegation_tracker = Arc::clone(agent_pool.delegation_tracker());

        // Wrap the pool in Arc<Mutex<...>> so both self.agent_pool and the
        // delegator share the same pool instance.
        let shared_pool = Arc::new(Mutex::new(agent_pool));

        let agent_delegator: Arc<dyn AgentDelegator> =
            Arc::new(MutexPoolDelegator(Arc::clone(&shared_pool)));
        let agent_delegator: Arc<dyn AgentDelegator> =
            Arc::new(crate::diagnostics::DiagnosticsAgentDelegator::new(
                agent_delegator,
                Arc::clone(diagnostics),
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
        let providers = build_providers_from_config(pool_config);
        let new_pool = Arc::new(ProviderPoolImpl::from_providers(providers, pool_config));
        let mut guard = self.provider_pool.write().await;
        *guard = new_pool;
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
            .get_tool(&y_core::types::ToolName::from_string("browser"))
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

    /// Two-phase initialisation: swap the agent runner from `SingleTurnRunner`
    /// to `ServiceAgentRunner` so that sub-agents use the unified
    /// `AgentService::execute()` loop with multi-turn tool calling.
    ///
    /// Because `agent_pool` is shared (via `MutexPoolDelegator`) with
    /// `agent_delegator`, this single swap upgrades both delegation and
    /// direct pool access paths.
    ///
    /// Must be called **after** the container has been wrapped in `Arc`.
    pub fn init_agent_runner(self: &Arc<Self>) {
        let runner = Arc::new(crate::agent_service::ServiceAgentRunner::new(Arc::clone(
            self,
        )));
        // The agent_pool held by the container is behind a Mutex.
        // We acquire it synchronously (blocking_lock) since this runs once
        // during startup, before any async work begins.
        // Since the delegator shares this same pool (via MutexPoolDelegator),
        // the runner swap automatically takes effect for all delegation calls.
        self.agent_pool.blocking_lock().set_runner(runner);
        tracing::info!("ServiceAgentRunner initialised for sub-agent delegation");
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

/// Built-in tools that are always active and don't need `tool_search` to use.
const CORE_TOOL_NAMES: &[&str] = &[
    "tool_search",
    "file_read",
    "file_write",
    "shell_exec",
    "browser",
    "web_fetch",
];

/// Pre-activate all built-in tools as always-active in the activation set.
async fn pre_activate_core_tools(
    registry: &ToolRegistryImpl,
    activation_set: &Arc<RwLock<ToolActivationSet>>,
) {
    let mut set = activation_set.write().await;
    for &name in CORE_TOOL_NAMES {
        if let Some(def) = registry.get_definition(&ToolName::from_string(name)).await {
            set.activate(def);
            set.set_always_active(&ToolName::from_string(name));
        }
    }
}

/// Generate a compact summary of always-active tools for prompt injection.
///
/// Produces a Markdown table with tool name, first-sentence description,
/// and a usage hint (required args or available params), followed by a
/// usage reminder. Called once at startup.
fn build_core_tools_summary(set: &ToolActivationSet) -> String {
    let mut defs = set.always_active_definitions();
    defs.sort_by_key(|d| d.name.as_str().to_string());
    let mut lines = vec![
        "## Core Tools (always available)\n".to_string(),
        "You can call these tools directly without searching:\n".to_string(),
        "| Tool | Description | Usage |".to_string(),
        "|------|-------------|-------|".to_string(),
    ];
    for def in &defs {
        // First sentence of description only.
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
    lines.push(
        "IMPORTANT: Use ONLY these exact tool names. \
         Do NOT invent tool names like 'ls', 'cat', 'grep', or 'mkdir'. \
         For shell operations not covered above, use shell_exec."
            .to_string(),
    );
    lines.join("\n")
}

/// Extract a compact usage hint from a JSON Schema `parameters` value.
///
/// When the schema has an explicit `"required"` array, the hint shows only
/// those params with placeholders derived from descriptions, e.g.
/// `{"path": "<File_path_to>"}`.
///
/// When no required params exist (common for multi-mode tools like
/// `tool_search` and `web_fetch`), the hint falls back to listing all
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
// Provider construction
// ---------------------------------------------------------------------------

/// Build LLM provider instances from configuration entries.
pub fn build_providers_from_config(
    pool_config: &y_provider::config::ProviderPoolConfig,
) -> Vec<Arc<dyn LlmProvider>> {
    let mut providers: Vec<Arc<dyn LlmProvider>> = Vec::new();

    for config in &pool_config.providers {
        let Some(api_key) = config.resolve_api_key() else {
            let env_var = config.api_key_env.as_deref().unwrap_or("(not configured)");
            warn!(
                provider_id = %config.id,
                env_var = %env_var,
                "Skipping provider: API key not found in environment"
            );
            continue;
        };

        let proxy_url = pool_config.resolve_proxy_url(&config.id, &config.tags);

        match config.provider_type.as_str() {
            "openai" | "openai-compat" => {
                // openai-compat covers any OpenAI-compatible REST endpoint
                // (e.g., vLLM, LiteLLM, self-hosted models).  Both types
                // use the same OpenAiProvider implementation; the distinction
                // is purely for user clarity in the configuration UI.
                providers.push(Arc::new(OpenAiProvider::new(
                    &config.id,
                    &config.model,
                    api_key,
                    config.base_url.clone(),
                    proxy_url,
                    config.tags.clone(),
                    config.max_concurrency,
                    config.context_window,
                )));
            }
            "anthropic" => {
                providers.push(Arc::new(AnthropicProvider::new(
                    &config.id,
                    &config.model,
                    api_key,
                    config.base_url.clone(),
                    proxy_url,
                    config.tags.clone(),
                    config.max_concurrency,
                    config.context_window,
                )));
            }
            "gemini" => {
                providers.push(Arc::new(GeminiProvider::new(
                    &config.id,
                    &config.model,
                    api_key,
                    config.base_url.clone(),
                    proxy_url,
                    config.tags.clone(),
                    config.max_concurrency,
                    config.context_window,
                )));
            }
            "ollama" => {
                providers.push(Arc::new(OllamaProvider::new(
                    &config.id,
                    &config.model,
                    api_key,
                    config.base_url.clone(),
                    proxy_url,
                    config.tags.clone(),
                    config.max_concurrency,
                    config.context_window,
                )));
            }
            "azure" => {
                providers.push(Arc::new(AzureOpenAiProvider::new(
                    &config.id,
                    &config.model,
                    api_key,
                    config.base_url.clone(),
                    proxy_url,
                    config.tags.clone(),
                    config.max_concurrency,
                    config.context_window,
                )));
            }
            "deepseek" => {
                // DeepSeek uses an OpenAI-compatible REST API.  Default
                // base URL points to the official DeepSeek endpoint.
                let base_url = config
                    .base_url
                    .clone()
                    .or_else(|| Some("https://api.deepseek.com/v1".to_string()));
                providers.push(Arc::new(OpenAiProvider::new(
                    &config.id,
                    &config.model,
                    api_key,
                    base_url,
                    proxy_url,
                    config.tags.clone(),
                    config.max_concurrency,
                    config.context_window,
                )));
            }
            other => {
                warn!(
                    provider_id = %config.id,
                    provider_type = %other,
                    "Skipping provider: unsupported type (supported: openai, openai-compat, anthropic, gemini, ollama, azure, deepseek)"
                );
            }
        }
    }

    providers
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
        config.storage.db_path = ":memory:".to_string();

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
        config.storage.db_path = ":memory:".to_string();

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
                tags: vec![],
                max_concurrency: 5,
                context_window: 128_000,
                cost_per_1k_input: 0.0,
                cost_per_1k_output: 0.0,
                api_key: None,
                api_key_env: Some("Y_AGENT_NONEXISTENT_KEY_12345".into()),
                base_url: None,
                temperature: None,
                top_p: None,
                tool_calling_mode: None,
            }],
            ..Default::default()
        };
        let providers = build_providers_from_config(&pool_config);
        assert!(providers.is_empty());
    }

    #[test]
    fn test_build_providers_skips_unsupported_type() {
        std::env::set_var("Y_AGENT_TEST_SVC_KEY", "test-key");

        let pool_config = y_provider::config::ProviderPoolConfig {
            providers: vec![y_provider::config::ProviderConfig {
                id: "test-unsupported".into(),
                provider_type: "unsupported_backend".into(),
                model: "some-model".into(),
                tags: vec![],
                max_concurrency: 5,
                context_window: 128_000,
                cost_per_1k_input: 0.0,
                cost_per_1k_output: 0.0,
                api_key: None,
                api_key_env: Some("Y_AGENT_TEST_SVC_KEY".into()),
                base_url: None,
                temperature: None,
                top_p: None,
                tool_calling_mode: None,
            }],
            ..Default::default()
        };
        let providers = build_providers_from_config(&pool_config);
        assert!(providers.is_empty());

        std::env::remove_var("Y_AGENT_TEST_SVC_KEY");
    }

    #[test]
    fn test_build_providers_openai_compat_alias() {
        std::env::set_var("Y_AGENT_TEST_COMPAT_KEY", "sk-test");

        let pool_config = y_provider::config::ProviderPoolConfig {
            providers: vec![y_provider::config::ProviderConfig {
                id: "my-compat".into(),
                provider_type: "openai-compat".into(),
                model: "local-model".into(),
                tags: vec![],
                max_concurrency: 2,
                context_window: 32_000,
                cost_per_1k_input: 0.0,
                cost_per_1k_output: 0.0,
                api_key: None,
                api_key_env: Some("Y_AGENT_TEST_COMPAT_KEY".into()),
                base_url: Some("http://localhost:8080/v1".into()),
                temperature: None,
                top_p: None,
                tool_calling_mode: None,
            }],
            ..Default::default()
        };
        let providers = build_providers_from_config(&pool_config);
        assert_eq!(
            providers.len(),
            1,
            "openai-compat should build exactly one provider"
        );

        std::env::remove_var("Y_AGENT_TEST_COMPAT_KEY");
    }

    #[test]
    fn test_build_providers_deepseek_alias() {
        std::env::set_var("Y_AGENT_TEST_DEEPSEEK_KEY", "sk-ds-test");

        let pool_config = y_provider::config::ProviderPoolConfig {
            providers: vec![y_provider::config::ProviderConfig {
                id: "deepseek-chat".into(),
                provider_type: "deepseek".into(),
                model: "deepseek-chat".into(),
                tags: vec![],
                max_concurrency: 3,
                context_window: 64_000,
                cost_per_1k_input: 0.0,
                cost_per_1k_output: 0.0,
                api_key: None,
                api_key_env: Some("Y_AGENT_TEST_DEEPSEEK_KEY".into()),
                base_url: None,
                temperature: None,
                top_p: None,
                tool_calling_mode: None,
            }],
            ..Default::default()
        };
        let providers = build_providers_from_config(&pool_config);
        assert_eq!(
            providers.len(),
            1,
            "deepseek should build exactly one provider"
        );

        std::env::remove_var("Y_AGENT_TEST_DEEPSEEK_KEY");
    }

    #[tokio::test]
    async fn test_container_registers_context_providers() {
        let mut config = ServiceConfig::default();
        config.storage.db_path = ":memory:".to_string();

        let sc = ServiceContainer::from_config(&config).await.unwrap();
        assert_eq!(sc.context_pipeline.provider_count(), 4);
    }

    #[tokio::test]
    async fn test_skill_ingestion_service_factory() {
        let mut config = ServiceConfig::default();
        config.storage.db_path = ":memory:".to_string();

        let sc = ServiceContainer::from_config(&config).await.unwrap();
        let registry = Arc::new(RwLock::new(y_skills::SkillRegistryImpl::new()));
        let _service = sc.skill_ingestion_service(registry);
        // Construction succeeds -- delegator is correctly wired.
    }
}
