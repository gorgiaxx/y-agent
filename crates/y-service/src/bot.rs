//! Bot message handling service.
//!
//! Orchestrates the bot lifecycle: responds to inbound platform messages by
//! creating or reusing sessions, running a chat turn through [`ChatService`],
//! and replying via the originating [`BotPlatform`].

use tracing::{info, warn};

use y_bot::{BotPlatform, InboundMessage, OutboundMessage, PlatformKind};
use y_core::types::SessionId;

use crate::chat::ChatService;
use crate::container::ServiceContainer;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors from [`BotService`] operations.
#[derive(Debug, thiserror::Error)]
pub enum BotServiceError {
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
/// All methods are static — they accept `&ServiceContainer` and a
/// `&dyn BotPlatform` reference to process messages. This follows the
/// same pattern as [`ChatService`].
pub struct BotService;

impl BotService {
    /// Handle an inbound message from a bot platform.
    ///
    /// 1. Derive a deterministic session ID from `(platform, chat_id)`.
    /// 2. Prepare and execute a chat turn via [`ChatService`].
    /// 3. Send the assistant response back through the platform.
    pub async fn handle_message(
        container: &ServiceContainer,
        platform: &dyn BotPlatform,
        message: InboundMessage,
    ) -> Result<(), BotServiceError> {
        let session_id = derive_bot_session_id(message.platform, &message.chat_id);

        info!(
            platform = %message.platform,
            chat_id = %message.chat_id,
            sender_id = %message.sender_id,
            session_id = %session_id.0,
            "Bot: handling inbound message"
        );

        // Append attachment info to the prompt if any exist
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

        // Prepare the chat turn (resolve/create session, persist user message).
        let prepared = ChatService::prepare_turn(
            container,
            crate::chat::PrepareTurnRequest {
                session_id: Some(session_id.clone()),
                user_input: user_input.clone(),
                provider_id: None,
                skills: None,
                knowledge_collections: None,
                thinking: None,
                user_message_metadata: None,
            },
        )
        .await;

        // If session not found, it's a first message — create new session.
        let prepared = match prepared {
            Ok(p) => p,
            Err(crate::chat::PrepareTurnError::SessionNotFound(_)) => {
                // Create with None session_id (auto-creates), but we need
                // the deterministic ID. Let's create the session explicitly.
                ChatService::prepare_turn(
                    container,
                    crate::chat::PrepareTurnRequest {
                        session_id: None,
                        user_input,
                        provider_id: None,
                        skills: None,
                        knowledge_collections: None,
                        thinking: None,
                        user_message_metadata: None,
                    },
                )
                .await
                .map_err(|e| BotServiceError::PrepareFailed(e.to_string()))?
            }
            Err(e) => return Err(BotServiceError::PrepareFailed(e.to_string())),
        };

        let actual_session_id = prepared.session_id.clone();
        let input = prepared.as_turn_input();

        // Execute the turn.
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

        // Send the assistant response back through the platform.
        let outbound = OutboundMessage {
            chat_id: message.chat_id.clone(),
            content: result.content.clone(),
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
}

/// Derive a deterministic session ID from platform + chat ID.
///
/// Format: `bot:<platform>:<chat_id>` — this ensures each platform chat
/// gets its own session, and the ID is stable across restarts.
fn derive_bot_session_id(platform: PlatformKind, chat_id: &str) -> SessionId {
    SessionId(format!("bot:{platform}:{chat_id}"))
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
