//! Discord bot platform adapter.
//!
//! Implements [`BotPlatform`] for Discord using the Interactions Endpoint
//! (webhook-based) for inbound events and REST API v10 for outbound messages.
//!
//! **Signature verification**: Discord requires Ed25519 verification of all
//! incoming interaction payloads using the application's public key.
//!
//! **Reference**: Discord Developer Documentation and patterns from
//! `OpenClaw`'s `extensions/discord/src/`.

use std::collections::HashMap;

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use tracing::{debug, warn};

use crate::{BotError, BotPlatform, ChatType, InboundMessage, OutboundMessage, PlatformKind};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const DISCORD_API_BASE: &str = "https://discord.com/api/v10";

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for a Discord bot application.
#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
#[serde(default)]
pub struct DiscordBotConfig {
    /// Discord bot token (from the Developer Portal).
    pub token: String,
    /// Application ID.
    pub application_id: String,
    /// Ed25519 public key (hex-encoded) for Interactions Endpoint verification.
    pub public_key: String,
}

// ---------------------------------------------------------------------------
// DiscordBot
// ---------------------------------------------------------------------------

/// Discord bot adapter implementing [`BotPlatform`].
pub struct DiscordBot {
    pub config: DiscordBotConfig,
    http: reqwest::Client,
    /// Cached parsed verifying key (None if `public_key` is empty/invalid).
    verifying_key: Option<VerifyingKey>,
}

impl DiscordBot {
    /// Create a new Discord bot adapter.
    pub fn new(config: DiscordBotConfig) -> Self {
        let verifying_key = parse_verifying_key(&config.public_key);
        Self {
            config,
            http: reqwest::Client::new(),
            verifying_key,
        }
    }
}

/// Parse a hex-encoded Ed25519 public key into a `VerifyingKey`.
fn parse_verifying_key(hex_key: &str) -> Option<VerifyingKey> {
    let trimmed = hex_key.trim();
    if trimmed.is_empty() {
        return None;
    }
    let bytes = match hex::decode(trimmed) {
        Ok(b) => b,
        Err(e) => {
            warn!("Discord public_key hex decode failed: {e}");
            return None;
        }
    };
    let Ok(key_bytes): Result<[u8; 32], _> = bytes.try_into() else {
        warn!("Discord public_key must be exactly 32 bytes");
        return None;
    };
    match VerifyingKey::from_bytes(&key_bytes) {
        Ok(k) => Some(k),
        Err(e) => {
            warn!("Discord public_key is not a valid Ed25519 key: {e}");
            None
        }
    }
}

// ---------------------------------------------------------------------------
// BotPlatform implementation
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl BotPlatform for DiscordBot {
    fn parse_event(&self, payload: &serde_json::Value) -> Result<InboundMessage, BotError> {
        // Discord Interactions endpoint sends different payload types.
        // We also support gateway-forwarded MESSAGE_CREATE events.

        // Check for gateway event format: { "t": "MESSAGE_CREATE", "d": { ... } }
        if let Some(event_type) = payload.get("t").and_then(|v| v.as_str()) {
            return match event_type {
                "MESSAGE_CREATE" => parse_gateway_message(payload),
                _ => Err(BotError::UnsupportedEvent(event_type.to_string())),
            };
        }

        // Interaction payload format: { "type": 1|2|3|..., "data": { ... } }
        let interaction_type = payload
            .get("type")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);

        match interaction_type {
            // Type 1 = PING (handled by handle_challenge, should not reach here)
            1 => Err(BotError::UnsupportedEvent("PING".to_string())),
            // Type 2 = APPLICATION_COMMAND (slash commands) -- future extension
            2 => Err(BotError::UnsupportedEvent(
                "APPLICATION_COMMAND".to_string(),
            )),
            // Type 3 = MESSAGE_COMPONENT -- future extension
            3 => Err(BotError::UnsupportedEvent("MESSAGE_COMPONENT".to_string())),
            _ => Err(BotError::UnsupportedEvent(format!(
                "interaction type {interaction_type}"
            ))),
        }
    }

    async fn send_message(&self, msg: &OutboundMessage) -> Result<String, BotError> {
        let url = format!("{DISCORD_API_BASE}/channels/{}/messages", msg.chat_id);

        let mut body = serde_json::json!({
            "content": msg.content,
        });

        // If replying to a message, set the message_reference field.
        if let Some(ref reply_to) = msg.reply_to_message_id {
            body["message_reference"] = serde_json::json!({
                "message_id": reply_to,
            });
        }

        let resp = self
            .http
            .post(&url)
            .header("Authorization", format!("Bot {}", self.config.token))
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;

        if !status.is_success() {
            let api_msg = json
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown error");
            let code = json.get("code").and_then(serde_json::Value::as_u64);
            warn!(
                status = %status,
                code = ?code,
                msg = api_msg,
                "Discord send_message failed"
            );
            return Err(BotError::ApiError(format!(
                "Discord send failed ({status}): {api_msg}"
            )));
        }

        let sent_message_id = json
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        debug!(message_id = %sent_message_id, "Discord message sent");
        Ok(sent_message_id)
    }

    fn verify_signature(
        &self,
        headers: &HashMap<String, String>,
        body: &[u8],
    ) -> Result<(), BotError> {
        let Some(ref vk) = self.verifying_key else {
            // No public key configured -- skip verification.
            return Ok(());
        };

        let signature_hex = headers
            .get("x-signature-ed25519")
            .ok_or(BotError::SignatureInvalid)?;
        let timestamp = headers
            .get("x-signature-timestamp")
            .ok_or(BotError::SignatureInvalid)?;

        // Decode hex signature.
        let sig_bytes = hex::decode(signature_hex).map_err(|_| BotError::SignatureInvalid)?;
        let signature =
            Signature::from_slice(&sig_bytes).map_err(|_| BotError::SignatureInvalid)?;

        // The signed message is: timestamp + body.
        let mut message = Vec::with_capacity(timestamp.len() + body.len());
        message.extend_from_slice(timestamp.as_bytes());
        message.extend_from_slice(body);

        vk.verify(&message, &signature)
            .map_err(|_| BotError::SignatureInvalid)
    }

    fn handle_challenge(&self, payload: &serde_json::Value) -> Option<serde_json::Value> {
        // Discord Interaction type 1 = PING, must respond with type 1 = PONG.
        let interaction_type = payload.get("type").and_then(serde_json::Value::as_u64)?;

        if interaction_type == 1 {
            return Some(serde_json::json!({ "type": 1 }));
        }

        None
    }

    fn platform_kind(&self) -> PlatformKind {
        PlatformKind::Discord
    }
}

// ---------------------------------------------------------------------------
// Gateway event parsing
// ---------------------------------------------------------------------------

/// Parse a Discord gateway `MESSAGE_CREATE` event into an [`InboundMessage`].
///
/// Gateway payload structure:
/// ```json
/// {
///   "t": "MESSAGE_CREATE",
///   "d": {
///     "id": "message_id",
///     "channel_id": "...",
///     "author": { "id": "...", "username": "..." },
///     "content": "...",
///     "timestamp": "2024-01-01T00:00:00.000000+00:00",
///     "guild_id": "..." (absent for DMs),
///     "message_reference": { "message_id": "..." }
///   }
/// }
/// ```
fn parse_gateway_message(payload: &serde_json::Value) -> Result<InboundMessage, BotError> {
    let data = payload
        .get("d")
        .ok_or_else(|| BotError::ParseError("missing 'd' field in gateway event".into()))?;

    let message_id = data
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let channel_id = data
        .get("channel_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let author = data
        .get("author")
        .ok_or_else(|| BotError::ParseError("missing 'author' field".into()))?;

    let sender_id = author
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let sender_name = author
        .get("username")
        .and_then(|v| v.as_str())
        .map(String::from);

    let content = data
        .get("content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // Determine chat type: if guild_id is present, it is a group/guild message.
    let chat_type = if data.get("guild_id").and_then(|v| v.as_str()).is_some() {
        ChatType::Group
    } else {
        ChatType::P2p
    };

    // Parse reply reference.
    let reply_to_message_id = data
        .get("message_reference")
        .and_then(|r| r.get("message_id"))
        .and_then(|v| v.as_str())
        .map(String::from);

    // Parse ISO 8601 timestamp.
    let timestamp = data
        .get("timestamp")
        .and_then(|v| v.as_str())
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map_or_else(chrono::Utc::now, |dt| dt.with_timezone(&chrono::Utc));

    Ok(InboundMessage {
        platform: PlatformKind::Discord,
        chat_id: channel_id,
        message_id,
        sender_id,
        sender_name,
        content,
        chat_type,
        reply_to_message_id,
        timestamp,
        raw: payload.clone(),
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn default_bot() -> DiscordBot {
        DiscordBot::new(DiscordBotConfig::default())
    }

    #[test]
    fn test_discord_config_default() {
        let cfg = DiscordBotConfig::default();
        assert!(cfg.token.is_empty());
        assert!(cfg.application_id.is_empty());
        assert!(cfg.public_key.is_empty());
    }

    #[test]
    fn test_discord_platform_kind() {
        let bot = default_bot();
        assert_eq!(bot.platform_kind(), PlatformKind::Discord);
    }

    #[test]
    fn test_discord_parse_event_message_create() {
        let bot = default_bot();
        let payload = serde_json::json!({
            "t": "MESSAGE_CREATE",
            "d": {
                "id": "msg_001",
                "channel_id": "ch_123",
                "author": {
                    "id": "user_456",
                    "username": "TestUser",
                },
                "content": "Hello from Discord!",
                "timestamp": "2024-06-15T12:00:00.000000+00:00",
                "guild_id": "guild_789",
            }
        });

        let msg = bot.parse_event(&payload).unwrap();
        assert_eq!(msg.platform, PlatformKind::Discord);
        assert_eq!(msg.chat_id, "ch_123");
        assert_eq!(msg.message_id, "msg_001");
        assert_eq!(msg.sender_id, "user_456");
        assert_eq!(msg.sender_name.as_deref(), Some("TestUser"));
        assert_eq!(msg.content, "Hello from Discord!");
        assert_eq!(msg.chat_type, ChatType::Group);
        assert!(msg.reply_to_message_id.is_none());
    }

    #[test]
    fn test_discord_parse_event_dm() {
        let bot = default_bot();
        let payload = serde_json::json!({
            "t": "MESSAGE_CREATE",
            "d": {
                "id": "msg_002",
                "channel_id": "dm_ch_999",
                "author": {
                    "id": "user_111",
                    "username": "DMUser",
                },
                "content": "DM content",
                "timestamp": "2024-06-15T12:00:00.000000+00:00",
            }
        });

        let msg = bot.parse_event(&payload).unwrap();
        assert_eq!(msg.chat_type, ChatType::P2p);
        assert_eq!(msg.chat_id, "dm_ch_999");
    }

    #[test]
    fn test_discord_parse_event_with_reply() {
        let bot = default_bot();
        let payload = serde_json::json!({
            "t": "MESSAGE_CREATE",
            "d": {
                "id": "msg_003",
                "channel_id": "ch_123",
                "author": { "id": "user_456", "username": "Replier" },
                "content": "This is a reply",
                "timestamp": "2024-06-15T12:00:00.000000+00:00",
                "guild_id": "guild_789",
                "message_reference": {
                    "message_id": "msg_original",
                }
            }
        });

        let msg = bot.parse_event(&payload).unwrap();
        assert_eq!(msg.reply_to_message_id.as_deref(), Some("msg_original"));
    }

    #[test]
    fn test_discord_parse_event_unsupported() {
        let bot = default_bot();
        let payload = serde_json::json!({
            "t": "GUILD_MEMBER_ADD",
            "d": {}
        });

        let result = bot.parse_event(&payload);
        assert!(matches!(result, Err(BotError::UnsupportedEvent(_))));
    }

    #[test]
    fn test_discord_parse_event_interaction_unsupported() {
        let bot = default_bot();
        // APPLICATION_COMMAND interaction (type 2)
        let payload = serde_json::json!({ "type": 2, "data": {} });
        let result = bot.parse_event(&payload);
        assert!(
            matches!(result, Err(BotError::UnsupportedEvent(ref s)) if s == "APPLICATION_COMMAND")
        );
    }

    #[test]
    fn test_discord_challenge_ping() {
        let bot = default_bot();
        let payload = serde_json::json!({ "type": 1 });
        let resp = bot.handle_challenge(&payload);
        assert!(resp.is_some());
        let resp = resp.unwrap();
        assert_eq!(resp.get("type").unwrap().as_u64().unwrap(), 1);
    }

    #[test]
    fn test_discord_challenge_normal_event() {
        let bot = default_bot();
        let payload = serde_json::json!({
            "t": "MESSAGE_CREATE",
            "d": {}
        });
        assert!(bot.handle_challenge(&payload).is_none());
    }

    #[test]
    fn test_discord_challenge_non_ping_interaction() {
        let bot = default_bot();
        let payload = serde_json::json!({ "type": 2, "data": {} });
        assert!(bot.handle_challenge(&payload).is_none());
    }

    #[test]
    fn test_discord_signature_verify_valid() {
        use ed25519_dalek::{Signer, SigningKey};

        // Generate a test keypair.
        let signing_key = SigningKey::from_bytes(&[42u8; 32]);
        let verifying_key = signing_key.verifying_key();
        let public_key_hex = hex::encode(verifying_key.as_bytes());

        let config = DiscordBotConfig {
            public_key: public_key_hex,
            ..Default::default()
        };
        let bot = DiscordBot::new(config);

        let timestamp = "1700000000";
        let body = r#"{"type":1}"#;

        // Sign: timestamp + body
        let mut message = Vec::new();
        message.extend_from_slice(timestamp.as_bytes());
        message.extend_from_slice(body.as_bytes());
        let signature = signing_key.sign(&message);
        let sig_hex = hex::encode(signature.to_bytes());

        let mut headers = HashMap::new();
        headers.insert("x-signature-ed25519".to_string(), sig_hex);
        headers.insert("x-signature-timestamp".to_string(), timestamp.to_string());

        assert!(bot.verify_signature(&headers, body.as_bytes()).is_ok());
    }

    #[test]
    fn test_discord_signature_verify_invalid() {
        use ed25519_dalek::SigningKey;

        let signing_key = SigningKey::from_bytes(&[42u8; 32]);
        let verifying_key = signing_key.verifying_key();
        let public_key_hex = hex::encode(verifying_key.as_bytes());

        let config = DiscordBotConfig {
            public_key: public_key_hex,
            ..Default::default()
        };
        let bot = DiscordBot::new(config);

        let mut headers = HashMap::new();
        headers.insert(
            "x-signature-ed25519".to_string(),
            hex::encode([0u8; 64]), // invalid signature
        );
        headers.insert(
            "x-signature-timestamp".to_string(),
            "1700000000".to_string(),
        );

        let result = bot.verify_signature(&headers, b"test body");
        assert!(matches!(result, Err(BotError::SignatureInvalid)));
    }

    #[test]
    fn test_discord_signature_skip_when_no_public_key() {
        let bot = default_bot();
        let headers = HashMap::new();
        // No public key configured -- verification is skipped.
        assert!(bot.verify_signature(&headers, b"anything").is_ok());
    }

    #[test]
    fn test_discord_signature_missing_headers() {
        use ed25519_dalek::SigningKey;

        let signing_key = SigningKey::from_bytes(&[42u8; 32]);
        let verifying_key = signing_key.verifying_key();
        let public_key_hex = hex::encode(verifying_key.as_bytes());

        let config = DiscordBotConfig {
            public_key: public_key_hex,
            ..Default::default()
        };
        let bot = DiscordBot::new(config);

        // Missing headers should fail.
        let headers = HashMap::new();
        assert!(matches!(
            bot.verify_signature(&headers, b"test"),
            Err(BotError::SignatureInvalid)
        ));
    }

    #[tokio::test]
    async fn test_discord_stub_send_message() {
        // Verifies send_message constructs a proper request (will fail at HTTP level
        // without a real server, so we just check that no panic occurs).
        let bot = default_bot();
        let msg = OutboundMessage {
            chat_id: "test_channel".into(),
            content: "hello".into(),
            reply_to_message_id: None,
        };
        // Since there is no Discord server, this will fail with an HTTP error.
        let result = bot.send_message(&msg).await;
        assert!(result.is_err());
    }
}
