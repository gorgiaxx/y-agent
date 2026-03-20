//! y-bot: Bot platform adapters for y-agent.
//!
//! This crate provides a unified [`BotPlatform`] trait and concrete adapters
//! for integrating y-agent with messaging platforms:
//!
//! - **Feishu** (飞书 / Lark) — fully implemented webhook-based adapter
//! - **Telegram** — interface reserved, implementation pending
//!
//! ## Architecture
//!
//! ```text
//! Platform webhook  →  y-web route  →  BotPlatform::parse_event()
//!                                          ↓
//!                                      BotService (y-service)
//!                                          ↓
//!                                      BotPlatform::send_message()
//! ```

pub mod error;
pub mod feishu;
pub mod telegram;

use std::collections::HashMap;

pub use error::BotError;

// ---------------------------------------------------------------------------
// Core types
// ---------------------------------------------------------------------------

/// Identifies which bot platform a message originates from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlatformKind {
    Feishu,
    Telegram,
}

impl std::fmt::Display for PlatformKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlatformKind::Feishu => write!(f, "feishu"),
            PlatformKind::Telegram => write!(f, "telegram"),
        }
    }
}

/// Chat type: direct message or group conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatType {
    /// Direct (1-on-1) message.
    P2p,
    /// Group conversation.
    Group,
}

/// An inbound message received from a bot platform.
#[derive(Debug, Clone, serde::Serialize)]
pub struct InboundMessage {
    /// Which platform this message came from.
    pub platform: PlatformKind,
    /// Platform-level chat/conversation identifier.
    pub chat_id: String,
    /// Platform-level message identifier.
    pub message_id: String,
    /// Platform-level sender identifier.
    pub sender_id: String,
    /// Display name of the sender (best-effort).
    pub sender_name: Option<String>,
    /// Extracted plain-text content.
    pub content: String,
    /// Chat type (P2P or Group).
    pub chat_type: ChatType,
    /// If this is a reply, the parent message ID.
    pub reply_to_message_id: Option<String>,
    /// Message timestamp.
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// Raw platform event payload for advanced use.
    pub raw: serde_json::Value,
}

/// An outbound message to send via a bot platform.
#[derive(Debug, Clone)]
pub struct OutboundMessage {
    /// Target chat/conversation identifier.
    pub chat_id: String,
    /// Markdown-formatted response content.
    pub content: String,
    /// Optional: reply to a specific message.
    pub reply_to_message_id: Option<String>,
}

// ---------------------------------------------------------------------------
// BotPlatform trait
// ---------------------------------------------------------------------------

/// Unified trait for bot platform adapters.
///
/// Each platform (Feishu, Telegram, …) implements this trait to handle
/// inbound webhook events and outbound message delivery.
#[async_trait::async_trait]
pub trait BotPlatform: Send + Sync {
    /// Parse a raw webhook event payload into an [`InboundMessage`].
    ///
    /// Returns `Err(BotError::UnsupportedEvent)` for events that do not
    /// represent user messages (e.g. bot-added events, reactions).
    fn parse_event(&self, payload: &serde_json::Value) -> Result<InboundMessage, BotError>;

    /// Send a reply message to the platform.
    ///
    /// Returns the platform-assigned message ID of the sent message.
    async fn send_message(&self, msg: &OutboundMessage) -> Result<String, BotError>;

    /// Verify the webhook request signature.
    ///
    /// `headers` contains the HTTP headers (lowercased keys).
    /// `body` is the raw request body bytes.
    ///
    /// # Errors
    fn verify_signature(
        &self,
        headers: &HashMap<String, String>,
        body: &[u8],
    ) -> Result<(), BotError>;

    /// Handle platform-specific URL verification challenges.
    ///
    /// Returns `Some(response_json)` if the payload is a challenge that
    /// should be echoed back. Returns `None` for normal event payloads.
    fn handle_challenge(&self, payload: &serde_json::Value) -> Option<serde_json::Value>;

    /// The platform kind this adapter handles.
    fn platform_kind(&self) -> PlatformKind;
}
