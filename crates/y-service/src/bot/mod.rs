//! Bot message handling service.
//!
//! Orchestrates the bot lifecycle: responds to inbound platform messages by
//! resolving the bot persona, assembling a persona-aware context pipeline,
//! executing via [`AgentService`], formatting the response, and delivering
//! it back through the originating [`BotPlatform`].
//!
//! ## Module Structure
//!
//! - [`config`] -- `persona.toml` deserialization schema
//! - [`persona`] -- Markdown persona file loading
//! - [`context`] -- `BotContextProvider` (context pipeline stage)
//! - [`formatter`] -- platform-aware response formatting
//! - [`session`] -- deterministic session binding

pub mod config;
pub mod context;
pub mod formatter;
pub mod persona;
pub mod session;

use std::path::PathBuf;

use tracing::{info, warn};

use y_bot::{BotPlatform, InboundMessage, OutboundMessage};

use crate::agent_service::{AgentExecutionConfig, AgentService};
use crate::chat::ChatService;
use crate::container::ServiceContainer;

pub use self::config::BotConfig;
pub use self::persona::BotPersona;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors from [`BotService`] operations.
#[derive(Debug, thiserror::Error)]
pub enum BotServiceError {
    /// Failed to load or resolve the bot persona.
    #[error("persona error: {0}")]
    PersonaError(String),
    /// Failed to prepare the chat turn (session creation / message persist).
    #[error("prepare turn failed: {0}")]
    PrepareFailed(String),
    /// LLM execution failed.
    #[error("turn execution failed: {0}")]
    TurnFailed(String),
    /// Outbound message delivery failed.
    #[error("send reply failed: {0}")]
    SendFailed(String),
}

// ---------------------------------------------------------------------------
// BotService
// ---------------------------------------------------------------------------

/// Bot message handling service.
///
/// All methods are static -- they accept `&ServiceContainer` and a
/// `&dyn BotPlatform` reference to process messages. This follows the
/// same pattern as [`ChatService`].
pub struct BotService;

impl BotService {
    /// Handle an inbound message from a bot platform.
    ///
    /// When persona is enabled (via `persona.toml`):
    /// 1. Load persona from `config/persona/` directory.
    /// 2. Build a persona-aware context pipeline with `BotContextProvider`.
    /// 3. Execute via `AgentService::execute`.
    /// 4. Format and deliver the response.
    ///
    /// When persona is disabled or absent, falls back to the pass-through
    /// behaviour (direct `ChatService` delegation).
    pub async fn handle_message(
        container: &ServiceContainer,
        platform: &dyn BotPlatform,
        message: InboundMessage,
    ) -> Result<(), BotServiceError> {
        // Resolve persona directory.
        let persona = Self::load_persona(container);

        if !persona.is_enabled() {
            // Fallback: pass-through to ChatService (pre-Phase-1 behaviour).
            return Self::handle_message_passthrough(container, platform, message).await;
        }

        // Persona-aware path.
        Self::handle_message_with_persona(container, platform, message, &persona).await
    }

    /// Persona-aware message handling.
    async fn handle_message_with_persona(
        container: &ServiceContainer,
        platform: &dyn BotPlatform,
        message: InboundMessage,
        persona: &BotPersona,
    ) -> Result<(), BotServiceError> {
        let session_id = session::derive_bot_session_id(message.platform, &message.chat_id);

        info!(
            platform = %message.platform,
            chat_id = %message.chat_id,
            sender_id = %message.sender_id,
            session_id = %session_id.0,
            persona_name = %persona.name(),
            "Bot: handling inbound message (persona-aware)"
        );

        // Build user input with attachment info.
        let user_input = Self::build_user_input(&message);

        // Prepare the turn via ChatService (session creation + message persist).
        let prepared = Self::prepare_turn(container, &session_id, &user_input).await?;

        let actual_session_id = prepared.session_id.clone();

        // Build the persona-aware system prompt from context provider.
        let system_prompt = Self::assemble_bot_system_prompt(persona).await;

        // Build tool definitions filtered to allowed_tools.
        let tool_defs = Self::build_filtered_tool_definitions(container, persona).await;

        // Resolve tool calling mode.
        let tool_calling_mode = {
            let pool = container.provider_pool().await;
            let metadata_list = pool.list_metadata();
            metadata_list
                .first()
                .map_or(y_core::provider::ToolCallingMode::default(), |m| {
                    m.tool_calling_mode
                })
        };

        // Build AgentExecutionConfig.
        let exec_config = AgentExecutionConfig {
            agent_name: format!("bot-{}", persona.name()),
            system_prompt,
            max_iterations: persona.config.persona.tools.max_tool_iterations,
            max_tool_calls: usize::MAX,
            tool_definitions: tool_defs,
            tool_calling_mode,
            messages: prepared.history.clone(),
            provider_id: None,
            preferred_models: vec![],
            provider_tags: vec![],
            request_mode: y_core::provider::RequestMode::TextChat,
            working_directory: None,
            additional_read_dirs: vec![],
            temperature: Some(0.7),
            max_tokens: None,
            thinking: None, // Phase 1: no thinking config override.
            session_id: Some(actual_session_id.clone()),
            session_uuid: prepared.session_uuid,
            knowledge_collections: vec![],
            use_context_pipeline: false, // Bot assembles its own context.
            user_query: user_input.clone(),
            external_trace_id: None,
            trust_tier: None,
            agent_allowed_tools: persona.config.persona.tools.allowed_tools.clone(),
            prune_tool_history: false,
            response_format: None,
            image_generation_options: None,
        };

        // Execute the agent.
        let result = AgentService::execute(container, &exec_config, None, None)
            .await
            .map_err(|e| BotServiceError::TurnFailed(e.to_string()))?;

        info!(
            session_id = %actual_session_id.0,
            model = %result.model,
            tokens_in = result.input_tokens,
            tokens_out = result.output_tokens,
            "Bot: turn complete (persona-aware)"
        );

        // Format the response for platform delivery.
        // Strip accumulated <think> blocks from intermediate tool-call
        // iterations. The GUI renders these via ThinkingCard components,
        // but bot platforms receive plain text -- raw XML must not leak.
        let clean_content = crate::agent_service::strip_think_tags(&result.content);
        let max_len = persona.config.persona.messaging.max_response_length;
        let formatted = formatter::format_response(&clean_content, max_len);

        // Send the response.
        let outbound = OutboundMessage {
            chat_id: message.chat_id.clone(),
            content: formatted,
            reply_to_message_id: Some(message.message_id.clone()),
        };

        if let Err(e) = platform.send_message(&outbound).await {
            warn!(
                error = %e,
                chat_id = %message.chat_id,
                "Bot: failed to send reply"
            );
            return Err(BotServiceError::SendFailed(e.to_string()));
        }

        Ok(())
    }

    /// Pass-through message handling (pre-persona behaviour).
    ///
    /// Direct delegation to `ChatService` without persona context.
    async fn handle_message_passthrough(
        container: &ServiceContainer,
        platform: &dyn BotPlatform,
        message: InboundMessage,
    ) -> Result<(), BotServiceError> {
        let session_id = session::derive_bot_session_id(message.platform, &message.chat_id);

        info!(
            platform = %message.platform,
            chat_id = %message.chat_id,
            sender_id = %message.sender_id,
            session_id = %session_id.0,
            "Bot: handling inbound message (pass-through)"
        );

        let user_input = Self::build_user_input(&message);

        let prepared = Self::prepare_turn(container, &session_id, &user_input).await?;
        let actual_session_id = prepared.session_id.clone();
        let input = prepared.as_turn_input();

        let result = ChatService::execute_turn(container, &input)
            .await
            .map_err(|e| BotServiceError::TurnFailed(e.to_string()))?;

        info!(
            session_id = %actual_session_id.0,
            model = %result.model,
            tokens_in = result.input_tokens,
            tokens_out = result.output_tokens,
            "Bot: turn complete"
        );

        // Strip accumulated <think> blocks -- same rationale as the
        // persona-aware path (see handle_message_with_persona).
        let clean_content = crate::agent_service::strip_think_tags(&result.content);
        let outbound = OutboundMessage {
            chat_id: message.chat_id.clone(),
            content: clean_content,
            reply_to_message_id: Some(message.message_id.clone()),
        };

        if let Err(e) = platform.send_message(&outbound).await {
            warn!(
                error = %e,
                chat_id = %message.chat_id,
                "Bot: failed to send reply"
            );
            return Err(BotServiceError::SendFailed(e.to_string()));
        }

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    /// Load the bot persona from the container's persona directory.
    fn load_persona(container: &ServiceContainer) -> BotPersona {
        match container.persona_dir {
            Some(ref dir) if dir.is_dir() => BotPersona::load(dir),
            _ => BotPersona::default_embedded(),
        }
    }

    /// Build user input string with attachment info appended.
    fn build_user_input(message: &InboundMessage) -> String {
        let mut user_input = message.content.clone();
        if !message.attachments.is_empty() {
            use std::fmt::Write;
            let _ = write!(
                &mut user_input,
                "\n\n[System: The user sent the following attachments:]\n"
            );
            for att in &message.attachments {
                if let Some(ref p) = att.path {
                    let _ = writeln!(
                        &mut user_input,
                        "- {} ({}) saved at: {}",
                        att.file_name, att.content_type, p
                    );
                } else {
                    let _ = writeln!(
                        &mut user_input,
                        "- {} ({}): {} bytes",
                        att.file_name,
                        att.content_type,
                        att.data.len()
                    );
                }
            }
        }
        user_input
    }

    /// Prepare a turn via `ChatService` (resolve/create session + persist user message).
    async fn prepare_turn(
        container: &ServiceContainer,
        session_id: &y_core::types::SessionId,
        user_input: &str,
    ) -> Result<crate::chat::PreparedTurn, BotServiceError> {
        let prepared = ChatService::prepare_turn(
            container,
            crate::chat::PrepareTurnRequest {
                session_id: Some(session_id.clone()),
                user_input: user_input.to_string(),
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
        .await;

        match prepared {
            Ok(p) => Ok(p),
            Err(crate::chat::PrepareTurnError::SessionNotFound(_)) => {
                // First message -- create session with auto-generated ID.
                ChatService::prepare_turn(
                    container,
                    crate::chat::PrepareTurnRequest {
                        session_id: None,
                        user_input: user_input.to_string(),
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
                .map_err(|e| BotServiceError::PrepareFailed(e.to_string()))
            }
            Err(e) => Err(BotServiceError::PrepareFailed(e.to_string())),
        }
    }

    /// Assemble the bot system prompt from persona sections.
    ///
    /// Creates a temporary `BotContextProvider` and runs it to produce the
    /// concatenated persona prompt. In a full context pipeline, this would
    /// be registered as a provider -- here we call it directly since the bot
    /// uses `use_context_pipeline: false`.
    async fn assemble_bot_system_prompt(persona: &BotPersona) -> String {
        use y_context::pipeline::AssembledContext;
        use y_context::pipeline::ContextProvider;

        let provider = context::BotContextProvider::new(persona.clone());
        let mut ctx = AssembledContext::default();

        if let Err(e) = provider.provide(&mut ctx).await {
            warn!(error = %e, "BotContextProvider failed; using empty system prompt");
            return String::new();
        }

        // Concatenate all items in insertion order (which is priority order
        // since BotContextProvider adds them in ascending priority).
        ctx.items
            .iter()
            .map(|item| item.content.as_str())
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    /// Build tool definitions filtered to the persona's `allowed_tools`.
    ///
    /// If `allowed_tools` is empty, no tools are provided (the bot operates
    /// in text-only mode).
    async fn build_filtered_tool_definitions(
        container: &ServiceContainer,
        persona: &BotPersona,
    ) -> Vec<serde_json::Value> {
        let allowed = &persona.config.persona.tools.allowed_tools;
        if allowed.is_empty() {
            return Vec::new();
        }

        let mut defs = Vec::with_capacity(allowed.len());
        for name in allowed {
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
}

/// Resolve the persona directory path.
///
/// Used by [`ServiceContainer::from_config`] to compute the default
/// persona directory from the config directory.
pub fn default_persona_dir(config_dir: &std::path::Path) -> PathBuf {
    config_dir.join("persona")
}

#[cfg(test)]
mod tests {
    use super::session::derive_bot_session_id;
    use y_bot::PlatformKind;

    #[test]
    fn test_derive_session_id_feishu() {
        let sid = derive_bot_session_id(PlatformKind::Feishu, "oc_group_123");
        assert_eq!(sid.0, "bot:feishu:oc_group_123");
    }

    #[test]
    fn test_derive_session_id_telegram() {
        let sid = derive_bot_session_id(PlatformKind::Telegram, "-100123456");
        assert_eq!(sid.0, "bot:telegram:-100123456");
    }

    #[test]
    fn test_derive_session_id_deterministic() {
        let a = derive_bot_session_id(PlatformKind::Feishu, "oc_abc");
        let b = derive_bot_session_id(PlatformKind::Feishu, "oc_abc");
        assert_eq!(a.0, b.0);
    }

    #[test]
    fn test_strip_think_tags_from_accumulated_content() {
        // Simulates the exact bug: accumulated_content from multiple
        // tool-call iterations prepended to the final response.
        let input = concat!(
            "<think>first iteration thinking</think>",
            "<think>second iteration thinking</think>",
            "<think>third iteration thinking\n</think>\n\n",
            "Final answer text"
        );
        let result = crate::agent_service::strip_think_tags(input);
        assert_eq!(result, "Final answer text");
    }

    #[test]
    fn test_strip_think_tags_no_tags() {
        let input = "Just plain text";
        let result = crate::agent_service::strip_think_tags(input);
        assert_eq!(result, "Just plain text");
    }

    #[test]
    fn test_strip_think_tags_chinese_content() {
        let input = concat!(
            "<think>chinese thinking content</think>",
            "<think>more thinking</think>",
            "Final answer"
        );
        let result = crate::agent_service::strip_think_tags(input);
        assert_eq!(result, "Final answer");
    }
}
