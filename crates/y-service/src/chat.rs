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
use y_core::provider::{RequestMode, ThinkingConfig, ToolCallingMode};
use y_core::session::{ChatMessageRecord, ChatMessageStatus, ChatMessageStore, SessionNode};
use y_core::types::{Message, Role, SessionId};

use crate::agent_service::{AgentExecutionConfig, AgentExecutionError, AgentExecutionResult};
use crate::container::ServiceContainer;

// Re-export types from chat_types for backward compatibility.
pub use crate::chat_types::{
    FollowUpMessage, FollowUpQueues, OperationMode, PendingInteractions, PendingPermissions,
    PendingPlanReviews, PermissionPromptResponse, PlanReviewDecision, PrepareTurnError,
    PrepareTurnRequest, PreparedTurn, ResendTurnError, ResendTurnRequest, SessionAgentConfig,
    SessionAgentFeatures, SteerMessage, SteeringQueues, ToolCallRecord, TurnCancellationToken,
    TurnError, TurnEvent, TurnEventSender, TurnInput, TurnMetaSummary, TurnResult,
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

/// Turn configuration resolved from request + agent config.
///
/// Extracts the common field-resolution logic that was duplicated across
/// `prepare_turn`, the intra-turn retry path, and the resend retry path.
struct ResolvedTurnConfig {
    provider_id: Option<String>,
    thinking: Option<ThinkingConfig>,
    plan_mode: Option<String>,
    operation_mode: OperationMode,
    mcp_mode: Option<String>,
    mcp_servers: Vec<String>,
    working_directory: Option<String>,
}
impl ChatService {
    // -- Steering queue management -----------------------------------------
    //
    // Per-session FIFO queues of user messages enqueued while a turn is
    // streaming. The agent execution loop drains them at LLM-call boundaries
    // (see `agent_service::executor`). These methods are the only mutators;
    // presentation layers call them via the transport endpoints.

    /// Enqueue a steering message for a session. Returns the created entry.
    pub async fn add_steer(
        container: &ServiceContainer,
        session_id: &SessionId,
        text: String,
    ) -> SteerMessage {
        let steer = SteerMessage::new(text);
        let mut queues = container.session_state.steering_queues.lock().await;
        queues
            .entry(session_id.clone())
            .or_default()
            .push(steer.clone());
        steer
    }

    /// List the pending steering messages for a session (FIFO order).
    pub async fn list_steers(
        container: &ServiceContainer,
        session_id: &SessionId,
    ) -> Vec<SteerMessage> {
        let queues = container.session_state.steering_queues.lock().await;
        queues.get(session_id).cloned().unwrap_or_default()
    }

    /// Remove a single steering message by id. Returns true if it was present.
    pub async fn delete_steer(
        container: &ServiceContainer,
        session_id: &SessionId,
        steer_id: &str,
    ) -> bool {
        let mut queues = container.session_state.steering_queues.lock().await;
        let Some(queue) = queues.get_mut(session_id) else {
            return false;
        };
        let before = queue.len();
        queue.retain(|s| s.id != steer_id);
        queue.len() != before
    }

    /// Take all pending steering messages for a session, leaving it empty.
    pub async fn drain_steers(
        container: &ServiceContainer,
        session_id: &SessionId,
    ) -> Vec<SteerMessage> {
        let mut queues = container.session_state.steering_queues.lock().await;
        queues
            .get_mut(session_id)
            .map(std::mem::take)
            .unwrap_or_default()
    }

    /// Clear a session's steering queue (called at run start).
    pub async fn clear_steers(container: &ServiceContainer, session_id: &SessionId) {
        let mut queues = container.session_state.steering_queues.lock().await;
        queues.remove(session_id);
    }

    // -- Follow-up queue management ----------------------------------------
    //
    // Per-session FIFO queues of user messages enqueued while a turn is
    // streaming but intended for processing after the run naturally stops.
    // The agent execution loop drains them after the inner loop exits
    // (no tool calls, no steering). When non-empty, the run continues.

    /// Enqueue a follow-up message for a session. Returns the created entry.
    pub async fn add_follow_up(
        container: &ServiceContainer,
        session_id: &SessionId,
        text: String,
    ) -> FollowUpMessage {
        let msg = FollowUpMessage::new(text);
        let mut queues = container.session_state.follow_up_queues.lock().await;
        queues
            .entry(session_id.clone())
            .or_default()
            .push(msg.clone());
        msg
    }

    /// List the pending follow-up messages for a session (FIFO order).
    pub async fn list_follow_ups(
        container: &ServiceContainer,
        session_id: &SessionId,
    ) -> Vec<FollowUpMessage> {
        let queues = container.session_state.follow_up_queues.lock().await;
        queues.get(session_id).cloned().unwrap_or_default()
    }

    /// Remove a single follow-up message by id. Returns true if it was present.
    pub async fn delete_follow_up(
        container: &ServiceContainer,
        session_id: &SessionId,
        follow_up_id: &str,
    ) -> bool {
        let mut queues = container.session_state.follow_up_queues.lock().await;
        let Some(queue) = queues.get_mut(session_id) else {
            return false;
        };
        let before = queue.len();
        queue.retain(|f| f.id != follow_up_id);
        queue.len() != before
    }

    /// Take all pending follow-up messages for a session, leaving it empty.
    pub async fn drain_follow_ups(
        container: &ServiceContainer,
        session_id: &SessionId,
    ) -> Vec<FollowUpMessage> {
        let mut queues = container.session_state.follow_up_queues.lock().await;
        queues
            .get_mut(session_id)
            .map(std::mem::take)
            .unwrap_or_default()
    }

    /// Clear a session's follow-up queue (called at run start).
    pub async fn clear_follow_ups(container: &ServiceContainer, session_id: &SessionId) {
        let mut queues = container.session_state.follow_up_queues.lock().await;
        queues.remove(session_id);
    }

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
            tool_dialect: y_core::provider::ToolDialect::default(),
            messages: input.history.to_vec(),
            provider_id: input.provider_id.clone(),
            preferred_models: input.preferred_models.clone(),
            provider_tags: input.provider_tags.clone(),
            fallback_provider_tags: vec![],
            request_mode: input.request_mode,
            working_directory: input.working_directory.clone(),
            additional_read_dirs: vec![],
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
            inherited_constraints: None,
        }
    }

    async fn root_additional_read_dirs(container: &ServiceContainer) -> Vec<String> {
        let (plan_mode_active, loop_mode_active) = {
            let pctx = container.prompt_context.read().await;
            (
                pctx.config_flags
                    .get("plan_mode.active")
                    .copied()
                    .unwrap_or(false),
                pctx.config_flags
                    .get("loop_mode.active")
                    .copied()
                    .unwrap_or(false),
            )
        };

        let mut dirs = Vec::new();
        if plan_mode_active {
            dirs.push(container.data_dir.join("plan").display().to_string());
        }
        if loop_mode_active {
            dirs.push(container.data_dir.join("loop").display().to_string());
        }
        dirs
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
                pctx.config_flags.remove("loop_mode.active");
                tracing::info!("plan_mode.active flag SET in prompt context");
            }
            "loop" => {
                let mut pctx = container.prompt_context.write().await;
                if input.request_mode == RequestMode::TextChat {
                    pctx.config_flags.insert("loop_mode.active".into(), true);
                } else {
                    pctx.config_flags.remove("loop_mode.active");
                }
                pctx.config_flags.remove("plan_mode.active");
                tracing::info!("loop_mode.active flag SET in prompt context");
            }
            "auto" => {
                if input.request_mode == RequestMode::TextChat {
                    let classification = crate::plan_orchestrator::assess_complexity(
                        container,
                        input.user_input,
                        input.provider_id.as_deref(),
                    )
                    .await;
                    let mut pctx = container.prompt_context.write().await;
                    match classification.as_str() {
                        "plan" => {
                            pctx.config_flags.insert("plan_mode.active".into(), true);
                            pctx.config_flags.remove("loop_mode.active");
                            tracing::info!("plan_mode.active flag SET (auto: complex)");
                        }
                        "loop" => {
                            pctx.config_flags.insert("loop_mode.active".into(), true);
                            pctx.config_flags.remove("plan_mode.active");
                            tracing::info!("loop_mode.active flag SET (auto: iterative)");
                        }
                        _ => {
                            pctx.config_flags.remove("plan_mode.active");
                            pctx.config_flags.remove("loop_mode.active");
                            tracing::info!("no mode flags set (auto: simple)");
                        }
                    }
                }
            }
            _ => {
                let mut pctx = container.prompt_context.write().await;
                pctx.config_flags.remove("plan_mode.active");
                pctx.config_flags.remove("loop_mode.active");
                tracing::info!("plan/loop mode flags CLEARED (fast mode)");
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

    /// Resolve turn configuration fields from request overrides and agent config.
    ///
    /// Request fields take priority; agent config fields are the fallback.
    /// `request_mcp_mode` / `request_mcp_servers` are `None` in the resend path
    /// (which has no MCP overrides in the request) -- they fall back to agent
    /// config.
    #[allow(clippy::too_many_arguments)]
    async fn resolve_turn_config(
        container: &ServiceContainer,
        session_id: &SessionId,
        request_provider_id: Option<&str>,
        request_thinking: Option<&ThinkingConfig>,
        request_plan_mode: Option<&str>,
        request_operation_mode: Option<OperationMode>,
        request_mcp_mode: Option<&str>,
        request_mcp_servers: Option<&[String]>,
        agent_config: Option<&SessionAgentConfig>,
    ) -> ResolvedTurnConfig {
        let provider_id = request_provider_id
            .map(ToOwned::to_owned)
            .or_else(|| agent_config.and_then(|c| c.provider_id.clone()));
        let thinking = request_thinking
            .cloned()
            .or_else(|| agent_config.and_then(|c| c.thinking.clone()));
        let plan_mode = request_plan_mode
            .map(ToOwned::to_owned)
            .or_else(|| agent_config.and_then(|c| c.plan_mode.clone()));
        let operation_mode = request_operation_mode.unwrap_or_default();
        {
            let mut modes = container
                .session_state
                .session_operation_modes
                .write()
                .await;
            modes.insert(session_id.clone(), operation_mode);
        }
        let mcp_mode = request_mcp_mode
            .map(ToOwned::to_owned)
            .or_else(|| agent_config.and_then(|c| c.mcp_mode.clone()));
        let mcp_servers = request_mcp_servers.map_or_else(
            || agent_config.map_or_else(Vec::new, |c| c.mcp_servers.clone()),
            ToOwned::to_owned,
        );
        let working_directory = agent_config.and_then(|c| c.working_directory.clone());
        ResolvedTurnConfig {
            provider_id,
            thinking,
            plan_mode,
            operation_mode,
            mcp_mode,
            mcp_servers,
            working_directory,
        }
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
        // A fresh run starts with an empty steering queue. Steers are enqueued
        // by the client only while a run is streaming, so clearing here just
        // guards against stale entries leaking across runs.
        Self::clear_steers(container, &session_id).await;
        Self::clear_follow_ups(container, &session_id).await;
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
        let turn_cfg = Self::resolve_turn_config(
            container,
            &session_id,
            request.provider_id.as_deref(),
            request.thinking.as_ref(),
            request.plan_mode.as_deref(),
            request.operation_mode,
            request.mcp_mode.as_deref(),
            request.mcp_servers.as_deref(),
            agent_config.as_ref(),
        )
        .await;
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
            provider_id: turn_cfg.provider_id,
            request_mode,
            session_created,
            working_directory: turn_cfg.working_directory,
            knowledge_collections,
            thinking: turn_cfg.thinking,
            plan_mode: turn_cfg.plan_mode,
            operation_mode: turn_cfg.operation_mode,
            mcp_mode: turn_cfg.mcp_mode,
            mcp_servers: turn_cfg.mcp_servers,
            skills,
            agent_config,
            image_generation_options: request.image_generation_options,
            pre_turn_message_count: None,
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
        // Start the resend run with an empty steering queue (see prepare_turn).
        Self::clear_steers(container, &request.session_id).await;
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

        // 2. Read display transcript to detect intra-turn retry (partial state
        //    from a failed LLM call that had already executed some tool calls).
        let display_msgs = container
            .session_manager
            .read_display_transcript(&request.session_id)
            .await
            .map_err(|e| ResendTurnError::TranscriptReadFailed(e.to_string()))?;

        let is_intra_turn_retry = display_msgs.last().is_some_and(|msg| {
            msg.role == Role::Assistant
                && msg
                    .metadata
                    .get("stream_error")
                    .and_then(|v| v.as_str())
                    .is_some_and(|s| !s.is_empty())
        });

        if is_intra_turn_retry {
            // -- Intra-turn retry: keep partial state, continue from the last
            //    successfully executed tool call. --

            // 2a. Remove the error-marked message from the display transcript.
            let display_len = display_msgs.len();
            container
                .session_manager
                .display_transcript_store()
                .truncate(&request.session_id, display_len.saturating_sub(1))
                .await
                .map_err(|e| ResendTurnError::TruncateFailed(e.to_string()))?;

            // 2b. Do NOT truncate the context transcript -- it already has the
            //     partial state (assistant + tool messages) from
            //     persist_llm_error_partial_state.

            // 2c. Do NOT invalidate the checkpoint -- the turn boundary hasn't
            //     changed; we are continuing the same turn.

            // 3. Read context transcript (includes partial state).
            let history = container
                .session_manager
                .read_transcript(&request.session_id)
                .await
                .map_err(|e| ResendTurnError::TranscriptReadFailed(e.to_string()))?;

            if history.is_empty() {
                return Err(ResendTurnError::TranscriptEmpty);
            }

            // 4. Find the user message at the checkpoint's message_count_before
            //    index (the same index used for turn-level truncation).
            let user_msg_index = checkpoint.message_count_before as usize;
            let Some(user_msg) = history.get(user_msg_index) else {
                return Err(ResendTurnError::TranscriptEmpty);
            };
            if user_msg.role != Role::User {
                return Err(ResendTurnError::TruncateFailed(format!(
                    "expected user message at index {} in intra-turn retry, found {:?}",
                    user_msg_index, user_msg.role
                )));
            }

            let requested_skills = user_msg
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
            let turn_cfg = Self::resolve_turn_config(
                container,
                &request.session_id,
                request.provider_id.as_deref(),
                request.thinking.as_ref(),
                request.plan_mode.as_deref(),
                request.operation_mode,
                None,
                None,
                agent_config.as_ref(),
            )
            .await;
            let request_mode = request
                .request_mode
                .or_else(|| Self::request_mode_from_metadata(&user_msg.metadata))
                .unwrap_or_default();
            let user_input = user_msg.content.clone();

            // Derive turn number from display transcript (after removing the
            // error-marked message) for checkpoint consistency.
            let display_len_after = display_len.saturating_sub(1);
            let turn_number = u32::try_from(display_len_after).unwrap_or(0);
            let session_uuid =
                Uuid::parse_str(request.session_id.as_str()).unwrap_or_else(|_| Uuid::new_v4());

            return Ok(PreparedTurn {
                session_id: request.session_id,
                session_uuid,
                history,
                turn_number,
                user_input,
                provider_id: turn_cfg.provider_id,
                request_mode,
                session_created: false,
                working_directory: turn_cfg.working_directory,
                knowledge_collections,
                thinking: turn_cfg.thinking,
                plan_mode: turn_cfg.plan_mode,
                operation_mode: turn_cfg.operation_mode,
                mcp_mode: turn_cfg.mcp_mode,
                mcp_servers: turn_cfg.mcp_servers,
                skills,
                agent_config,
                image_generation_options: None,
                pre_turn_message_count: Some(checkpoint.message_count_before),
            });
        }

        // -- Turn-level retry: restart from the user message (existing behavior). --

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
        let turn_cfg = Self::resolve_turn_config(
            container,
            &request.session_id,
            request.provider_id.as_deref(),
            request.thinking.as_ref(),
            request.plan_mode.as_deref(),
            request.operation_mode,
            None,
            None,
            agent_config.as_ref(),
        )
        .await;
        let request_mode = request
            .request_mode
            .or_else(|| Self::request_mode_from_metadata(&last_msg.metadata))
            .unwrap_or_default();
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
            provider_id: turn_cfg.provider_id,
            request_mode,
            session_created: false,
            working_directory: turn_cfg.working_directory,
            knowledge_collections,
            thinking: turn_cfg.thinking,
            plan_mode: turn_cfg.plan_mode,
            operation_mode: turn_cfg.operation_mode,
            mcp_mode: turn_cfg.mcp_mode,
            mcp_servers: turn_cfg.mcp_servers,
            skills,
            agent_config,
            image_generation_options: None,
            pre_turn_message_count: None,
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
        let (last_cache_read, last_cache_write) = last_gen.map_or((0, 0), |o| {
            let read = o
                .metadata
                .get("cache_read_tokens")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            let write = o
                .metadata
                .get("cache_write_tokens")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            (read, write)
        });
        // Context occupancy is the total prompt size: fresh input plus cache.
        let last_gen_input_tokens = last_gen
            .map_or(0, |o| o.input_tokens)
            .saturating_add(last_cache_read)
            .saturating_add(last_cache_write);

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
            cache_read_tokens: last_cache_read,
            cache_write_tokens: last_cache_write,
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

        // 1c. Inject Plan/Loop tool schema when respective mode is active.
        if input.request_mode == RequestMode::TextChat {
            Self::apply_plan_mode_tool_adjustments(container, &mut tool_defs).await;
            Self::apply_loop_mode_tool_adjustments(container, &mut tool_defs).await;
        }

        // 2. Construct execution config for the root agent.
        let max_tool_iterations = container.guardrail_manager.config().max_tool_iterations;
        let mut exec_config =
            Self::build_execution_config(input, tool_defs, tool_calling_mode, max_tool_iterations);
        exec_config.additional_read_dirs = Self::root_additional_read_dirs(container).await;

        // 3. Delegate to AgentService.
        let mut result =
            match AgentService::execute(container, &exec_config, progress, cancel).await {
                Ok(r) => r,
                Err(AgentExecutionError::LlmError {
                    message,
                    provider_error: _,
                    partial_messages,
                }) => {
                    Self::persist_llm_error_partial_state(
                        container,
                        input,
                        &message,
                        &partial_messages,
                    )
                    .await;
                    if !partial_messages.is_empty() {
                        tracing::info!(
                            count = partial_messages.len(),
                            session = %input.session_id.0,
                            "persisted partial messages before LLM error"
                        );
                    }
                    // Record the turn boundary so a retry can anchor to it and
                    // resume from the partial tool-call state, instead of falling
                    // back to a destructive full-turn resend.
                    Self::persist_turn_checkpoint(container, input).await;
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

        // Persist the assistant output. With steering this is an interleaved
        // sequence of assistant segments and injected user-message bubbles;
        // without steering it is a single consolidated assistant message.
        let messages = Self::build_steered_messages(&result);
        for msg in &messages {
            if let Err(e) = container
                .session_manager
                .append_message(&input.session_id, msg)
                .await
            {
                tracing::warn!(
                    error = %e,
                    session_id = %input.session_id,
                    role = ?msg.role,
                    "failed to persist turn message to session transcript"
                );
            }
        }

        // The final assistant segment carries the turn-level metadata; use it
        // for the pruning-engine mirror and the returned `new_messages`.
        let assistant_msg = messages
            .last()
            .cloned()
            .expect("build_steered_messages always yields a final assistant message");

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

        let mut new_messages = std::mem::take(&mut result.new_messages);
        new_messages.push(assistant_msg);

        // Checkpoint the completed turn boundary.
        Self::persist_turn_checkpoint(container, input).await;

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
            last_cache_read_tokens: result.last_cache_read_tokens,
            last_cache_write_tokens: result.last_cache_write_tokens,
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

    /// Split a turn's consolidated assistant output into segments at each
    /// steering-injection boundary, interleaving the injected user messages so
    /// the persisted transcript reads `[asst seg][steer][asst seg]...`.
    ///
    /// With no steers (the common case) this returns a single assistant message
    /// identical to the non-steered consolidation.
    fn build_steered_messages(result: &AgentExecutionResult) -> Vec<Message> {
        let total_blocks = result.iteration_texts.len();
        let mut messages = Vec::new();
        let mut prev = 0usize;
        let mut idx = 0usize;
        let steers = &result.injected_steers;

        while idx < steers.len() {
            let gap = steers[idx].after_iteration.min(total_blocks);
            // Assistant segment for blocks [prev, gap); skip empty ranges (e.g.
            // multiple steers at the same boundary, or a steer before any text).
            if gap > prev {
                messages.push(Self::build_segment_message(result, prev, gap, false));
                prev = gap;
            }
            // Emit every steer anchored at this boundary, in injection order.
            while idx < steers.len() && steers[idx].after_iteration.min(total_blocks) == gap {
                messages.push(steers[idx].message.clone());
                idx += 1;
            }
        }

        // Final segment: remaining blocks plus the final response + turn metadata.
        messages.push(Self::build_segment_message(
            result,
            prev,
            total_blocks,
            true,
        ));
        messages
    }

    /// Build one assistant message covering iteration blocks `[start, end)`.
    /// When `is_final`, appends the final response text and attaches the
    /// turn-level token/cost/reasoning metadata (matching the non-steered
    /// consolidated message).
    fn build_segment_message(
        result: &AgentExecutionResult,
        start: usize,
        end: usize,
        is_final: bool,
    ) -> Message {
        let tool_start: usize = result.iteration_tool_counts[..start].iter().sum();
        let tool_end: usize = result.iteration_tool_counts[..end].iter().sum();
        let tool_results_meta =
            Self::build_tool_results_metadata(&result.tool_calls_executed[tool_start..tool_end]);

        let mut content = result.iteration_texts[start..end].concat();
        if is_final {
            content.push_str(&result.final_response);
        }

        let mut meta = serde_json::json!({
            "model": result.model,
            "context_window": result.context_window,
            "tool_results": tool_results_meta,
            "iteration_texts": &result.iteration_texts[start..end],
            "iteration_reasonings": &result.iteration_reasonings[start..end],
            "iteration_reasoning_durations_ms": &result.iteration_reasoning_durations_ms[start..end],
            "iteration_tool_counts": &result.iteration_tool_counts[start..end],
        });

        if is_final {
            meta["input_tokens"] = serde_json::json!(result.input_tokens);
            meta["output_tokens"] = serde_json::json!(result.output_tokens);
            meta["cost_usd"] = serde_json::json!(result.cost_usd);
            meta["context_tokens_used"] = serde_json::json!(result.last_input_tokens);
            meta["cache_read_tokens"] = serde_json::json!(result.last_cache_read_tokens);
            meta["cache_write_tokens"] = serde_json::json!(result.last_cache_write_tokens);
            meta["final_response"] = serde_json::json!(result.final_response);

            if !result.generated_images.is_empty() {
                meta["generated_images"] = serde_json::to_value(&result.generated_images)
                    .unwrap_or(serde_json::Value::Array(vec![]));
            }

            // Preserve reasoning_content: prefer the direct field, then fall back
            // to scanning new_messages for an earlier iteration's reasoning.
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

            if let Some(rd) = result.reasoning_duration_ms {
                meta["reasoning_duration_ms"] = serde_json::json!(rd);
            }
        }

        Message {
            message_id: y_core::types::generate_message_id(),
            role: Role::Assistant,
            content,
            tool_call_id: None,
            tool_calls: vec![],
            timestamp: y_core::types::now(),
            metadata: meta,
        }
    }

    /// Create (or refresh) the chat checkpoint marking this turn's boundary.
    ///
    /// Called from both the success path and the LLM-error path so that a
    /// failed turn still has a checkpoint to anchor an intra-turn retry. The
    /// checkpoint manager keys on `(session_id, turn_number)`, so calling this
    /// twice for the same turn (failure then a successful retry) reuses the
    /// same checkpoint slot rather than creating a duplicate.
    ///
    /// When `pre_turn_message_count` is set (intra-turn retry), the history
    /// includes partial tool-call state from the failed attempt, so
    /// `history.len() - 1` would overcount; the original pre-turn count is used.
    async fn persist_turn_checkpoint(container: &ServiceContainer, input: &TurnInput<'_>) {
        let msg_count_before = input
            .pre_turn_message_count
            .unwrap_or_else(|| u32::try_from(input.history.len().saturating_sub(1)).unwrap_or(0));
        let turn = input.turn_number + 1;
        let scope_id = format!("turn-{}-{}", input.session_id.0, turn);
        if let Err(e) = container
            .chat_checkpoint_manager
            .create_checkpoint(&input.session_id, turn, msg_count_before, scope_id)
            .await
        {
            tracing::warn!(error = %e, "failed to create chat checkpoint");
        }
    }

    async fn persist_llm_error_partial_state(
        container: &ServiceContainer,
        input: &TurnInput<'_>,
        error_message: &str,
        partial_messages: &[Message],
    ) {
        let ctx_store = container.session_manager.transcript_store();
        for msg in partial_messages {
            let _ = ctx_store.append(&input.session_id, msg).await;
        }

        let tool_calls = Self::extract_tool_call_records(partial_messages);
        let accumulated_content = Self::accumulate_assistant_content(partial_messages);

        let display_store = container.session_manager.display_transcript_store();

        // Persist completed work (text + executed tool calls) as a standalone
        // consolidated message WITHOUT `stream_error`, so it survives an
        // intra-turn retry instead of being discarded with the failure.
        // Skip this only when there is genuinely no work to show -- but still
        // append the failure marker below so the retry path can detect an
        // intra-turn retry and preserve earlier partial state.
        let has_work = !accumulated_content.trim().is_empty() || !tool_calls.is_empty();
        if has_work {
            let success_metadata = serde_json::json!({
                "tool_results": Self::build_tool_results_metadata(&tool_calls),
                "iteration_texts": Self::assistant_iteration_texts(partial_messages),
                "iteration_reasonings": Self::assistant_iteration_reasonings(partial_messages),
                "iteration_reasoning_durations_ms": Vec::<Option<u64>>::new(),
                "iteration_tool_counts": Self::assistant_iteration_tool_counts(partial_messages),
            });
            let success_msg = Message {
                message_id: y_core::types::generate_message_id(),
                role: Role::Assistant,
                content: accumulated_content,
                tool_call_id: None,
                tool_calls: vec![],
                timestamp: y_core::types::now(),
                metadata: success_metadata,
            };
            let _ = display_store.append(&input.session_id, &success_msg).await;
        }

        // Always append the failure marker as the LAST display message. Both
        // the frontend and `prepare_resend_turn` detect an intra-turn retry by
        // a trailing assistant message carrying a non-empty `stream_error`,
        // and the intra-turn truncate removes ONLY this trailing marker --
        // which keeps the successful-iteration message above intact. Without
        // this marker, a retry falls through to the destructive turn-level
        // branch and wipes all partial work from the display transcript.
        let failure_marker = Message {
            message_id: y_core::types::generate_message_id(),
            role: Role::Assistant,
            content: String::new(),
            tool_call_id: None,
            tool_calls: vec![],
            timestamp: y_core::types::now(),
            metadata: serde_json::json!({ "stream_error": error_message }),
        };
        let _ = display_store
            .append(&input.session_id, &failure_marker)
            .await;
    }

    fn accumulate_assistant_content(messages: &[Message]) -> String {
        messages
            .iter()
            .filter(|msg| msg.role == Role::Assistant)
            .map(|msg| msg.content.as_str())
            .collect()
    }

    fn assistant_iteration_texts(messages: &[Message]) -> Vec<String> {
        messages
            .iter()
            .filter(|msg| msg.role == Role::Assistant)
            .map(|msg| msg.content.clone())
            .collect()
    }

    fn assistant_iteration_reasonings(messages: &[Message]) -> Vec<Option<String>> {
        messages
            .iter()
            .filter(|msg| msg.role == Role::Assistant)
            .map(|msg| {
                msg.metadata
                    .get("reasoning_content")
                    .and_then(serde_json::Value::as_str)
                    .map(String::from)
            })
            .collect()
    }

    fn assistant_iteration_tool_counts(messages: &[Message]) -> Vec<usize> {
        messages
            .iter()
            .filter(|msg| msg.role == Role::Assistant)
            .map(|msg| msg.tool_calls.len())
            .collect()
    }

    fn extract_tool_call_records(messages: &[Message]) -> Vec<ToolCallRecord> {
        let mut records = Vec::new();
        for assistant in messages
            .iter()
            .filter(|msg| msg.role == Role::Assistant && !msg.tool_calls.is_empty())
        {
            for tool_call in &assistant.tool_calls {
                let tool_result = messages.iter().find(|msg| {
                    msg.role == Role::Tool
                        && msg.tool_call_id.as_deref() == Some(tool_call.id.as_str())
                });

                records.push(ToolCallRecord {
                    name: tool_call.name.clone(),
                    arguments: serde_json::to_string(&tool_call.arguments).unwrap_or_default(),
                    success: tool_result
                        .is_some_and(|msg| tool_result_success_from_content(&msg.content)),
                    duration_ms: 0,
                    result_content: tool_result.map_or_else(
                        || {
                            serde_json::json!({
                                "error": "Tool result was not recorded before the LLM call failed."
                            })
                            .to_string()
                        },
                        |msg| msg.content.clone(),
                    ),
                    url_meta: None,
                    metadata: None,
                });
            }
        }
        records
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

    async fn apply_loop_mode_tool_adjustments(
        container: &ServiceContainer,
        tool_defs: &mut Vec<serde_json::Value>,
    ) {
        let is_active = {
            let pctx = container.prompt_context.read().await;
            pctx.config_flags
                .get("loop_mode.active")
                .copied()
                .unwrap_or(false)
        };
        if !is_active {
            return;
        }

        let already_present = tool_defs.iter().any(|def| {
            def.get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                == Some("Loop")
        });
        if already_present {
            return;
        }

        let tn = y_core::types::ToolName::from_string("Loop");
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
            "loop mode: injected Loop tool schema"
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

    /// Pure decision for title generation: enabled when `title_interval` is
    /// non-zero and the history contains at least one user message to
    /// summarize.
    ///
    /// `title_summarize_interval` is now an on/off switch rather than a cadence:
    /// any non-zero value means "regenerate on every user message". The title
    /// only consumes user messages, so it no longer needs to wait for the
    /// assistant turn or throttle to every-N turns.
    fn title_generation_enabled(title_interval: u32, history: &[Message]) -> bool {
        title_interval != 0 && history.iter().any(|m| m.role == Role::User)
    }

    /// Determine whether title generation should be triggered for this send.
    ///
    /// Generates a title on every user message; disabled entirely when
    /// `title_summarize_interval` is 0.
    pub fn should_generate_title(container: &ServiceContainer, history: &[Message]) -> bool {
        let title_interval = container.session_manager.config().title_summarize_interval;
        Self::title_generation_enabled(title_interval, history)
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
            has_tool_calls: !msg.tool_calls.is_empty(),
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

    /// Persist a sub-agent run (plan phase, loop round, plan-writer, ...) to its
    /// own child session transcript, using the SAME message assembly the main
    /// turn uses so the child session renders identically in the GUI.
    ///
    /// Unlike the root turn, sub-agents never steer and get no checkpoint /
    /// post-turn optimization — this only records the initiating prompt and the
    /// resulting assistant message(s) so the child session is a faithful,
    /// drill-in-able transcript.
    pub(crate) async fn persist_subagent_turn(
        container: &ServiceContainer,
        session_id: &SessionId,
        user_input: &str,
        result: &AgentExecutionResult,
    ) {
        let user_msg = Message {
            message_id: y_core::types::generate_message_id(),
            role: Role::User,
            content: user_input.to_string(),
            tool_call_id: None,
            tool_calls: vec![],
            timestamp: y_core::types::now(),
            metadata: serde_json::json!({}),
        };
        if let Err(e) = container
            .session_manager
            .append_message(session_id, &user_msg)
            .await
        {
            tracing::warn!(error = %e, session_id = %session_id, "failed to persist sub-agent prompt");
        }

        let messages = Self::build_steered_messages(result);
        for msg in &messages {
            if let Err(e) = container
                .session_manager
                .append_message(session_id, msg)
                .await
            {
                tracing::warn!(error = %e, session_id = %session_id, "failed to persist sub-agent message");
            }
        }

        if let Some(assistant_msg) = messages.last() {
            Self::mirror_to_chat_message_store(
                container,
                session_id,
                assistant_msg,
                Some(&result.model),
                Some(result.input_tokens),
                Some(result.output_tokens),
                Some(result.cost_usd),
                Some(result.context_window),
            )
            .await;
        }
    }
}

fn tool_result_success_from_content(content: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(content).map_or(true, |value| {
        value.get("error").is_none_or(serde_json::Value::is_null)
    })
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

    /// Build a history containing `user_count` user messages, each followed by
    /// an assistant reply, mirroring a real multi-turn conversation.
    fn history_with_user_messages(user_count: usize) -> Vec<Message> {
        let mut history = Vec::new();
        for i in 0..user_count {
            history.push(Message {
                message_id: y_core::types::generate_message_id(),
                role: Role::User,
                content: format!("user message {i}"),
                tool_call_id: None,
                tool_calls: vec![],
                timestamp: y_core::types::now(),
                metadata: serde_json::Value::Null,
            });
            history.push(Message {
                message_id: y_core::types::generate_message_id(),
                role: Role::Assistant,
                content: format!("assistant reply {i}"),
                tool_call_id: None,
                tool_calls: vec![],
                timestamp: y_core::types::now(),
                metadata: serde_json::Value::Null,
            });
        }
        history
    }

    #[test]
    fn test_title_generation_disabled_when_interval_zero() {
        let history = history_with_user_messages(3);
        assert!(!ChatService::title_generation_enabled(0, &history));
    }

    #[test]
    fn test_title_generation_skipped_without_user_messages() {
        let history: Vec<Message> = Vec::new();
        assert!(!ChatService::title_generation_enabled(3, &history));
    }

    #[test]
    fn test_title_generation_fires_on_first_user_message() {
        let history = history_with_user_messages(1);
        assert!(ChatService::title_generation_enabled(3, &history));
    }

    #[test]
    fn test_title_generation_fires_on_every_user_message() {
        // Two user messages with the default interval of 3 previously returned
        // false (2 != 1 and 2 % 3 != 0). It must now fire on every message.
        let history = history_with_user_messages(2);
        assert!(ChatService::title_generation_enabled(3, &history));
    }

    #[test]
    fn test_title_generation_ignores_interval_magnitude() {
        let history = history_with_user_messages(4);
        assert!(ChatService::title_generation_enabled(1, &history));
        assert!(ChatService::title_generation_enabled(7, &history));
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
    fn test_extract_tool_call_records_preserves_json_error_object() {
        let tool_call = y_core::types::ToolCallRequest {
            id: "call_123".to_string(),
            name: "FileRead".to_string(),
            arguments: serde_json::json!({ "path": "/missing.rs" }),
        };
        let messages = vec![
            Message {
                message_id: y_core::types::generate_message_id(),
                role: Role::Assistant,
                content: "I will inspect that file.\n".to_string(),
                tool_call_id: None,
                tool_calls: vec![tool_call],
                timestamp: y_core::types::now(),
                metadata: serde_json::Value::Null,
            },
            Message {
                message_id: y_core::types::generate_message_id(),
                role: Role::Tool,
                content: serde_json::json!({
                    "error": "file not found: /missing.rs",
                    "retryable": false,
                })
                .to_string(),
                tool_call_id: Some("call_123".to_string()),
                tool_calls: vec![],
                timestamp: y_core::types::now(),
                metadata: serde_json::Value::Null,
            },
        ];

        let records = ChatService::extract_tool_call_records(&messages);

        assert_eq!(records.len(), 1);
        assert!(!records[0].success);
        let content: serde_json::Value = serde_json::from_str(&records[0].result_content).unwrap();
        assert!(content.is_object());
        assert_eq!(
            content.get("error").and_then(serde_json::Value::as_str),
            Some("file not found: /missing.rs")
        );
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
            operation_mode: OperationMode::Default,
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
            pre_turn_message_count: None,
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
            operation_mode: OperationMode::Default,
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
            pre_turn_message_count: None,
        };

        let config =
            ChatService::build_execution_config(&input, vec![], ToolCallingMode::default(), 8);
        assert_eq!(config.temperature, Some(1.0));
        assert_eq!(config.working_directory.as_deref(), Some("/repo/workspace"));
    }

    #[tokio::test]
    async fn test_root_additional_read_dirs_include_plan_dir_when_plan_mode_active() {
        let (container, _tmp) = make_test_container().await;
        {
            let mut pctx = container.prompt_context.write().await;
            pctx.config_flags.insert("plan_mode.active".into(), true);
        }

        let dirs = ChatService::root_additional_read_dirs(&container).await;

        assert_eq!(
            dirs,
            vec![container.data_dir.join("plan").display().to_string()]
        );
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
    async fn persist_subagent_turn_writes_prompt_and_assistant_to_child_session() {
        let (container, _tmp) = make_test_container().await;
        let parent = container
            .session_manager
            .create_session(y_core::session::CreateSessionOptions {
                parent_id: None,
                session_type: y_core::session::SessionType::Main,
                agent_id: None,
                title: Some("parent".into()),
            })
            .await
            .expect("parent session");
        let child = container
            .session_manager
            .create_session(y_core::session::CreateSessionOptions {
                parent_id: Some(parent.id.clone()),
                session_type: y_core::session::SessionType::SubAgent,
                agent_id: None,
                title: Some("Phase 1: demo".into()),
            })
            .await
            .expect("child session");

        let result = make_steer_result(vec!["working\n".into()], vec![0], "phase done", vec![]);
        ChatService::persist_subagent_turn(&container, &child.id, "Phase 1: demo", &result).await;

        let msgs = container
            .session_manager
            .read_display_transcript(&child.id)
            .await
            .expect("read child transcript");

        assert_eq!(msgs.len(), 2, "expected user prompt + assistant message");
        assert_eq!(msgs[0].role, Role::User);
        assert_eq!(msgs[0].content, "Phase 1: demo");
        assert_eq!(msgs[1].role, Role::Assistant);
        assert_eq!(msgs[1].content, "working\nphase done");
    }

    #[tokio::test]
    async fn steer_queue_add_list_preserves_fifo_order() {
        let (container, _tmp) = make_test_container().await;
        let sid = SessionId("steer-sess".into());

        let a = ChatService::add_steer(&container, &sid, "first".into()).await;
        let b = ChatService::add_steer(&container, &sid, "second".into()).await;

        let listed = ChatService::list_steers(&container, &sid).await;
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].id, a.id);
        assert_eq!(listed[0].text, "first");
        assert_eq!(listed[1].id, b.id);
        assert_eq!(listed[1].text, "second");
    }

    #[tokio::test]
    async fn steer_queue_delete_removes_only_matching_id() {
        let (container, _tmp) = make_test_container().await;
        let sid = SessionId("steer-sess".into());
        let a = ChatService::add_steer(&container, &sid, "keep".into()).await;
        let b = ChatService::add_steer(&container, &sid, "drop".into()).await;

        assert!(ChatService::delete_steer(&container, &sid, &b.id).await);
        assert!(!ChatService::delete_steer(&container, &sid, "missing").await);

        let listed = ChatService::list_steers(&container, &sid).await;
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, a.id);
    }

    #[tokio::test]
    async fn steer_queue_drain_takes_all_and_empties() {
        let (container, _tmp) = make_test_container().await;
        let sid = SessionId("steer-sess".into());
        ChatService::add_steer(&container, &sid, "one".into()).await;
        ChatService::add_steer(&container, &sid, "two".into()).await;

        let drained = ChatService::drain_steers(&container, &sid).await;
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].text, "one");
        assert_eq!(drained[1].text, "two");

        assert!(ChatService::list_steers(&container, &sid).await.is_empty());
    }

    #[tokio::test]
    async fn steer_queue_clear_and_isolation_between_sessions() {
        let (container, _tmp) = make_test_container().await;
        let sid_a = SessionId("sess-a".into());
        let sid_b = SessionId("sess-b".into());
        ChatService::add_steer(&container, &sid_a, "a1".into()).await;
        ChatService::add_steer(&container, &sid_b, "b1".into()).await;

        ChatService::clear_steers(&container, &sid_a).await;

        assert!(ChatService::list_steers(&container, &sid_a)
            .await
            .is_empty());
        let b = ChatService::list_steers(&container, &sid_b).await;
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].text, "b1");
    }

    fn steer_user_msg(text: &str) -> Message {
        Message {
            message_id: y_core::types::generate_message_id(),
            role: Role::User,
            content: text.to_string(),
            tool_call_id: None,
            tool_calls: vec![],
            timestamp: y_core::types::now(),
            metadata: serde_json::Value::Null,
        }
    }

    fn make_steer_result(
        iteration_texts: Vec<String>,
        iteration_tool_counts: Vec<usize>,
        final_response: &str,
        injected_steers: Vec<crate::agent_service::InjectedSteer>,
    ) -> AgentExecutionResult {
        let content = format!("{}{final_response}", iteration_texts.concat());
        let n = iteration_texts.len();
        let total_tools: usize = iteration_tool_counts.iter().sum();
        AgentExecutionResult {
            content,
            model: "test-model".into(),
            provider_id: None,
            input_tokens: 10,
            output_tokens: 20,
            last_input_tokens: 5,
            last_cache_read_tokens: 0,
            last_cache_write_tokens: 0,
            context_window: 1000,
            cost_usd: 0.1,
            tool_calls_executed: (0..total_tools)
                .map(|i| crate::agent_service::ToolCallRecord {
                    name: format!("tool{i}"),
                    arguments: "{}".into(),
                    success: true,
                    duration_ms: 1,
                    result_content: String::new(),
                    url_meta: None,
                    metadata: None,
                })
                .collect(),
            iterations: n,
            generated_images: vec![],
            new_messages: vec![],
            final_response: final_response.to_string(),
            iteration_texts,
            iteration_reasonings: vec![None; n],
            iteration_reasoning_durations_ms: vec![None; n],
            iteration_tool_counts,
            reasoning_content: None,
            reasoning_duration_ms: None,
            injected_steers,
        }
    }

    #[test]
    fn build_steered_messages_no_steers_yields_single_message() {
        let result = make_steer_result(vec!["alpha\n".into()], vec![1], "final answer", vec![]);
        let msgs = ChatService::build_steered_messages(&result);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, Role::Assistant);
        assert_eq!(msgs[0].content, "alpha\nfinal answer");
        assert_eq!(msgs[0].metadata["input_tokens"], serde_json::json!(10));
        assert_eq!(
            msgs[0].metadata["final_response"],
            serde_json::json!("final answer")
        );
        assert_eq!(
            msgs[0].metadata["tool_results"].as_array().unwrap().len(),
            1
        );
    }

    #[test]
    fn build_steered_messages_splits_at_boundary_and_slices_tools() {
        let steer = crate::agent_service::InjectedSteer {
            steer_id: "s1".into(),
            message: steer_user_msg("redirect now"),
            after_iteration: 1,
        };
        let result = make_steer_result(
            vec!["seg0\n".into(), "seg1\n".into()],
            vec![1, 1],
            "done",
            vec![steer],
        );
        let msgs = ChatService::build_steered_messages(&result);
        assert_eq!(msgs.len(), 3);

        // Segment 0 (block 0): one tool, no turn-level metadata.
        assert_eq!(msgs[0].role, Role::Assistant);
        assert_eq!(msgs[0].content, "seg0\n");
        assert_eq!(
            msgs[0].metadata["tool_results"].as_array().unwrap().len(),
            1
        );
        assert!(msgs[0].metadata.get("input_tokens").is_none());

        // Injected steer bubble.
        assert_eq!(msgs[1].role, Role::User);
        assert_eq!(msgs[1].content, "redirect now");

        // Final segment (block 1 + final): one tool + turn-level metadata.
        assert_eq!(msgs[2].role, Role::Assistant);
        assert_eq!(msgs[2].content, "seg1\ndone");
        assert_eq!(
            msgs[2].metadata["tool_results"].as_array().unwrap().len(),
            1
        );
        assert_eq!(msgs[2].metadata["input_tokens"], serde_json::json!(10));
    }

    #[test]
    fn build_steered_messages_multiple_steers_same_boundary() {
        let s1 = crate::agent_service::InjectedSteer {
            steer_id: "a".into(),
            message: steer_user_msg("one"),
            after_iteration: 1,
        };
        let s2 = crate::agent_service::InjectedSteer {
            steer_id: "b".into(),
            message: steer_user_msg("two"),
            after_iteration: 1,
        };
        let result = make_steer_result(vec!["seg0\n".into()], vec![0], "fin", vec![s1, s2]);
        let msgs = ChatService::build_steered_messages(&result);
        // [asst seg0][steer one][steer two][asst final]
        assert_eq!(msgs.len(), 4);
        assert_eq!(msgs[0].content, "seg0\n");
        assert_eq!(msgs[1].content, "one");
        assert_eq!(msgs[2].content, "two");
        assert_eq!(msgs[3].content, "fin");
    }

    #[test]
    fn build_steered_messages_steer_before_any_text() {
        let s = crate::agent_service::InjectedSteer {
            steer_id: "x".into(),
            message: steer_user_msg("early"),
            after_iteration: 0,
        };
        let result = make_steer_result(vec!["seg0\n".into()], vec![0], "fin", vec![s]);
        let msgs = ChatService::build_steered_messages(&result);
        // No leading assistant segment: [steer][asst seg0+final].
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, Role::User);
        assert_eq!(msgs[0].content, "early");
        assert_eq!(msgs[1].content, "seg0\nfin");
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
            operation_mode: None,
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
            operation_mode: None,
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
            operation_mode: None,
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
            operation_mode: None,
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
            operation_mode: None,
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
                operation_mode: None,
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
                operation_mode: None,
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
    async fn failed_turn_records_checkpoint_for_intra_turn_resend() {
        let (container, _tmp) = make_test_container().await;

        // 1. Start a turn -- persists the user message at display index 0.
        let prepared = ChatService::prepare_turn(
            &container,
            PrepareTurnRequest {
                session_id: None,
                user_input: "run the task".into(),
                provider_id: None,
                request_mode: None,
                skills: None,
                knowledge_collections: None,
                thinking: None,
                user_message_metadata: None,
                plan_mode: None,
                operation_mode: None,
                mcp_mode: None,
                mcp_servers: None,
                image_generation_options: None,
            },
        )
        .await
        .expect("prepare_turn should succeed");

        let input = prepared.as_turn_input();

        // 2. Simulate an LLM error after one tool call ran -- exactly what
        //    execute_turn_inner's LlmError branch does: persist the partial
        //    tool-call state plus the turn-boundary checkpoint.
        let assistant = Message {
            message_id: y_core::types::generate_message_id(),
            role: Role::Assistant,
            content: "calling the tool".into(),
            tool_call_id: None,
            tool_calls: vec![],
            timestamp: y_core::types::now(),
            metadata: serde_json::Value::Null,
        };
        let tool = Message {
            message_id: y_core::types::generate_message_id(),
            role: Role::Tool,
            content: "tool result".into(),
            tool_call_id: Some("call-1".into()),
            tool_calls: vec![],
            timestamp: y_core::types::now(),
            metadata: serde_json::Value::Null,
        };
        ChatService::persist_llm_error_partial_state(
            &container,
            &input,
            "LLM error: HTTP 504 Gateway Timeout",
            &[assistant, tool],
        )
        .await;
        ChatService::persist_turn_checkpoint(&container, &input).await;

        // 3. The failed turn must now have a checkpoint at its boundary.
        let checkpoint = container
            .chat_checkpoint_manager
            .list_checkpoints(&prepared.session_id)
            .await
            .expect("list checkpoints")
            .into_iter()
            .find(|cp| cp.message_count_before == 0)
            .expect("failed turn should record a boundary checkpoint");

        // 4. Resend must take the intra-turn branch: resume from the pre-turn
        //    count and keep the partial tool-call state, not wipe the turn.
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
                operation_mode: None,
            },
        )
        .await
        .expect("prepare_resend_turn should succeed");

        assert_eq!(
            resent.pre_turn_message_count,
            Some(0),
            "intra-turn retry should resume from the pre-turn message count"
        );
        assert!(
            resent.history.len() >= 3,
            "partial tool-call state must be preserved (user + assistant + tool), got {}",
            resent.history.len()
        );
        assert_eq!(resent.history[0].role, Role::User);
    }

    #[tokio::test]
    async fn intra_turn_resend_preserves_successful_tool_call_display() {
        let (container, _tmp) = make_test_container().await;

        // 1. Start a turn -- persists the user message at display index 0.
        let prepared = ChatService::prepare_turn(
            &container,
            PrepareTurnRequest {
                session_id: None,
                user_input: "run the task".into(),
                provider_id: None,
                request_mode: None,
                skills: None,
                knowledge_collections: None,
                thinking: None,
                user_message_metadata: None,
                plan_mode: None,
                operation_mode: None,
                mcp_mode: None,
                mcp_servers: None,
                image_generation_options: None,
            },
        )
        .await
        .expect("prepare_turn should succeed");
        let input = prepared.as_turn_input();

        // 2. Iteration 1 executed a tool successfully; a later LLM call then
        //    timed out. The partial state carries the completed assistant +
        //    tool messages (this is what `ctx.new_messages` accumulates).
        let assistant = Message {
            message_id: y_core::types::generate_message_id(),
            role: Role::Assistant,
            content: "calling the tool".into(),
            tool_call_id: None,
            tool_calls: vec![y_core::types::ToolCallRequest {
                id: "call-1".into(),
                name: "do_work".into(),
                arguments: serde_json::json!({ "x": 1 }),
            }],
            timestamp: y_core::types::now(),
            metadata: serde_json::Value::Null,
        };
        let tool = Message {
            message_id: y_core::types::generate_message_id(),
            role: Role::Tool,
            content: "tool result OK".into(),
            tool_call_id: Some("call-1".into()),
            tool_calls: vec![],
            timestamp: y_core::types::now(),
            metadata: serde_json::Value::Null,
        };
        ChatService::persist_llm_error_partial_state(
            &container,
            &input,
            "LLM error: HTTP 504 Gateway Timeout",
            &[assistant, tool],
        )
        .await;
        ChatService::persist_turn_checkpoint(&container, &input).await;

        // 3. Resend -- must take the intra-turn branch.
        let checkpoint = container
            .chat_checkpoint_manager
            .list_checkpoints(&prepared.session_id)
            .await
            .expect("list checkpoints")
            .into_iter()
            .find(|cp| cp.message_count_before == 0)
            .expect("failed turn should record a boundary checkpoint");
        let _resent = ChatService::prepare_resend_turn(
            &container,
            ResendTurnRequest {
                session_id: prepared.session_id.clone(),
                checkpoint_id: checkpoint.checkpoint_id,
                provider_id: None,
                request_mode: None,
                knowledge_collections: None,
                thinking: None,
                plan_mode: None,
                operation_mode: None,
            },
        )
        .await
        .expect("prepare_resend_turn should succeed");

        // 4. The display transcript must STILL show the already-executed tool
        //    call after resend prep. Only the failure marker should be removed,
        //    not the successful iteration's work.
        let display = container
            .session_manager
            .read_display_transcript(&prepared.session_id)
            .await
            .expect("read display transcript");

        let work_visible = display
            .iter()
            .any(|m| m.role == Role::Assistant && m.content.contains("calling the tool"));
        assert!(
            work_visible,
            "intra-turn retry must preserve the display of the already-executed \
             tool call; display after resend = {:?}",
            display
                .iter()
                .map(|m| (format!("{:?}", m.role), m.content.clone()))
                .collect::<Vec<_>>()
        );
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
            operation_mode: None,
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
            operation_mode: None,
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
            operation_mode: None,
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
                operation_mode: None,
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
                operation_mode: None,
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
                operation_mode: None,
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
                operation_mode: None,
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
                operation_mode: None,
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

    // --- Follow-up queue tests ---

    #[tokio::test]
    async fn test_follow_up_queue_add_and_list() {
        let (container, _tmp) = make_test_container().await;
        let sid = SessionId("test-followup-1".into());

        let msg1 = ChatService::add_follow_up(&container, &sid, "first follow-up".into()).await;
        let msg2 = ChatService::add_follow_up(&container, &sid, "second follow-up".into()).await;

        let list = ChatService::list_follow_ups(&container, &sid).await;
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].text, "first follow-up");
        assert_eq!(list[1].text, "second follow-up");
        assert_eq!(list[0].id, msg1.id);
        assert_eq!(list[1].id, msg2.id);
    }

    #[tokio::test]
    async fn test_follow_up_queue_delete() {
        let (container, _tmp) = make_test_container().await;
        let sid = SessionId("test-followup-2".into());

        let msg = ChatService::add_follow_up(&container, &sid, "to be deleted".into()).await;
        ChatService::add_follow_up(&container, &sid, "to keep".into()).await;

        let deleted = ChatService::delete_follow_up(&container, &sid, &msg.id).await;
        assert!(deleted);

        let list = ChatService::list_follow_ups(&container, &sid).await;
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].text, "to keep");
    }

    #[tokio::test]
    async fn test_follow_up_queue_delete_nonexistent() {
        let (container, _tmp) = make_test_container().await;
        let sid = SessionId("test-followup-3".into());

        ChatService::add_follow_up(&container, &sid, "exists".into()).await;
        let deleted = ChatService::delete_follow_up(&container, &sid, "nonexistent-id").await;
        assert!(!deleted);

        let list = ChatService::list_follow_ups(&container, &sid).await;
        assert_eq!(list.len(), 1);
    }

    #[tokio::test]
    async fn test_follow_up_queue_drain() {
        let (container, _tmp) = make_test_container().await;
        let sid = SessionId("test-followup-4".into());

        ChatService::add_follow_up(&container, &sid, "msg1".into()).await;
        ChatService::add_follow_up(&container, &sid, "msg2".into()).await;

        let drained = ChatService::drain_follow_ups(&container, &sid).await;
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].text, "msg1");
        assert_eq!(drained[1].text, "msg2");

        // Queue should be empty after drain.
        let list = ChatService::list_follow_ups(&container, &sid).await;
        assert!(list.is_empty());
    }

    #[tokio::test]
    async fn test_follow_up_queue_clear() {
        let (container, _tmp) = make_test_container().await;
        let sid = SessionId("test-followup-5".into());

        ChatService::add_follow_up(&container, &sid, "msg1".into()).await;
        ChatService::add_follow_up(&container, &sid, "msg2".into()).await;

        ChatService::clear_follow_ups(&container, &sid).await;

        let list = ChatService::list_follow_ups(&container, &sid).await;
        assert!(list.is_empty());
    }

    #[tokio::test]
    async fn test_follow_up_queue_empty_session() {
        let (container, _tmp) = make_test_container().await;
        let sid = SessionId("test-followup-empty".into());

        let list = ChatService::list_follow_ups(&container, &sid).await;
        assert!(list.is_empty());

        let drained = ChatService::drain_follow_ups(&container, &sid).await;
        assert!(drained.is_empty());
    }

    #[tokio::test]
    async fn test_follow_up_message_new_generates_id_and_timestamp() {
        let msg = FollowUpMessage::new("test text".into());
        assert!(!msg.id.is_empty());
        assert_eq!(msg.text, "test text");
        assert!(msg.created_at > 0);
    }

    #[tokio::test]
    async fn test_follow_up_queue_independent_from_steering() {
        let (container, _tmp) = make_test_container().await;
        let sid = SessionId("test-followup-steer".into());

        // Add both steer and follow-up.
        ChatService::add_steer(&container, &sid, "steer msg".into()).await;
        ChatService::add_follow_up(&container, &sid, "follow-up msg".into()).await;

        // Both queues should be independent.
        let steers = ChatService::list_steers(&container, &sid).await;
        let follow_ups = ChatService::list_follow_ups(&container, &sid).await;
        assert_eq!(steers.len(), 1);
        assert_eq!(follow_ups.len(), 1);
        assert_eq!(steers[0].text, "steer msg");
        assert_eq!(follow_ups[0].text, "follow-up msg");

        // Draining one should not affect the other.
        let drained_follow_ups = ChatService::drain_follow_ups(&container, &sid).await;
        assert_eq!(drained_follow_ups.len(), 1);

        let steers_after = ChatService::list_steers(&container, &sid).await;
        assert_eq!(steers_after.len(), 1);
    }
    // -----------------------------------------------------------------------
    // Retry data-loss regression tests
    // -----------------------------------------------------------------------

    /// When the LLM call fails on the very first iteration (no tool calls
    /// completed, no assistant message in `partial_messages`), the display
    /// transcript must STILL receive a failure marker. Without it,
    /// `prepare_resend_turn` cannot detect an intra-turn retry and falls
    /// through to the destructive turn-level branch, wiping the entire turn.
    #[tokio::test]
    async fn persist_llm_error_appends_failure_marker_even_with_no_partial_work() {
        let (container, _tmp) = make_test_container().await;
        let prepared = ChatService::prepare_turn(
            &container,
            PrepareTurnRequest {
                session_id: None,
                user_input: "do something".into(),
                provider_id: None,
                request_mode: None,
                skills: None,
                knowledge_collections: None,
                thinking: None,
                user_message_metadata: None,
                plan_mode: None,
                operation_mode: None,
                mcp_mode: None,
                mcp_servers: None,
                image_generation_options: None,
            },
        )
        .await
        .expect("prepare_turn should succeed");
        let input = prepared.as_turn_input();

        // Simulate a first-iteration LLM failure with zero partial messages
        // (the LLM call timed out before producing any tool call).
        ChatService::persist_llm_error_partial_state(
            &container,
            &input,
            "LLM error: rate limited by SenseNova: retry after 60s",
            &[],
        )
        .await;

        let display = container
            .session_manager
            .read_display_transcript(&prepared.session_id)
            .await
            .expect("read display transcript");

        // user message + failure marker (no success message because there
        // was no completed work).
        assert_eq!(
            display.len(),
            2,
            "display must contain the user message plus a failure marker; got {:?}",
            display
                .iter()
                .map(|m| (format!("{:?}", m.role), m.content.clone()))
                .collect::<Vec<_>>()
        );
        assert_eq!(display[0].role, Role::User);
        assert_eq!(display[1].role, Role::Assistant);
        assert_eq!(display[1].content, "");
        let stream_error = display[1]
            .metadata
            .get("stream_error")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        assert!(
            stream_error.contains("rate limited"),
            "failure marker must carry the error message; got metadata {:?}",
            display[1].metadata
        );
    }

    /// When the LLM streams partial text before failing (e.g. a 504 mid-stream),
    /// that partial content must be persisted to the display transcript so it
    /// survives a retry. The executor materializes partial streaming content as
    /// an assistant message in `partial_messages`; this test verifies
    /// `persist_llm_error_partial_state` renders it correctly.
    #[tokio::test]
    async fn persist_llm_error_preserves_partial_streaming_text() {
        let (container, _tmp) = make_test_container().await;
        let prepared = ChatService::prepare_turn(
            &container,
            PrepareTurnRequest {
                session_id: None,
                user_input: "write some code".into(),
                provider_id: None,
                request_mode: None,
                skills: None,
                knowledge_collections: None,
                thinking: None,
                user_message_metadata: None,
                plan_mode: None,
                operation_mode: None,
                mcp_mode: None,
                mcp_servers: None,
                image_generation_options: None,
            },
        )
        .await
        .expect("prepare_turn should succeed");
        let input = prepared.as_turn_input();

        // The executor's Err(e) branch materializes partial streaming content
        // as an assistant message before calling persist_llm_error_partial_state.
        let partial_assistant = Message {
            message_id: y_core::types::generate_message_id(),
            role: Role::Assistant,
            content: "Here is a partial answer that was streaming when".into(),
            tool_call_id: None,
            tool_calls: vec![],
            timestamp: y_core::types::now(),
            metadata: serde_json::Value::Null,
        };
        ChatService::persist_llm_error_partial_state(
            &container,
            &input,
            "LLM error: HTTP 504 Gateway Timeout",
            &[partial_assistant],
        )
        .await;

        let display = container
            .session_manager
            .read_display_transcript(&prepared.session_id)
            .await
            .expect("read display transcript");

        // user + success(partial text) + failure marker
        assert_eq!(display.len(), 3);
        assert_eq!(display[0].role, Role::User);
        assert_eq!(display[1].role, Role::Assistant);
        assert!(
            display[1]
                .content
                .contains("partial answer that was streaming"),
            "partial streaming text must be persisted; got {:?}",
            display[1].content
        );
        // The success message must NOT carry stream_error.
        assert!(
            display[1].metadata.get("stream_error").is_none(),
            "success message must not have stream_error"
        );
        // The failure marker must carry stream_error.
        assert_eq!(display[2].role, Role::Assistant);
        assert!(
            display[2]
                .metadata
                .get("stream_error")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .contains("504"),
            "failure marker must carry the error"
        );
    }

    /// Full regression: first-iteration failure with partial streaming text,
    /// then a retry that also fails. The partial text from the first attempt
    /// must survive the second failure -- not be wiped to just an error banner.
    #[tokio::test]
    async fn retry_after_failed_retry_preserves_partial_work() {
        let (container, _tmp) = make_test_container().await;
        let prepared = ChatService::prepare_turn(
            &container,
            PrepareTurnRequest {
                session_id: None,
                user_input: "write a function".into(),
                provider_id: None,
                request_mode: None,
                skills: None,
                knowledge_collections: None,
                thinking: None,
                user_message_metadata: None,
                plan_mode: None,
                operation_mode: None,
                mcp_mode: None,
                mcp_servers: None,
                image_generation_options: None,
            },
        )
        .await
        .expect("prepare_turn should succeed");
        let input = prepared.as_turn_input();

        // 1. First attempt: partial text streamed, then 504.
        let partial_assistant = Message {
            message_id: y_core::types::generate_message_id(),
            role: Role::Assistant,
            content: "def hello():\n    print(\"hello".into(),
            tool_call_id: None,
            tool_calls: vec![],
            timestamp: y_core::types::now(),
            metadata: serde_json::Value::Null,
        };
        ChatService::persist_llm_error_partial_state(
            &container,
            &input,
            "LLM error: HTTP 504 Gateway Timeout",
            &[partial_assistant],
        )
        .await;
        ChatService::persist_turn_checkpoint(&container, &input).await;

        // 2. Retry: intra-turn retry detected (trailing failure marker).
        let checkpoint = container
            .chat_checkpoint_manager
            .list_checkpoints(&prepared.session_id)
            .await
            .expect("list checkpoints")
            .into_iter()
            .find(|cp| cp.message_count_before == 0)
            .expect("failed turn should record a boundary checkpoint");

        let resent = ChatService::prepare_resend_turn(
            &container,
            ResendTurnRequest {
                session_id: prepared.session_id.clone(),
                checkpoint_id: checkpoint.checkpoint_id,
                provider_id: Some("other-model".into()),
                request_mode: None,
                knowledge_collections: None,
                thinking: None,
                plan_mode: None,
                operation_mode: None,
            },
        )
        .await
        .expect("prepare_resend_turn should succeed");

        // The intra-turn branch must have been taken.
        assert_eq!(
            resent.pre_turn_message_count,
            Some(0),
            "intra-turn retry should resume from the pre-turn message count"
        );

        // Display after resend prep: user + partial work (failure marker removed).
        let display_after_resend = container
            .session_manager
            .read_display_transcript(&prepared.session_id)
            .await
            .expect("read display transcript");
        assert_eq!(display_after_resend.len(), 2);
        assert!(
            display_after_resend[1].content.contains("def hello"),
            "partial work must survive resend prep"
        );

        // 3. Second attempt also fails (rate-limited, no new work).
        let retry_input = resent.as_turn_input();
        ChatService::persist_llm_error_partial_state(
            &container,
            &retry_input,
            "LLM error: rate limited by SenseNova: retry after 60s",
            &[],
        )
        .await;
        ChatService::persist_turn_checkpoint(&container, &retry_input).await;

        // 4. The partial work from the first attempt must STILL be visible.
        let final_display = container
            .session_manager
            .read_display_transcript(&prepared.session_id)
            .await
            .expect("read final display transcript");

        let has_partial_work = final_display
            .iter()
            .any(|m| m.content.contains("def hello"));
        assert!(
            has_partial_work,
            "partial work from the first attempt must survive a failed retry; \
             final display = {:?}",
            final_display
                .iter()
                .map(|m| (format!("{:?}", m.role), m.content.clone()))
                .collect::<Vec<_>>()
        );

        // And a failure marker must be present for the second failure.
        let has_failure_marker = final_display.iter().any(|m| {
            m.role == Role::Assistant
                && m.metadata
                    .get("stream_error")
                    .and_then(|v| v.as_str())
                    .is_some_and(|s| s.contains("rate limited"))
        });
        assert!(
            has_failure_marker,
            "a failure marker must be appended for the second failure so the \
             user can retry again"
        );
    }
}
