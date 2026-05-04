//! Chat service — LLM turn lifecycle orchestration.
//!
//! Centralises the full LLM-turn lifecycle:
//! 1. Context assembly (system prompt via context pipeline)
//! 2. Build `ChatRequest` with tool definitions
//! 3. LLM call via `ProviderPool`
//! 4. Diagnostics recording (trace, generation, tool observations)
//! 5. Tool execution loop (up to `guardrails.max_tool_iterations`)
//! 6. Session message persistence
//! 7. Checkpoint creation
//!
//! The core LLM + tool loop has been extracted into [`crate::agent_service::AgentService`]
//! so that sub-agents (A2A) share the same execution path. `ChatService` is now
//! a thin session-management wrapper.

use uuid::Uuid;

use y_agent::agent::definition::AgentDefinition;
use y_context::AssembledContext;
use y_core::provider::{RequestMode, ToolCallingMode};
use y_core::session::{ChatMessageRecord, ChatMessageStatus, ChatMessageStore, SessionNode};
use y_core::types::{Message, Role, SessionId};

use crate::agent_service::{AgentExecutionConfig, AgentExecutionError};
use crate::container::ServiceContainer;

// Re-export types from chat_types for backward compatibility.
pub use crate::chat_types::{
    PendingInteractions, PendingPermissions, PermissionPromptResponse, PrepareTurnError,
    PrepareTurnRequest, PreparedTurn, ResendTurnError, ResendTurnRequest, SessionAgentConfig,
    SessionAgentFeatures, ToolCallRecord, TurnCancellationToken, TurnError, TurnEvent,
    TurnEventSender, TurnInput, TurnMetaSummary, TurnResult,
};

// ---------------------------------------------------------------------------
// ChatService
// ---------------------------------------------------------------------------

/// LLM chat turn orchestration service.
///
/// All methods are static — they accept a `&ServiceContainer` reference
/// to access domain services. This keeps the API simple and avoids
/// lifetime issues with holding container references.
pub struct ChatService;

impl ChatService {
    /// Execute a single chat turn (no progress events).
    pub async fn execute_turn(
        container: &ServiceContainer,
        input: &TurnInput<'_>,
    ) -> Result<TurnResult, TurnError> {
        Self::execute_turn_inner(container, input, None, None).await
    }

    /// Execute a single chat turn with real-time progress events.
    ///
    /// The sender receives [`TurnEvent`] values that presentation layers
    /// can translate into Tauri events, SSE payloads, TUI redraws, etc.
    ///
    /// Pass a [`TurnCancellationToken`] to support mid-turn cancellation.
    /// When the token is cancelled the function returns `Err(TurnError::Cancelled)`
    /// as soon as it is safe to do so (typically within one LLM HTTP round-trip).
    pub async fn execute_turn_with_progress(
        container: &ServiceContainer,
        input: &TurnInput<'_>,
        progress: TurnEventSender,
        cancel: Option<TurnCancellationToken>,
    ) -> Result<TurnResult, TurnError> {
        Self::execute_turn_inner(container, input, Some(progress), cancel).await
    }

    fn build_execution_config(
        input: &TurnInput<'_>,
        tool_defs: Vec<serde_json::Value>,
        tool_calling_mode: ToolCallingMode,
        max_tool_iterations: usize,
    ) -> AgentExecutionConfig {
        let max_iterations = input
            .max_iterations
            .map_or(max_tool_iterations, |value| value.min(max_tool_iterations));

        AgentExecutionConfig {
            agent_name: input.agent_name.clone(),
            system_prompt: String::new(), // Uses context pipeline instead
            max_iterations,
            max_tool_calls: input.max_tool_calls.unwrap_or(usize::MAX),
            tool_definitions: tool_defs,
            tool_calling_mode,
            messages: input.history.to_vec(),
            provider_id: input.provider_id.clone(),
            preferred_models: input.preferred_models.clone(),
            provider_tags: input.provider_tags.clone(),
            request_mode: input.request_mode,
            working_directory: input.working_directory.clone(),
            temperature: input.temperature,
            max_tokens: input.max_completion_tokens,
            thinking: input.thinking.clone(),
            session_id: Some(input.session_id.clone()),
            session_uuid: input.session_uuid,
            knowledge_collections: input.knowledge_collections.clone(),
            use_context_pipeline: true,
            user_query: input.user_input.to_string(),
            external_trace_id: None,
            trust_tier: input.trust_tier,
            agent_allowed_tools: input.agent_allowed_tools.clone(),
            prune_tool_history: input.prune_tool_history,
            response_format: None,
            image_generation_options: input.image_generation_options.clone(),
        }
    }

    async fn resolve_tool_calling_mode(
        container: &ServiceContainer,
        input: &TurnInput<'_>,
    ) -> ToolCallingMode {
        let pool = container.provider_pool().await;
        let metadata_list = pool.list_metadata();
        if let Some(ref provider_id) = input.provider_id {
            metadata_list
                .iter()
                .find(|metadata| metadata.id.to_string() == *provider_id)
                .map_or(ToolCallingMode::default(), |metadata| {
                    metadata.tool_calling_mode
                })
        } else {
            metadata_list
                .first()
                .map_or(ToolCallingMode::default(), |metadata| {
                    metadata.tool_calling_mode
                })
        }
    }

    async fn build_turn_tool_definitions(
        container: &ServiceContainer,
        input: &TurnInput<'_>,
    ) -> Vec<serde_json::Value> {
        if !input.toolcall_enabled || input.request_mode != RequestMode::TextChat {
            return vec![];
        }

        let mut tool_defs = if input.trust_tier.is_none() && input.agent_allowed_tools.is_empty() {
            Self::build_essential_tool_definitions(container).await
        } else {
            crate::agent_service::AgentService::build_filtered_tool_definitions(
                container,
                &input.agent_allowed_tools,
            )
            .await
        };

        Self::apply_mcp_mode_filter(
            &mut tool_defs,
            input.mcp_mode.as_deref(),
            &input.mcp_servers,
        );
        tool_defs
    }

    async fn configure_mcp_prompt_flag(container: &ServiceContainer, input: &TurnInput<'_>) {
        let mcp_mode = input.mcp_mode.as_deref().unwrap_or("auto");
        let has_mcp =
            if input.request_mode != RequestMode::ImageGeneration && mcp_mode != "disabled" {
                container.mcp_manager.connected_count().await > 0
            } else {
                false
            };
        let mut pctx = container.prompt_context.write().await;
        if has_mcp {
            pctx.config_flags.insert("mcp.enabled".into(), true);
        } else {
            pctx.config_flags.remove("mcp.enabled");
        }
    }

    async fn configure_plan_mode_prompt_flag(container: &ServiceContainer, input: &TurnInput<'_>) {
        let plan_mode = input.plan_mode.as_deref().unwrap_or("fast");
        tracing::info!(
            plan_mode = %plan_mode,
            raw_plan_mode = ?input.plan_mode,
            "plan mode received from frontend"
        );
        match plan_mode {
            "plan" => {
                let mut pctx = container.prompt_context.write().await;
                if input.request_mode == RequestMode::TextChat {
                    pctx.config_flags.insert("plan_mode.active".into(), true);
                } else {
                    pctx.config_flags.remove("plan_mode.active");
                }
                tracing::info!("plan_mode.active flag SET in prompt context");
            }
            "auto" => {
                let needs_plan = input.request_mode == RequestMode::TextChat
                    && crate::plan_orchestrator::assess_complexity(
                        container,
                        input.user_input,
                        input.provider_id.as_deref(),
                    )
                    .await;
                if needs_plan {
                    let mut pctx = container.prompt_context.write().await;
                    pctx.config_flags.insert("plan_mode.active".into(), true);
                    tracing::info!("plan_mode.active flag SET (auto: complex)");
                } else {
                    tracing::info!("plan_mode.active flag NOT set (auto: simple)");
                }
            }
            _ => {
                let mut pctx = container.prompt_context.write().await;
                pctx.config_flags.remove("plan_mode.active");
                tracing::info!("plan_mode.active flag CLEARED (fast mode)");
            }
        }
    }

    fn session_agent_config_from_definition(definition: &AgentDefinition) -> SessionAgentConfig {
        SessionAgentConfig {
            agent_id: definition.id.clone(),
            agent_name: definition.id.clone(),
            agent_mode: format!("{:?}", definition.mode).to_lowercase(),
            working_directory: definition.working_directory.clone(),
            features: SessionAgentFeatures {
                toolcall: definition.toolcall_enabled_resolved(),
                skills: definition.skills_enabled_resolved(),
                knowledge: definition.knowledge_enabled_resolved(),
            },
            allowed_tools: definition.allowed_tools.clone(),
            preset_skills: definition.skills.clone(),
            knowledge_collections: definition.knowledge_collections.clone(),
            prompt_section_ids: definition.prompt_section_ids.clone(),
            system_prompt: (!definition.system_prompt.trim().is_empty())
                .then(|| definition.system_prompt.clone()),
            provider_id: definition.provider_id.clone(),
            preferred_models: definition.preferred_models.clone(),
            provider_tags: definition.provider_tags.clone(),
            temperature: definition.temperature,
            max_completion_tokens: definition
                .max_completion_tokens
                .map(|value| u32::try_from(value).unwrap_or(u32::MAX)),
            thinking: definition.thinking_config(),
            plan_mode: definition.plan_mode.clone(),
            permission_mode: definition.permission_mode,
            max_iterations: definition.max_iterations,
            max_tool_calls: definition.max_tool_calls,
            trust_tier: definition.trust_tier,
            prune_tool_history: definition.prune_tool_history,
            mcp_mode: definition.mcp_mode.clone(),
            mcp_servers: definition.mcp_servers.clone(),
        }
    }

    async fn resolve_session_agent_config(
        container: &ServiceContainer,
        session: &SessionNode,
    ) -> Result<Option<SessionAgentConfig>, String> {
        let Some(agent_id) = session.agent_id.as_ref() else {
            return Ok(None);
        };

        let registry = container.agent_registry.lock().await;
        let definition = registry
            .get(agent_id.as_str())
            .ok_or_else(|| agent_id.as_str().to_string())?;
        Ok(Some(Self::session_agent_config_from_definition(definition)))
    }

    fn resolve_turn_skills(
        requested_skills: Option<Vec<String>>,
        agent_config: Option<&SessionAgentConfig>,
        inject_preset_skills: bool,
    ) -> Vec<String> {
        let mut resolved = if agent_config.is_some_and(|config| !config.features.skills) {
            Vec::new()
        } else if inject_preset_skills {
            agent_config.map_or_else(Vec::new, |config| config.preset_skills.clone())
        } else {
            Vec::new()
        };

        if agent_config.is_some_and(|config| !config.features.skills) {
            return resolved;
        }

        for skill in requested_skills.unwrap_or_default() {
            if !resolved.contains(&skill) {
                resolved.push(skill);
            }
        }

        resolved
    }

    fn resolve_turn_knowledge(
        requested_collections: Option<Vec<String>>,
        agent_config: Option<&SessionAgentConfig>,
    ) -> Vec<String> {
        let Some(config) = agent_config else {
            return requested_collections.unwrap_or_default();
        };

        if !config.features.knowledge {
            return Vec::new();
        }

        let requested = requested_collections.unwrap_or_default();
        if requested.is_empty() {
            config.knowledge_collections.clone()
        } else {
            requested
        }
    }

    fn request_mode_from_metadata(metadata: &serde_json::Value) -> Option<RequestMode> {
        metadata
            .get("request_mode")
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok())
    }

    /// Prepare a turn: resolve/create session, persist user message, read
    /// transcript, compute turn number, and assemble all data needed for
    /// `execute_turn()`.
    ///
    /// The returned [`PreparedTurn`] owns all intermediate data. Callers
    /// use [`PreparedTurn::as_turn_input()`] to obtain the borrowing
    /// [`TurnInput`] expected by `execute_turn*`.
    pub async fn prepare_turn(
        container: &ServiceContainer,
        request: PrepareTurnRequest,
    ) -> Result<PreparedTurn, PrepareTurnError> {
        use y_core::session::{CreateSessionOptions, SessionType};
        use y_core::types::{generate_message_id, now};

        // 1. Resolve or create session.
        let (session, session_created) = if let Some(sid) = request.session_id {
            let session = container
                .session_manager
                .get_session(&sid)
                .await
                .map_err(|e| PrepareTurnError::SessionNotFound(e.to_string()))?;
            (session, false)
        } else {
            let session = container
                .session_manager
                .create_session(CreateSessionOptions {
                    parent_id: None,
                    session_type: SessionType::Main,
                    agent_id: None,
                    title: None,
                })
                .await
                .map_err(|e| PrepareTurnError::SessionCreationFailed(e.to_string()))?;
            (session, true)
        };
        let session_id = session.id.clone();
        let agent_config = Self::resolve_session_agent_config(container, &session)
            .await
            .map_err(PrepareTurnError::SessionAgentNotFound)?;
        let existing_user_turns = container
            .session_manager
            .read_display_transcript(&session_id)
            .await
            .map_err(|e| PrepareTurnError::TranscriptReadFailed(e.to_string()))?
            .into_iter()
            .filter(|message| message.role == Role::User)
            .count();

        if let Some(config) = agent_config.as_ref() {
            if existing_user_turns >= config.max_iterations {
                return Err(PrepareTurnError::SessionTurnLimitReached {
                    agent_id: config.agent_id.clone(),
                    max_turns: config.max_iterations,
                });
            }
        }

        let skills = Self::resolve_turn_skills(
            request.skills,
            agent_config.as_ref(),
            existing_user_turns == 0,
        );
        let knowledge_collections =
            Self::resolve_turn_knowledge(request.knowledge_collections, agent_config.as_ref());
        let provider_id = request.provider_id.or_else(|| {
            agent_config
                .as_ref()
                .and_then(|config| config.provider_id.clone())
        });
        let thinking = request.thinking.or_else(|| {
            agent_config
                .as_ref()
                .and_then(|config| config.thinking.clone())
        });
        let plan_mode = request.plan_mode.or_else(|| {
            agent_config
                .as_ref()
                .and_then(|config| config.plan_mode.clone())
        });
        let mcp_mode = request.mcp_mode.or_else(|| {
            agent_config
                .as_ref()
                .and_then(|config| config.mcp_mode.clone())
        });
        let mcp_servers = request.mcp_servers.unwrap_or_else(|| {
            agent_config
                .as_ref()
                .map_or_else(Vec::new, |config| config.mcp_servers.clone())
        });
        let request_mode = request.request_mode.unwrap_or_default();

        // 2. Build and persist the user message.
        let metadata = {
            let mut meta = serde_json::Map::new();
            if !skills.is_empty() {
                meta.insert("skills".into(), serde_json::json!(skills));
            }
            if let Some(extra) = &request.user_message_metadata {
                if let Some(obj) = extra.as_object() {
                    for (k, v) in obj {
                        meta.insert(k.clone(), v.clone());
                    }
                }
            }
            if request_mode != RequestMode::TextChat {
                meta.insert(
                    "request_mode".into(),
                    serde_json::to_value(request_mode).unwrap_or(serde_json::Value::Null),
                );
            }
            if meta.is_empty() {
                serde_json::Value::Null
            } else {
                serde_json::Value::Object(meta)
            }
        };
        let user_msg = Message {
            message_id: generate_message_id(),
            role: Role::User,
            content: request.user_input.clone(),
            tool_call_id: None,
            tool_calls: vec![],
            timestamp: now(),
            metadata,
        };
        container
            .session_manager
            .append_message(&session_id, &user_msg)
            .await
            .map_err(|e| PrepareTurnError::PersistFailed(e.to_string()))?;

        // Mirror to SQLite chat_message_store so the pruning engine can
        // detect candidates. Fire-and-forget: failure here must not block
        // the turn.
        Self::mirror_to_chat_message_store(
            container,
            &session_id,
            &user_msg,
            None, // no model for user messages
            None,
            None,
            None,
            None,
        )
        .await;

        // 2b. File history snapshot (rewind support).
        //     Ensure a FileHistoryManager exists for this session, then
        //     create a snapshot at this user-message boundary. If the
        //     manager cannot be created, log and continue (rewind is
        //     best-effort, not turn-blocking).
        if let Err(e) = crate::rewind::RewindService::ensure_manager(
            &container.file_history_managers,
            &session_id,
            &container.data_dir,
        )
        .await
        {
            tracing::warn!(error = %e, "failed to initialize file history manager");
        }
        crate::rewind::RewindService::make_snapshot(
            &container.file_history_managers,
            &session_id,
            &user_msg.message_id,
        )
        .await;

        // 3. Read class transcript for LLM context (may be compacted).
        //    The context transcript is the source of truth for what the LLM
        //    sees. After compaction, older messages are replaced by a summary
        //    system message, so the LLM receives a shorter history.
        let history = container
            .session_manager
            .read_transcript(&session_id)
            .await
            .map_err(|e| PrepareTurnError::TranscriptReadFailed(e.to_string()))?;

        // 4. Derive turn number from the *display* transcript length (which is
        //    never compacted) so checkpoint bookkeeping stays consistent.
        let display_len = container
            .session_manager
            .read_display_transcript(&session_id)
            .await
            .map(|t| t.len())
            .unwrap_or(history.len());
        let turn_number = u32::try_from(display_len).unwrap_or(u32::MAX);
        let session_uuid = Uuid::parse_str(session_id.as_str()).unwrap_or_else(|_| Uuid::new_v4());

        Ok(PreparedTurn {
            session_id,
            session_uuid,
            history,
            turn_number,
            user_input: request.user_input,
            provider_id,
            request_mode,
            session_created,
            working_directory: agent_config
                .as_ref()
                .and_then(|config| config.working_directory.clone()),
            knowledge_collections,
            thinking,
            plan_mode,
            mcp_mode,
            mcp_servers,
            skills,
            agent_config,
            image_generation_options: request.image_generation_options,
        })
    }

    /// Prepare a resend turn: keep the original user message, truncate the
    /// assistant reply + tool messages, invalidate newer checkpoints, and
    /// return a [`PreparedTurn`] ready for execution.
    ///
    /// This mirrors [`Self::prepare_turn`] but for the resend case where no new
    /// user message is appended — the existing one is reused.
    pub async fn prepare_resend_turn(
        container: &ServiceContainer,
        request: ResendTurnRequest,
    ) -> Result<PreparedTurn, ResendTurnError> {
        let session = container
            .session_manager
            .get_session(&request.session_id)
            .await
            .map_err(|e| ResendTurnError::TranscriptReadFailed(e.to_string()))?;
        let agent_config = Self::resolve_session_agent_config(container, &session)
            .await
            .map_err(ResendTurnError::SessionAgentNotFound)?;

        // 1. Load the checkpoint to find message_count_before.
        let checkpoint = container
            .chat_checkpoint_manager
            .checkpoint_store()
            .load(&request.checkpoint_id)
            .await
            .map_err(|e| ResendTurnError::CheckpointNotFound(e.to_string()))?;

        // 2. Partial truncation: keep user message (message_count_before + 1),
        //    remove assistant reply and any tool messages after it.
        let keep_count = checkpoint.message_count_before as usize + 1;
        container
            .session_manager
            .display_transcript_store()
            .truncate(&request.session_id, keep_count)
            .await
            .map_err(|e| ResendTurnError::TruncateFailed(e.to_string()))?;
        container
            .session_manager
            .transcript_store()
            .truncate(&request.session_id, keep_count)
            .await
            .map_err(|e| ResendTurnError::TruncateFailed(e.to_string()))?;

        // 3. Invalidate this checkpoint and all newer ones.
        container
            .chat_checkpoint_manager
            .checkpoint_store()
            .invalidate_after(
                &request.session_id,
                checkpoint.turn_number.saturating_sub(1),
            )
            .await
            .map_err(|e| ResendTurnError::TruncateFailed(e.to_string()))?;

        // 4. Read context transcript (may be compacted) for LLM messages.
        let history = container
            .session_manager
            .read_transcript(&request.session_id)
            .await
            .map_err(|e| ResendTurnError::TranscriptReadFailed(e.to_string()))?;

        if history.is_empty() {
            return Err(ResendTurnError::TranscriptEmpty);
        }

        // The last message after truncation must be the original user message.
        let Some(last_msg) = history.last() else {
            // Unreachable: guarded by is_empty() above.
            return Err(ResendTurnError::TranscriptEmpty);
        };
        if last_msg.role != Role::User {
            return Err(ResendTurnError::TruncateFailed(format!(
                "expected last message to be User after truncation, found {:?}",
                last_msg.role
            )));
        }
        let requested_skills = last_msg
            .metadata
            .get("skills")
            .and_then(|value| value.as_array())
            .map(|values| {
                values
                    .iter()
                    .filter_map(|value| value.as_str().map(str::to_owned))
                    .collect::<Vec<_>>()
            });
        let user_turns = history
            .iter()
            .filter(|message| message.role == Role::User)
            .count();
        let skills =
            Self::resolve_turn_skills(requested_skills, agent_config.as_ref(), user_turns == 1);
        let knowledge_collections =
            Self::resolve_turn_knowledge(request.knowledge_collections, agent_config.as_ref());
        let provider_id = request.provider_id.or_else(|| {
            agent_config
                .as_ref()
                .and_then(|config| config.provider_id.clone())
        });
        let thinking = request.thinking.or_else(|| {
            agent_config
                .as_ref()
                .and_then(|config| config.thinking.clone())
        });
        let plan_mode = request.plan_mode.or_else(|| {
            agent_config
                .as_ref()
                .and_then(|config| config.plan_mode.clone())
        });
        let request_mode = request
            .request_mode
            .or_else(|| Self::request_mode_from_metadata(&last_msg.metadata))
            .unwrap_or_default();
        let mcp_mode = agent_config
            .as_ref()
            .and_then(|config| config.mcp_mode.clone());
        let mcp_servers = agent_config
            .as_ref()
            .map_or_else(Vec::new, |config| config.mcp_servers.clone());
        let user_input = last_msg.content.clone();

        // Derive turn number from display transcript (never compacted) for
        // checkpoint consistency.
        let display_len = container
            .session_manager
            .read_display_transcript(&request.session_id)
            .await
            .map(|t| t.len())
            .unwrap_or(history.len());
        let turn_number = u32::try_from(display_len).unwrap_or(0);
        let session_uuid =
            Uuid::parse_str(request.session_id.as_str()).unwrap_or_else(|_| Uuid::new_v4());

        Ok(PreparedTurn {
            session_id: request.session_id,
            session_uuid,
            history,
            turn_number,
            user_input,
            provider_id,
            request_mode,
            session_created: false,
            working_directory: agent_config
                .as_ref()
                .and_then(|config| config.working_directory.clone()),
            knowledge_collections,
            thinking,
            plan_mode,
            mcp_mode,
            mcp_servers,
            skills,
            agent_config,
            image_generation_options: None,
        })
    }

    /// Look up metadata for the last completed LLM turn in a session.
    ///
    /// Queries the diagnostics store for the most recent trace belonging to
    /// the session, extracts the model from the last Generation observation,
    /// and resolves `context_window` from the provider pool by model match.
    ///
    /// Returns `None` if no trace data exists for this session.
    pub async fn get_last_turn_meta(
        container: &ServiceContainer,
        session_id: &str,
    ) -> Result<Option<TurnMetaSummary>, String> {
        let Ok(session_uuid) = Uuid::parse_str(session_id) else {
            return Ok(None);
        };

        let store = container.diagnostics.store();
        let traces = store
            .list_traces_by_session(&session_uuid.to_string(), 1)
            .await
            .unwrap_or_default();

        let Some(trace) = traces.first() else {
            return Ok(None);
        };

        let observations = store.get_observations(trace.id).await.unwrap_or_default();
        let last_gen = observations
            .iter()
            .rev()
            .find(|o| o.obs_type == y_diagnostics::ObservationType::Generation);

        let model = last_gen.and_then(|o| o.model.clone()).unwrap_or_default();
        let last_gen_input_tokens = last_gen.map_or(0, |o| o.input_tokens);

        let pool = container.provider_pool().await;
        let metadata_list = pool.list_metadata();
        let matched = metadata_list.iter().find(|m| m.model == model);
        let context_window = matched.map_or(0, |m| m.context_window);
        let provider_id = matched.map(|m| m.id.to_string());

        Ok(Some(TurnMetaSummary {
            provider_id,
            model,
            input_tokens: trace.total_input_tokens,
            output_tokens: trace.total_output_tokens,
            cost_usd: trace.total_cost_usd,
            context_window,
            context_tokens_used: last_gen_input_tokens,
        }))
    }

    /// Internal implementation shared by both entry points.
    ///
    /// Delegates the entire LLM + tool execution loop to [`AgentService::execute`],
    /// then handles session-specific post-processing (message persistence,
    /// checkpointing, metadata enrichment).
    async fn execute_turn_inner(
        container: &ServiceContainer,
        input: &TurnInput<'_>,
        progress: Option<TurnEventSender>,
        cancel: Option<TurnCancellationToken>,
    ) -> Result<TurnResult, TurnError> {
        use crate::agent_service::AgentService;

        // 1. Build provider/tool execution settings for the root agent.
        let tool_calling_mode = Self::resolve_tool_calling_mode(container, input).await;
        let mut tool_defs = Self::build_turn_tool_definitions(container, input).await;

        // 1a'. Set mcp.enabled flag so the MCP hint prompt section is included.
        //
        // MCP tools live in the connection manager (not the tool registry),
        // so we check for connected MCP servers directly.
        Self::configure_mcp_prompt_flag(container, input).await;

        // 1b. Inject plan_mode.active config flag based on the user's mode selection.
        //
        // - "fast" (default/None): no plan mode prompts injected.
        // - "plan": always inject plan_mode_active prompt section.
        // - "auto": run a lightweight complexity classification, inject if complex.
        Self::configure_plan_mode_prompt_flag(container, input).await;

        // 1c. Inject Plan tool schema when plan mode is active.
        if input.request_mode == RequestMode::TextChat {
            Self::apply_plan_mode_tool_adjustments(container, &mut tool_defs).await;
        }

        // 2. Construct execution config for the root agent.
        let max_tool_iterations = container.guardrail_manager.config().max_tool_iterations;
        let exec_config =
            Self::build_execution_config(input, tool_defs, tool_calling_mode, max_tool_iterations);

        // 3. Delegate to AgentService.
        let result = match AgentService::execute(container, &exec_config, progress, cancel).await {
            Ok(r) => r,
            Err(AgentExecutionError::LlmError {
                message,
                partial_messages,
            }) => {
                // Persist intermediate messages (assistant + tool results from
                // earlier successful iterations) so the conversation history
                // survives the error and the user can continue / retry.
                for msg in &partial_messages {
                    let _ = container
                        .session_manager
                        .append_message(&input.session_id, msg)
                        .await;
                }
                if !partial_messages.is_empty() {
                    tracing::info!(
                        count = partial_messages.len(),
                        session = %input.session_id.0,
                        "persisted partial messages before LLM error"
                    );
                }
                return Err(TurnError::LlmError(message));
            }
            Err(AgentExecutionError::Cancelled {
                partial_messages,
                accumulated_content,
                iteration_texts,
                iteration_reasonings,
                iteration_reasoning_durations_ms,
                iteration_tool_counts,
                tool_calls_executed,
                iterations,
                input_tokens,
                output_tokens,
                cost_usd,
                model,
                generated_images,
            }) => {
                // Persist intermediate messages (assistant + tool results from
                // earlier successful iterations) to the CONTEXT transcript only.
                // These are raw protocol messages (individual assistant msgs with
                // tool_calls + tool role msgs) that the LLM needs for continuity
                // on resume, but they are NOT suitable for GUI display (the
                // frontend expects a single consolidated assistant message).
                let ctx_store = container.session_manager.transcript_store();
                for msg in &partial_messages {
                    let _ = ctx_store.append(&input.session_id, msg).await;
                }

                // Build and persist a consolidated assistant message with all
                // accumulated content and metadata. This goes to BOTH transcripts
                // so the GUI can render it properly and the LLM sees the final
                // state on resume.
                if !accumulated_content.trim().is_empty() || !tool_calls_executed.is_empty() {
                    let tool_results_meta: Vec<serde_json::Value> =
                        Self::build_tool_results_metadata(&tool_calls_executed);

                    let mut meta = serde_json::json!({
                        "model": model,
                        "input_tokens": input_tokens,
                        "output_tokens": output_tokens,
                        "cost_usd": cost_usd,
                        "tool_results": tool_results_meta,
                        "iteration_texts": iteration_texts,
                        "iteration_reasonings": iteration_reasonings,
                        "iteration_reasoning_durations_ms": iteration_reasoning_durations_ms,
                        "iteration_tool_counts": iteration_tool_counts,
                        "cancelled": true,
                    });

                    if !generated_images.is_empty() {
                        meta["generated_images"] = serde_json::to_value(&generated_images)
                            .unwrap_or(serde_json::Value::Array(vec![]));
                    }

                    let assistant_msg = Message {
                        message_id: y_core::types::generate_message_id(),
                        role: Role::Assistant,
                        content: accumulated_content.clone(),
                        tool_call_id: None,
                        tool_calls: vec![],
                        timestamp: y_core::types::now(),
                        metadata: meta,
                    };

                    // Display transcript: consolidated message for GUI rendering.
                    let _ = container
                        .session_manager
                        .display_transcript_store()
                        .append(&input.session_id, &assistant_msg)
                        .await;

                    // Context transcript: consolidated message for LLM context.
                    let _ = ctx_store.append(&input.session_id, &assistant_msg).await;
                }

                if !partial_messages.is_empty() || !accumulated_content.trim().is_empty() {
                    tracing::info!(
                        partial_count = partial_messages.len(),
                        accumulated_len = accumulated_content.len(),
                        iterations,
                        session = %input.session_id.0,
                        "persisted partial state on cancellation"
                    );
                }

                // No checkpoint or post-turn optimization for cancelled turns.
                return Err(TurnError::Cancelled);
            }
            Err(e) => return Err(TurnError::from(e)),
        };

        // 4. Session-specific post-processing: persist final assistant message,
        //    create checkpoint. AgentService doesn't handle session storage —
        //    that's the ChatService's responsibility.

        // Build tool_results metadata for frontend rendering after session reload.
        let tool_results_meta: Vec<serde_json::Value> =
            Self::build_tool_results_metadata(&result.tool_calls_executed);

        let mut meta = serde_json::json!({
            "model": result.model,
            "input_tokens": result.input_tokens,
            "output_tokens": result.output_tokens,
            "cost_usd": result.cost_usd,
            "tool_results": tool_results_meta,
            "context_window": result.context_window,
            "context_tokens_used": result.last_input_tokens,
            "final_response": result.final_response,
            "iteration_texts": result.iteration_texts,
            "iteration_reasonings": result.iteration_reasonings,
            "iteration_reasoning_durations_ms": result.iteration_reasoning_durations_ms,
            "iteration_tool_counts": result.iteration_tool_counts,
        });

        if !result.generated_images.is_empty() {
            meta["generated_images"] = serde_json::to_value(&result.generated_images)
                .unwrap_or(serde_json::Value::Array(vec![]));
        }

        // Preserve reasoning_content: prefer the direct field (always available),
        // then fall back to scanning new_messages (for multi-iteration cases where
        // reasoning was produced in an earlier iteration).
        if let Some(ref rc) = result.reasoning_content {
            meta["reasoning_content"] = serde_json::Value::String(rc.clone());
        } else if let Some(last_assistant) = result
            .new_messages
            .iter()
            .rev()
            .find(|m| m.role == Role::Assistant)
        {
            if let Some(rc) = last_assistant.metadata.get("reasoning_content") {
                meta["reasoning_content"] = rc.clone();
            }
        }

        // Persist reasoning/thinking duration so the frontend can show it
        // after page reload (without relying on client-side timestamps).
        if let Some(rd) = result.reasoning_duration_ms {
            meta["reasoning_duration_ms"] = serde_json::json!(rd);
        }

        let assistant_msg = Message {
            message_id: y_core::types::generate_message_id(),
            role: Role::Assistant,
            content: result.content.clone(),
            tool_call_id: None,
            tool_calls: vec![],
            timestamp: y_core::types::now(),
            metadata: meta,
        };

        if let Err(e) = container
            .session_manager
            .append_message(&input.session_id, &assistant_msg)
            .await
        {
            tracing::warn!(
                error = %e,
                session_id = %input.session_id,
                "failed to persist assistant message to session transcript"
            );
        }

        // Mirror to SQLite chat_message_store for pruning engine visibility.
        Self::mirror_to_chat_message_store(
            container,
            &input.session_id,
            &assistant_msg,
            Some(&result.model),
            Some(result.input_tokens),
            Some(result.output_tokens),
            Some(result.cost_usd),
            Some(result.context_window),
        )
        .await;

        let mut new_messages = result.new_messages.clone();
        new_messages.push(assistant_msg);

        // Checkpoint.
        let msg_count_before = u32::try_from(input.history.len().saturating_sub(1)).unwrap_or(0);
        let turn = input.turn_number + 1;
        let scope_id = format!("turn-{}-{}", input.session_id.0, turn);
        if let Err(e) = container
            .chat_checkpoint_manager
            .create_checkpoint(&input.session_id, turn, msg_count_before, scope_id)
            .await
        {
            tracing::warn!(error = %e, "failed to create chat checkpoint");
        }

        // Post-turn context optimization (pruning + conditional compaction).
        if let Err(e) = crate::context_optimization::ContextOptimizationService::optimize_post_turn(
            container,
            &input.session_id,
            result.context_window,
        )
        .await
        {
            tracing::warn!(error = %e, "post-turn context optimization failed");
        }

        Ok(TurnResult {
            content: result.content,
            model: result.model,
            provider_id: result.provider_id,
            input_tokens: result.input_tokens,
            output_tokens: result.output_tokens,
            last_input_tokens: result.last_input_tokens,
            context_window: result.context_window,
            cost_usd: result.cost_usd,
            tool_calls_executed: result.tool_calls_executed,
            iterations: result.iterations,
            generated_images: result.generated_images,
            new_messages,
        })
    }

    /// Build tool results metadata for persisting in assistant message metadata.
    ///
    /// Shared by the normal completion path and the cancellation persistence
    /// path to avoid duplicating the URL metadata extraction logic.
    fn build_tool_results_metadata(
        tool_calls: &[crate::agent_service::ToolCallRecord],
    ) -> Vec<serde_json::Value> {
        tool_calls
            .iter()
            .map(|tc| {
                let mut entry = serde_json::json!({
                    "name": tc.name,
                    "arguments": tc.arguments,
                    "success": tc.success,
                    "duration_ms": tc.duration_ms,
                    "result_preview": &tc.result_content,
                });
                // Use pre-extracted url_meta directly (survives result
                // stripping for Browser/WebFetch tools).
                if let Some(ref meta_str) = tc.url_meta {
                    if let Ok(meta_val) = serde_json::from_str::<serde_json::Value>(meta_str) {
                        entry["url_meta"] = meta_val;
                    }
                }
                if let Some(ref meta) = tc.metadata {
                    entry["metadata"] = meta.clone();
                }
                entry
            })
            .collect()
    }

    /// Adjust tool definitions for plan mode.
    ///
    /// When `plan_mode.active` is set in the prompt context, injects the
    /// `Plan` tool schema so the LLM can trigger the planning workflow.
    /// Unlike the old system, no tools are blocked -- the Plan tool
    /// orchestrator handles everything via sub-agent delegation.
    async fn apply_plan_mode_tool_adjustments(
        container: &ServiceContainer,
        tool_defs: &mut Vec<serde_json::Value>,
    ) {
        let is_active = {
            let pctx = container.prompt_context.read().await;
            pctx.config_flags
                .get("plan_mode.active")
                .copied()
                .unwrap_or(false)
        };
        if !is_active {
            return;
        }

        // Inject Plan tool schema if not already present.
        let already_present = tool_defs.iter().any(|def| {
            def.get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                == Some("Plan")
        });
        if already_present {
            return;
        }

        let tn = y_core::types::ToolName::from_string("Plan");
        if let Some(def) = container.tool_registry.get_definition(&tn).await {
            tool_defs.push(serde_json::json!({
                "type": "function",
                "function": {
                    "name": def.name.as_str(),
                    "description": def.description,
                    "parameters": def.parameters,
                }
            }));
        }

        tracing::info!(
            final_count = tool_defs.len(),
            "plan mode: injected Plan tool schema"
        );
    }

    /// Filter MCP tool definitions according to the user's MCP mode.
    ///
    /// - `"auto"` (default / `None`): no filtering (all MCP tools pass through).
    /// - `"manual"`: keep only MCP tools whose server name is in `allowed_servers`.
    /// - `"disabled"`: remove every tool whose name starts with the `mcp_` prefix.
    ///
    /// Non-MCP tools (no `mcp_` prefix) are never removed.
    fn apply_mcp_mode_filter(
        tool_defs: &mut Vec<serde_json::Value>,
        mcp_mode: Option<&str>,
        allowed_servers: &[String],
    ) {
        let mode = mcp_mode.unwrap_or("auto");
        if mode == "auto" {
            return;
        }

        let before = tool_defs.len();
        tool_defs.retain(|def| {
            let name = def
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                .unwrap_or("");
            let Some((server, _)) = y_tools::mcp_integration::split_qualified_tool_name(name)
            else {
                return true;
            };
            match mode {
                "disabled" => false,
                "manual" => allowed_servers.iter().any(|s| s == server),
                _ => true,
            }
        });

        tracing::info!(
            mcp_mode = mode,
            before = before,
            after = tool_defs.len(),
            "mcp mode filter applied"
        );
    }

    /// Build LLM messages by prepending system prompt from assembled context.
    ///
    /// Delegates to [`crate::message_builder::build_chat_messages`].
    pub fn build_chat_messages(assembled: &AssembledContext, history: &[Message]) -> Vec<Message> {
        crate::message_builder::build_chat_messages(assembled, history)
    }

    /// Build tool definitions in `OpenAI` function-calling JSON format.
    ///
    /// Returns definitions for ALL registered tools. Prefer
    /// [`Self::build_essential_tool_definitions`] for root agent turns to enforce
    /// lazy loading.
    pub async fn build_tool_definitions(container: &ServiceContainer) -> Vec<serde_json::Value> {
        let defs = container.tool_registry.get_all_definitions().await;
        defs.iter()
            .map(|def| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": def.name.as_str(),
                        "description": def.description,
                        "parameters": def.parameters,
                    }
                })
            })
            .collect()
    }

    /// Build tool definitions for essential tools only (lazy loading).
    ///
    /// Returns definitions for `ESSENTIAL_TOOL_NAMES` -- the minimal set
    /// required for every LLM call. Additional tools are injected
    /// dynamically after `ToolSearch` activates them.
    pub async fn build_essential_tool_definitions(
        container: &ServiceContainer,
    ) -> Vec<serde_json::Value> {
        use crate::container::ESSENTIAL_TOOL_NAMES;

        let mut defs = Vec::with_capacity(ESSENTIAL_TOOL_NAMES.len());
        for &name in ESSENTIAL_TOOL_NAMES {
            if let Some(def) = container
                .tool_registry
                .get_definition(&y_core::types::ToolName::from_string(name))
                .await
            {
                defs.push(serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": def.name.as_str(),
                        "description": def.description,
                        "parameters": def.parameters,
                    }
                }));
            }
        }
        defs
    }

    /// Determine whether title generation should be triggered for this turn.
    ///
    /// Business rule: generate a title when the session has at least one user
    /// message and (`user_msg_count == 1` OR `user_msg_count` is a multiple of
    /// `title_summarize_interval`). Disabled when `title_summarize_interval` is 0.
    pub fn should_generate_title(container: &ServiceContainer, history: &[Message]) -> bool {
        let title_interval = container.session_manager.config().title_summarize_interval;
        if title_interval == 0 {
            return false;
        }
        let user_msg_count = history.iter().filter(|m| m.role == Role::User).count();
        user_msg_count > 0 && (user_msg_count == 1 || user_msg_count % title_interval as usize == 0)
    }

    /// Mirror a `Message` to the `ChatMessageStore` (`SQLite`) so that the
    /// pruning engine can detect candidates and invoke `pruning-summarizer`.
    ///
    /// This is fire-and-forget: a failure is logged but never propagated,
    /// because the JSONL transcript is the primary persistence layer.
    async fn mirror_to_chat_message_store(
        container: &ServiceContainer,
        session_id: &SessionId,
        msg: &Message,
        model: Option<&str>,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
        cost_usd: Option<f64>,
        context_window: Option<usize>,
    ) {
        let role_str = match msg.role {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
        };

        let record = ChatMessageRecord {
            id: msg.message_id.clone(),
            session_id: session_id.clone(),
            role: role_str.to_string(),
            content: msg.content.clone(),
            status: ChatMessageStatus::Active,
            checkpoint_id: None,
            model: model.map(std::string::ToString::to_string),
            input_tokens: input_tokens.map(|v| i64::try_from(v).unwrap_or(i64::MAX)),
            output_tokens: output_tokens.map(|v| i64::try_from(v).unwrap_or(i64::MAX)),
            cost_usd,
            context_window: context_window.map(|v| i64::try_from(v).unwrap_or(i64::MAX)),
            parent_message_id: None,
            pruning_group_id: None,
            created_at: msg.timestamp,
        };

        if let Err(e) = container.chat_message_store.insert(&record).await {
            tracing::warn!(
                error = %e,
                session_id = %session_id,
                message_id = %msg.message_id,
                "failed to mirror message to chat_message_store"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use y_context::{ContextCategory, ContextItem};

    fn make_history() -> Vec<Message> {
        vec![
            Message {
                message_id: y_core::types::generate_message_id(),
                role: Role::User,
                content: "Hello".to_string(),
                tool_call_id: None,
                tool_calls: vec![],
                timestamp: y_core::types::now(),
                metadata: serde_json::Value::Null,
            },
            Message {
                message_id: y_core::types::generate_message_id(),
                role: Role::Assistant,
                content: "Hi there!".to_string(),
                tool_call_id: None,
                tool_calls: vec![],
                timestamp: y_core::types::now(),
                metadata: serde_json::Value::Null,
            },
        ]
    }

    #[test]
    fn test_build_chat_messages_prepends_system() {
        let mut assembled = AssembledContext::default();
        assembled.add(ContextItem {
            category: ContextCategory::SystemPrompt,
            content: "You are y-agent, a helpful AI assistant.".to_string(),
            token_estimate: 10,
            priority: 100,
        });

        let history = make_history();
        let messages = ChatService::build_chat_messages(&assembled, &history);

        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].role, Role::System);
        assert!(messages[0].content.contains("y-agent"));
        assert_eq!(messages[1].role, Role::User);
        assert_eq!(messages[2].role, Role::Assistant);
    }

    #[test]
    fn test_build_chat_messages_no_system_when_empty() {
        let assembled = AssembledContext::default();
        let history = make_history();
        let messages = ChatService::build_chat_messages(&assembled, &history);
        assert_eq!(messages.len(), 2);
    }

    #[test]
    fn test_build_chat_messages_joins_multiple_system_items() {
        let mut assembled = AssembledContext::default();
        assembled.add(ContextItem {
            category: ContextCategory::SystemPrompt,
            content: "Part one".to_string(),
            token_estimate: 5,
            priority: 100,
        });
        assembled.add(ContextItem {
            category: ContextCategory::Status,
            content: "status info".to_string(),
            token_estimate: 5,
            priority: 500,
        });
        assembled.add(ContextItem {
            category: ContextCategory::SystemPrompt,
            content: "Part two".to_string(),
            token_estimate: 5,
            priority: 200,
        });
        assembled.add(ContextItem {
            category: ContextCategory::Skills,
            content: "### Skill: code_review\nReviews code.".to_string(),
            token_estimate: 10,
            priority: 400,
        });

        let history = make_history();
        let messages = ChatService::build_chat_messages(&assembled, &history);
        assert_eq!(messages.len(), 3);
        assert!(messages[0].content.contains("Part one"));
        assert!(messages[0].content.contains("Part two"));
        assert!(messages[0].content.contains("### Skill: code_review")); // Skills included
        assert!(!messages[0].content.contains("status info")); // Status excluded
    }

    #[test]
    fn test_build_chat_messages_includes_skills() {
        let mut assembled = AssembledContext::default();
        assembled.add(ContextItem {
            category: ContextCategory::SystemPrompt,
            content: "You are y-agent.".to_string(),
            token_estimate: 5,
            priority: 100,
        });
        assembled.add(ContextItem {
            category: ContextCategory::Skills,
            content: "### Skill: refactor\nRefactors code to improve structure.".to_string(),
            token_estimate: 10,
            priority: 400,
        });

        let history = make_history();
        let messages = ChatService::build_chat_messages(&assembled, &history);
        assert_eq!(messages.len(), 3); // system + 2 history
        assert_eq!(messages[0].role, Role::System);
        assert!(messages[0].content.contains("You are y-agent."));
        assert!(messages[0].content.contains("### Skill: refactor"));
    }

    #[test]
    fn test_turn_error_display() {
        assert!(TurnError::LlmError("timeout".into())
            .to_string()
            .contains("timeout"));
        assert!(TurnError::ToolLoopLimitExceeded { max_iterations: 10 }
            .to_string()
            .contains("10"));
    }

    #[test]
    fn test_build_execution_config_preserves_none_temperature() {
        let history = make_history();
        let input = TurnInput {
            user_input: "hello",
            session_id: SessionId::from_string("session-1"),
            session_uuid: Uuid::new_v4(),
            history: &history,
            turn_number: 2,
            provider_id: None,
            request_mode: RequestMode::TextChat,
            working_directory: None,
            knowledge_collections: vec![],
            thinking: None,
            plan_mode: None,
            agent_name: "chat-turn".into(),
            toolcall_enabled: true,
            preferred_models: vec![],
            provider_tags: vec![],
            temperature: None,
            max_completion_tokens: None,
            max_iterations: None,
            max_tool_calls: None,
            trust_tier: None,
            agent_allowed_tools: vec![],
            prune_tool_history: false,
            mcp_mode: None,
            mcp_servers: vec![],
            image_generation_options: None,
        };

        let config =
            ChatService::build_execution_config(&input, vec![], ToolCallingMode::default(), 8);
        assert_eq!(config.temperature, None);
    }

    #[test]
    fn test_build_execution_config_preserves_explicit_temperature() {
        let history = make_history();
        let input = TurnInput {
            user_input: "hello",
            session_id: SessionId::from_string("session-1"),
            session_uuid: Uuid::new_v4(),
            history: &history,
            turn_number: 2,
            provider_id: None,
            request_mode: RequestMode::TextChat,
            working_directory: Some("/repo/workspace".into()),
            knowledge_collections: vec![],
            thinking: None,
            plan_mode: None,
            agent_name: "chat-turn".into(),
            toolcall_enabled: true,
            preferred_models: vec![],
            provider_tags: vec![],
            temperature: Some(1.0),
            max_completion_tokens: None,
            max_iterations: None,
            max_tool_calls: None,
            trust_tier: None,
            agent_allowed_tools: vec![],
            prune_tool_history: false,
            mcp_mode: None,
            mcp_servers: vec![],
            image_generation_options: None,
        };

        let config =
            ChatService::build_execution_config(&input, vec![], ToolCallingMode::default(), 8);
        assert_eq!(config.temperature, Some(1.0));
        assert_eq!(config.working_directory.as_deref(), Some("/repo/workspace"));
    }

    // -----------------------------------------------------------------------
    // prepare_turn tests
    // -----------------------------------------------------------------------

    async fn make_test_container() -> (crate::container::ServiceContainer, tempfile::TempDir) {
        let tmpdir = tempfile::TempDir::new().unwrap();
        let mut config = crate::config::ServiceConfig::default();
        config.storage = y_storage::StorageConfig {
            db_path: ":memory:".to_string(),
            pool_size: 1,
            wal_enabled: false,
            transcript_dir: tmpdir.path().join("transcripts"),
            ..y_storage::StorageConfig::default()
        };
        let container = crate::container::ServiceContainer::from_config(&config)
            .await
            .expect("test container should build");
        (container, tmpdir)
    }

    #[tokio::test]
    async fn prepare_turn_creates_new_session() {
        let (container, _tmp) = make_test_container().await;
        let request = PrepareTurnRequest {
            session_id: None,
            user_input: "hello".into(),
            provider_id: None,
            request_mode: None,
            skills: None,
            knowledge_collections: None,
            thinking: None,
            user_message_metadata: None,
            plan_mode: None,
            mcp_mode: None,
            mcp_servers: None,
            image_generation_options: None,
        };
        let prepared = ChatService::prepare_turn(&container, request)
            .await
            .expect("prepare_turn should succeed");
        assert!(prepared.session_created);
        assert!(!prepared.session_id.as_str().is_empty());
        assert!(!prepared.history.is_empty());
    }

    #[tokio::test]
    async fn prepare_turn_resolves_existing_session() {
        use y_core::session::{CreateSessionOptions, SessionType};

        let (container, _tmp) = make_test_container().await;
        let session = container
            .session_manager
            .create_session(CreateSessionOptions {
                parent_id: None,
                session_type: SessionType::Main,
                agent_id: None,
                title: None,
            })
            .await
            .unwrap();

        let request = PrepareTurnRequest {
            session_id: Some(session.id.clone()),
            user_input: "hello".into(),
            provider_id: None,
            request_mode: None,
            skills: None,
            knowledge_collections: None,
            thinking: None,
            user_message_metadata: None,
            plan_mode: None,
            mcp_mode: None,
            mcp_servers: None,
            image_generation_options: None,
        };
        let prepared = ChatService::prepare_turn(&container, request)
            .await
            .expect("should resolve existing session");
        assert!(!prepared.session_created);
        assert_eq!(prepared.session_id, session.id);
    }

    #[tokio::test]
    async fn prepare_turn_invalid_session_returns_not_found() {
        let (container, _tmp) = make_test_container().await;
        let request = PrepareTurnRequest {
            session_id: Some(SessionId("nonexistent-id".into())),
            user_input: "hello".into(),
            provider_id: None,
            request_mode: None,
            skills: None,
            knowledge_collections: None,
            thinking: None,
            user_message_metadata: None,
            plan_mode: None,
            mcp_mode: None,
            mcp_servers: None,
            image_generation_options: None,
        };
        let err = ChatService::prepare_turn(&container, request)
            .await
            .unwrap_err();
        assert!(matches!(err, PrepareTurnError::SessionNotFound(_)));
    }

    #[tokio::test]
    async fn prepare_turn_persists_user_message() {
        let (container, _tmp) = make_test_container().await;
        let request = PrepareTurnRequest {
            session_id: None,
            user_input: "test message".into(),
            provider_id: None,
            request_mode: None,
            skills: None,
            knowledge_collections: None,
            thinking: None,
            user_message_metadata: None,
            plan_mode: None,
            mcp_mode: None,
            mcp_servers: None,
            image_generation_options: None,
        };
        let prepared = ChatService::prepare_turn(&container, request)
            .await
            .unwrap();

        // History should contain at least the user message.
        let last = prepared
            .history
            .last()
            .expect("history should not be empty");
        assert_eq!(last.role, Role::User);
        assert_eq!(last.content, "test message");
    }

    #[tokio::test]
    async fn prepare_turn_persists_image_generation_request_mode() {
        let (container, _tmp) = make_test_container().await;
        let request = PrepareTurnRequest {
            session_id: None,
            user_input: "draw a lighthouse".into(),
            provider_id: None,
            request_mode: Some(RequestMode::ImageGeneration),
            skills: None,
            knowledge_collections: None,
            thinking: None,
            user_message_metadata: None,
            plan_mode: None,
            mcp_mode: None,
            mcp_servers: None,
            image_generation_options: None,
        };
        let prepared = ChatService::prepare_turn(&container, request)
            .await
            .expect("prepare_turn should succeed");

        let last = prepared
            .history
            .last()
            .expect("history should not be empty");
        assert_eq!(prepared.request_mode, RequestMode::ImageGeneration);
        assert_eq!(
            prepared.as_turn_input().request_mode,
            RequestMode::ImageGeneration
        );
        assert_eq!(
            last.metadata
                .get("request_mode")
                .and_then(|value| value.as_str()),
            Some("image_generation")
        );
    }

    #[tokio::test]
    async fn prepare_resend_turn_restores_request_mode_from_user_metadata() {
        let (container, _tmp) = make_test_container().await;
        let prepared = ChatService::prepare_turn(
            &container,
            PrepareTurnRequest {
                session_id: None,
                user_input: "generate a skyline at dusk".into(),
                provider_id: None,
                request_mode: Some(RequestMode::ImageGeneration),
                skills: None,
                knowledge_collections: None,
                thinking: None,
                user_message_metadata: None,
                plan_mode: None,
                mcp_mode: None,
                mcp_servers: None,
                image_generation_options: None,
            },
        )
        .await
        .expect("prepare_turn should succeed");

        let assistant = Message {
            message_id: y_core::types::generate_message_id(),
            role: Role::Assistant,
            content: "done".into(),
            tool_call_id: None,
            tool_calls: vec![],
            timestamp: y_core::types::now(),
            metadata: serde_json::Value::Null,
        };
        container
            .session_manager
            .append_message(&prepared.session_id, &assistant)
            .await
            .expect("assistant message should persist");

        let checkpoint = container
            .chat_checkpoint_manager
            .create_checkpoint(&prepared.session_id, 1, 0, "scope-1".to_string())
            .await
            .expect("checkpoint should create");

        let resent = ChatService::prepare_resend_turn(
            &container,
            ResendTurnRequest {
                session_id: prepared.session_id.clone(),
                checkpoint_id: checkpoint.checkpoint_id,
                provider_id: None,
                request_mode: None,
                knowledge_collections: None,
                thinking: None,
                plan_mode: None,
            },
        )
        .await
        .expect("prepare_resend_turn should succeed");

        assert_eq!(resent.request_mode, RequestMode::ImageGeneration);
        assert_eq!(
            resent.as_turn_input().request_mode,
            RequestMode::ImageGeneration
        );
        assert_eq!(resent.history.len(), 1);
        assert_eq!(resent.history[0].role, Role::User);
    }

    #[tokio::test]
    async fn prepare_turn_as_turn_input_matches() {
        let (container, _tmp) = make_test_container().await;
        let request = PrepareTurnRequest {
            session_id: None,
            user_input: "hello".into(),
            provider_id: Some("test-provider".into()),
            request_mode: None,
            skills: None,
            knowledge_collections: None,
            thinking: None,
            user_message_metadata: None,
            plan_mode: None,
            mcp_mode: None,
            mcp_servers: None,
            image_generation_options: None,
        };
        let prepared = ChatService::prepare_turn(&container, request)
            .await
            .unwrap();
        let input = prepared.as_turn_input();
        assert_eq!(input.user_input, "hello");
        assert_eq!(input.session_id, prepared.session_id);
        assert_eq!(input.session_uuid, prepared.session_uuid);
        assert_eq!(input.turn_number, prepared.turn_number);
        assert_eq!(input.provider_id, Some("test-provider".into()));
    }

    #[tokio::test]
    async fn prepare_turn_turn_number_equals_history_len() {
        let (container, _tmp) = make_test_container().await;
        let request = PrepareTurnRequest {
            session_id: None,
            user_input: "first".into(),
            provider_id: None,
            request_mode: None,
            skills: None,
            knowledge_collections: None,
            thinking: None,
            user_message_metadata: None,
            plan_mode: None,
            mcp_mode: None,
            mcp_servers: None,
            image_generation_options: None,
        };
        let p1 = ChatService::prepare_turn(&container, request)
            .await
            .unwrap();
        assert_eq!(p1.turn_number, p1.history.len() as u32);

        // Second message in same session.
        let request2 = PrepareTurnRequest {
            session_id: Some(p1.session_id.clone()),
            user_input: "second".into(),
            provider_id: None,
            request_mode: None,
            skills: None,
            knowledge_collections: None,
            thinking: None,
            user_message_metadata: None,
            plan_mode: None,
            mcp_mode: None,
            mcp_servers: None,
            image_generation_options: None,
        };
        let p2 = ChatService::prepare_turn(&container, request2)
            .await
            .unwrap();
        assert_eq!(p2.turn_number, p2.history.len() as u32);
        assert!(p2.turn_number > p1.turn_number);
    }

    #[tokio::test]
    async fn prepare_turn_agent_session_applies_agent_defaults() {
        use y_agent::agent::definition::AgentDefinition;
        use y_core::provider::ThinkingEffort;
        use y_core::session::{CreateSessionOptions, SessionType};
        use y_core::types::AgentId;

        let (container, _tmp) = make_test_container().await;
        let definition = AgentDefinition::from_toml(
            r#"
id = "agent-session"
name = "Agent Session"
description = "Preset-backed chat session"
mode = "general"
trust_tier = "user_defined"
system_prompt = "You are the bound agent."
provider_id = "preset-provider"
skills = ["workspace-skill"]
knowledge_enabled = true
knowledge_collections = ["project-notes"]
plan_mode = "plan"
thinking_effort = "high"
"#,
        )
        .expect("agent definition should parse");
        container
            .agent_registry
            .lock()
            .await
            .register_user_defined(definition)
            .expect("agent should register");

        let session = container
            .session_manager
            .create_session(CreateSessionOptions {
                parent_id: None,
                session_type: SessionType::Main,
                agent_id: Some(AgentId::from_string("agent-session")),
                title: None,
            })
            .await
            .expect("session should create");

        let prepared = ChatService::prepare_turn(
            &container,
            PrepareTurnRequest {
                session_id: Some(session.id),
                user_input: "hello".into(),
                provider_id: None,
                request_mode: None,
                skills: None,
                knowledge_collections: None,
                thinking: None,
                user_message_metadata: None,
                plan_mode: None,
                mcp_mode: None,
                mcp_servers: None,
                image_generation_options: None,
            },
        )
        .await
        .expect("agent session prepare_turn should succeed");

        assert_eq!(prepared.provider_id.as_deref(), Some("preset-provider"));
        assert_eq!(prepared.skills, vec!["workspace-skill"]);
        assert_eq!(prepared.knowledge_collections, vec!["project-notes"]);
        assert_eq!(prepared.plan_mode.as_deref(), Some("plan"));
        assert_eq!(
            prepared.thinking.as_ref().map(|config| config.effort),
            Some(ThinkingEffort::High)
        );
    }

    #[tokio::test]
    async fn prepare_turn_agent_session_injects_preset_skills_only_on_first_turn() {
        use y_agent::agent::definition::AgentDefinition;
        use y_core::session::{CreateSessionOptions, SessionType};
        use y_core::types::AgentId;

        let (container, _tmp) = make_test_container().await;
        let definition = AgentDefinition::from_toml(
            r#"
id = "skill-agent"
name = "Skill Agent"
description = "Injects preset skills only once"
mode = "general"
trust_tier = "user_defined"
system_prompt = "Use the preset skill."
skills = ["workspace-skill"]
"#,
        )
        .expect("agent definition should parse");
        container
            .agent_registry
            .lock()
            .await
            .register_user_defined(definition)
            .expect("agent should register");

        let session = container
            .session_manager
            .create_session(CreateSessionOptions {
                parent_id: None,
                session_type: SessionType::Main,
                agent_id: Some(AgentId::from_string("skill-agent")),
                title: None,
            })
            .await
            .expect("session should create");

        let first = ChatService::prepare_turn(
            &container,
            PrepareTurnRequest {
                session_id: Some(session.id.clone()),
                user_input: "first".into(),
                provider_id: None,
                request_mode: None,
                skills: None,
                knowledge_collections: None,
                thinking: None,
                user_message_metadata: None,
                plan_mode: None,
                mcp_mode: None,
                mcp_servers: None,
                image_generation_options: None,
            },
        )
        .await
        .expect("first turn should succeed");
        assert_eq!(first.skills, vec!["workspace-skill"]);

        let second = ChatService::prepare_turn(
            &container,
            PrepareTurnRequest {
                session_id: Some(session.id),
                user_input: "second".into(),
                provider_id: None,
                request_mode: None,
                skills: None,
                knowledge_collections: None,
                thinking: None,
                user_message_metadata: None,
                plan_mode: None,
                mcp_mode: None,
                mcp_servers: None,
                image_generation_options: None,
            },
        )
        .await
        .expect("second turn should succeed");
        assert!(second.skills.is_empty());
    }

    #[tokio::test]
    async fn prepare_turn_agent_session_uses_max_iterations_as_turn_limit() {
        use y_agent::agent::definition::AgentDefinition;
        use y_core::session::{CreateSessionOptions, SessionType};
        use y_core::types::AgentId;

        let (container, _tmp) = make_test_container().await;
        let definition = AgentDefinition::from_toml(
            r#"
id = "limited-agent"
name = "Limited Agent"
description = "Single-turn session agent"
mode = "general"
trust_tier = "user_defined"
system_prompt = "One turn only."
max_iterations = 1
"#,
        )
        .expect("agent definition should parse");
        container
            .agent_registry
            .lock()
            .await
            .register_user_defined(definition)
            .expect("agent should register");

        let session = container
            .session_manager
            .create_session(CreateSessionOptions {
                parent_id: None,
                session_type: SessionType::Main,
                agent_id: Some(AgentId::from_string("limited-agent")),
                title: None,
            })
            .await
            .expect("session should create");

        ChatService::prepare_turn(
            &container,
            PrepareTurnRequest {
                session_id: Some(session.id.clone()),
                user_input: "first".into(),
                provider_id: None,
                request_mode: None,
                skills: None,
                knowledge_collections: None,
                thinking: None,
                user_message_metadata: None,
                plan_mode: None,
                mcp_mode: None,
                mcp_servers: None,
                image_generation_options: None,
            },
        )
        .await
        .expect("first turn should succeed");

        let err = ChatService::prepare_turn(
            &container,
            PrepareTurnRequest {
                session_id: Some(session.id),
                user_input: "second".into(),
                provider_id: None,
                request_mode: None,
                skills: None,
                knowledge_collections: None,
                thinking: None,
                user_message_metadata: None,
                plan_mode: None,
                mcp_mode: None,
                mcp_servers: None,
                image_generation_options: None,
            },
        )
        .await
        .expect_err("second turn should hit the session turn limit");

        assert!(matches!(
            err,
            PrepareTurnError::SessionTurnLimitReached { .. }
        ));
    }
}
