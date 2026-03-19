//! Telegram bot platform adapter (stub).
//!
//! This module reserves the interface for Telegram bot integration.
//! The implementation will be provided in a future update.

use std::collections::HashMap;

use crate::{BotError, BotPlatform, InboundMessage, OutboundMessage, PlatformKind};

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for a Telegram bot.
#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
#[serde(default)]
pub struct TelegramBotConfig {
    /// Telegram Bot API token (from `@BotFather`).
    pub token: String,
    /// Optional webhook secret for request verification.
    pub webhook_secret: Option<String>,
}

// ---------------------------------------------------------------------------
// TelegramBot (stub)
// ---------------------------------------------------------------------------

/// Telegram bot adapter — **not yet implemented**.
///
/// All trait methods return `BotError::NotImplemented`. This struct exists
/// to define the configuration surface and allow compile-time wiring.
pub struct TelegramBot {
    pub config: TelegramBotConfig,
}

impl TelegramBot {
    /// Create a new Telegram bot adapter (stub).
    pub fn new(config: TelegramBotConfig) -> Self {
        Self { config }
    }
}

#[async_trait::async_trait]
impl BotPlatform for TelegramBot {
    fn parse_event(&self, _payload: &serde_json::Value) -> Result<InboundMessage, BotError> {
        Err(BotError::NotImplemented(
            "Telegram bot integration not yet implemented".into(),
        ))
    }

    async fn send_message(&self, _msg: &OutboundMessage) -> Result<String, BotError> {
        Err(BotError::NotImplemented(
            "Telegram bot integration not yet implemented".into(),
        ))
    }

    fn verify_signature(
        &self,
        _headers: &HashMap<String, String>,
        _body: &[u8],
    ) -> Result<(), BotError> {
        Err(BotError::NotImplemented(
            "Telegram bot integration not yet implemented".into(),
        ))
    }

    fn handle_challenge(&self, _payload: &serde_json::Value) -> Option<serde_json::Value> {
        None // Telegram doesn't use URL verification challenges.
    }

    fn platform_kind(&self) -> PlatformKind {
        PlatformKind::Telegram
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_telegram_stub_parse_event() {
        let bot = TelegramBot::new(TelegramBotConfig::default());
        let result = bot.parse_event(&serde_json::json!({}));
        assert!(matches!(result, Err(BotError::NotImplemented(_))));
    }

    #[tokio::test]
    async fn test_telegram_stub_send_message() {
        let bot = TelegramBot::new(TelegramBotConfig::default());
        let msg = OutboundMessage {
            chat_id: "test".into(),
            content: "hello".into(),
            reply_to_message_id: None,
        };
        let result = bot.send_message(&msg).await;
        assert!(matches!(result, Err(BotError::NotImplemented(_))));
    }

    #[test]
    fn test_telegram_platform_kind() {
        let bot = TelegramBot::new(TelegramBotConfig::default());
        assert_eq!(bot.platform_kind(), PlatformKind::Telegram);
    }
}
