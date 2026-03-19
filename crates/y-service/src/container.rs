//! Central dependency container — shared by all presentation layers.
//!
//! Mirrors the former `AppServices` from `y-cli/wire.rs`, but lives in the
//! service layer so CLI, TUI, and future Web API can all construct one.

use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::sync::{Mutex, RwLock};
use tracing::{info, warn};

use y_agent::{AgentPool, AgentRegistry, DelegationTracker, MultiAgentConfig};
use y_context::{
    BuildSystemPromptProvider, BunVenvPromptInfo, ContextPipeline, InjectContextStatus,
    InjectSkills, InjectTools, KnowledgeContextProvider, PythonVenvPromptInfo,
    SystemPromptConfig, VenvPromptInfo,
};
use y_core::agent::AgentDelegator;
use y_core::provider::LlmProvider;
use y_core::types::ToolName;
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
use y_skills::SkillRegistryImpl;
use y_storage::{
    SqliteChatCheckpointStore, SqliteChatMessageStore, SqliteSessionStore, SqliteWorkflowStore,
};
use y_tools::{ToolActivationSet, ToolRegistryImpl, ToolTaxonomy};

use crate::config::ServiceConfig;
use crate::diagnostics::DiagnosticsAgentDelegator;
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
    pub agent_pool: Mutex<AgentPool>,

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

    /// Shared dynamic tool schemas — updated when tools are activated
    /// via `tool_search`, read by `InjectTools` at context assembly time.
    pub dynamic_tool_schemas: Arc<RwLock<Vec<String>>>,

    /// Chat message store for session history tree (Phase 2).
    pub chat_message_store: Arc<SqliteChatMessageStore>,

    /// Knowledge base service (ingestion, retrieval, embedding).
    ///
    /// Uses `tokio::sync::Mutex` so the GUI layer can share this `Arc` and
    /// hold the lock across `.await` points (e.g. `ingest().await`).
    pub knowledge_service: Arc<Mutex<KnowledgeService>>,
}

impl ServiceContainer {
    /// Wire all services from a validated [`ServiceConfig`].
    ///
    /// This is the sole dependency-wiring entry point. Presentation layers
    /// call this once and pass the resulting `ServiceContainer` to service
    /// methods and command handlers.
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

        // 8. Tool registry.
        let tool_registry = ToolRegistryImpl::new(config.tools.clone());
        y_tools::builtin::register_builtin_tools(&tool_registry, config.browser.clone(), None).await;

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
        // Pre-activate tool_search (always-active).
        {
            let tool_search_def = tool_registry
                .get_definition(&ToolName::from_string("tool_search"))
                .await;
            let mut set = tool_activation_set.write().await;
            if let Some(def) = tool_search_def {
                set.activate(def);
                set.set_always_active(&ToolName::from_string("tool_search"));
            }
        }

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
        context_pipeline.register(Box::new(BuildSystemPromptProvider::with_venv_info(
            default_template(),
            builtin_section_store_with_overrides(config.prompts_dir.as_deref()),
            Arc::clone(&prompt_context),
            SystemPromptConfig::default(),
            venv_info,
        )));
        context_pipeline.register(Box::new(InjectContextStatus::new(4096)));

        // 10b. Register InjectTools (PromptBased mode) with taxonomy + dynamic schemas.
        let dynamic_tool_schemas: Arc<RwLock<Vec<String>>> = Arc::new(RwLock::new(Vec::new()));
        context_pipeline.register(Box::new(InjectTools::with_taxonomy_and_dynamic_schemas(
            tool_taxonomy.root_summary(),
            Arc::clone(&dynamic_tool_schemas),
        )));

        // 10c. Register InjectSkills (dynamic -- reads active_skills from PromptContext).
        if let Some(ref skills_dir) = config.skills_dir {
            context_pipeline.register(Box::new(InjectSkills::new(
                Arc::clone(&prompt_context),
                skills_dir.clone(),
            )));
        }

        // 10d. Knowledge service + embedding provider.
        //
        // Derive knowledge data dir from the storage db_path parent. This
        // places knowledge data alongside the SQLite database (e.g.,
        // `~/.local/state/y-agent/data/knowledge/`).
        let knowledge_data_dir = {
            let db_path = std::path::Path::new(&config.storage.db_path);
            db_path
                .parent()
                .unwrap_or(std::path::Path::new("."))
                .join("knowledge")
        };
        let mut knowledge_service =
            KnowledgeService::with_data_dir(config.knowledge.clone(), knowledge_data_dir);

        // Construct embedding provider if enabled.
        let embedding_provider: Option<Arc<dyn y_core::embedding::EmbeddingProvider>> =
            if config.knowledge.embedding_enabled {
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

        let knowledge_service = Arc::new(Mutex::new(knowledge_service));

        // 10e. Register KnowledgeContextProvider in context pipeline.
        {
            let ks = knowledge_service.lock().await;
            let knowledge_handle = ks.knowledge_handle();
            if let Some(ref provider) = embedding_provider {
                context_pipeline.register(Box::new(
                    KnowledgeContextProvider::with_embedding(
                        knowledge_handle,
                        Arc::clone(provider),
                    ),
                ));
            } else {
                context_pipeline.register(Box::new(
                    KnowledgeContextProvider::new(knowledge_handle),
                ));
            }
        }

        // 11. Agent registry + pool.
        let agent_registry = Mutex::new(AgentRegistry::new());
        let mut agent_pool = AgentPool::new(MultiAgentConfig::default());

        let runner = Arc::new(SingleTurnRunner::new(
            Arc::clone(&provider_pool) as Arc<dyn y_core::provider::ProviderPool>
        ));
        agent_pool.set_runner(runner);

        // Extract the delegation tracker *before* the pool is consumed by Arc::new().
        // This is the tracker that `delegate()` will write to.
        let delegation_tracker = Arc::clone(agent_pool.delegation_tracker());

        let agent_delegator: Arc<dyn AgentDelegator> = Arc::new(agent_pool);
        // Create a second pool with the same config and runner for service-level use.
        let mut agent_pool_for_services = AgentPool::new(MultiAgentConfig::default());
        let runner2 = Arc::new(SingleTurnRunner::new(
            Arc::clone(&provider_pool) as Arc<dyn y_core::provider::ProviderPool>
        ));
        agent_pool_for_services.set_runner(runner2);
        let agent_pool_for_services = Mutex::new(agent_pool_for_services);

        // 12. Workflow store.
        let workflow_store = SqliteWorkflowStore::new(pool.clone());

        // 13. Diagnostics -- use SQLite-backed store so data survives restarts.
        // The store needs a SqlitePool; we clone the existing pool reference.
        let sqlite_trace_store = y_diagnostics::SqliteTraceStore::new(pool.clone());
        let trace_store_dyn: Arc<dyn TraceStore> = Arc::new(sqlite_trace_store);
        let diagnostics = Arc::new(DiagnosticsSubscriber::new(trace_store_dyn));

        // 13b. Wrap the agent delegator with diagnostics recording so subagent
        // LLM calls (title-generator, skill-ingestion, etc.) appear in the
        // DIAGNOSTICS panel.
        let agent_delegator: Arc<dyn AgentDelegator> =
            Arc::new(DiagnosticsAgentDelegator::new(agent_delegator, Arc::clone(&diagnostics)));

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
            dynamic_tool_schemas,
            chat_message_store,
            knowledge_service,
        })
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
    /// to [`ServiceAgentRunner`] so that sub-agents use the unified
    /// `AgentService::execute()` loop.
    ///
    /// Must be called **after** the container has been wrapped in `Arc`.
    pub fn init_agent_runner(self: &Arc<Self>) {
        let runner = Arc::new(crate::agent_service::ServiceAgentRunner::new(
            Arc::clone(self),
        ));
        // The agent_pool held by the container is behind a Mutex.
        // We acquire it synchronously (blocking_lock) since this runs once
        // during startup, before any async work begins.
        self.agent_pool.blocking_lock().set_runner(runner);
        tracing::info!("ServiceAgentRunner initialised for sub-agent delegation");
    }
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
        let api_key = if let Some(key) = config.resolve_api_key() {
            key
        } else {
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
