//! Feishu (飞书 / Lark) bot platform adapter.
//!
//! Implements [`BotPlatform`] for the Feishu messaging platform using
//! webhook-based event delivery and REST API for outbound messages.
//!
//! **Reference**: Patterns adapted from OpenClaw's `extensions/feishu/src/`.

use std::collections::HashMap;
use std::sync::RwLock;

use sha2::{Digest, Sha256};
use tracing::{debug, warn};

use crate::{BotError, BotPlatform, ChatType, InboundMessage, OutboundMessage, PlatformKind};

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for a Feishu bot application.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(default)]
pub struct FeishuBotConfig {
    /// Feishu App ID.
    pub app_id: String,
    /// Feishu App Secret.
    pub app_secret: String,
    /// Encrypt key for webhook signature verification (optional).
    pub encrypt_key: Option<String>,
    /// Verification token for event subscription (optional).
    pub verification_token: Option<String>,
    /// API domain: `"feishu"` (default), `"lark"` (international), or custom URL.
    pub domain: String,
}

impl Default for FeishuBotConfig {
    fn default() -> Self {
        Self {
            app_id: String::new(),
            app_secret: String::new(),
            encrypt_key: None,
            verification_token: None,
            domain: "feishu".to_string(),
        }
    }
}

impl FeishuBotConfig {
    /// Resolve the base API URL from the domain setting.
    pub fn api_base_url(&self) -> String {
        match self.domain.as_str() {
            "feishu" => "https://open.feishu.cn/open-apis".to_string(),
            "lark" => "https://open.larksuite.com/open-apis".to_string(),
            custom if custom.starts_with("http") => {
                format!("{}/open-apis", custom.trim_end_matches('/'))
            }
            _ => "https://open.feishu.cn/open-apis".to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// Cached tenant access token
// ---------------------------------------------------------------------------

struct CachedToken {
    token: String,
    expires_at: std::time::Instant,
}

// ---------------------------------------------------------------------------
// FeishuBot
// ---------------------------------------------------------------------------

/// Feishu bot adapter implementing [`BotPlatform`].
pub struct FeishuBot {
    pub config: FeishuBotConfig,
    http: reqwest::Client,
    cached_token: RwLock<Option<CachedToken>>,
}

impl FeishuBot {
    /// Create a new Feishu bot adapter.
    pub fn new(config: FeishuBotConfig) -> Self {
        Self {
            config,
            http: reqwest::Client::new(),
            cached_token: RwLock::new(None),
        }
    }

    /// Get a valid tenant access token, refreshing if expired.
    async fn tenant_access_token(&self) -> Result<String, BotError> {
        // Check cache first.
        {
            let guard = self.cached_token.read().map_err(|e| {
                BotError::ApiError(format!("token lock poisoned: {e}"))
            })?;
            if let Some(ref cached) = *guard {
                if cached.expires_at > std::time::Instant::now() {
                    return Ok(cached.token.clone());
                }
            }
        }

        // Refresh token.
        let url = format!(
            "{}/auth/v3/tenant_access_token/internal",
            self.config.api_base_url()
        );
        let body = serde_json::json!({
            "app_id": self.config.app_id,
            "app_secret": self.config.app_secret,
        });

        let resp = self
            .http
            .post(&url)
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;

        if !status.is_success() || json.get("code").and_then(|c| c.as_i64()) != Some(0) {
            let msg = json
                .get("msg")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown error");
            return Err(BotError::ApiError(format!(
                "failed to get tenant access token: {msg}"
            )));
        }

        let token = json
            .get("tenant_access_token")
            .and_then(|t| t.as_str())
            .ok_or_else(|| BotError::ApiError("missing tenant_access_token in response".into()))?
            .to_string();

        let expire_secs = json
            .get("expire")
            .and_then(|e| e.as_u64())
            .unwrap_or(7200);
        // Refresh 5 minutes early to avoid edge cases.
        let ttl = std::time::Duration::from_secs(expire_secs.saturating_sub(300));

        let mut guard = self.cached_token.write().map_err(|e| {
            BotError::ApiError(format!("token lock poisoned: {e}"))
        })?;
        *guard = Some(CachedToken {
            token: token.clone(),
            expires_at: std::time::Instant::now() + ttl,
        });

        debug!("Feishu tenant access token refreshed (ttl={expire_secs}s)");
        Ok(token)
    }

    /// Build a Feishu interactive card (schema 2.0) with markdown content.
    fn build_markdown_card(text: &str) -> serde_json::Value {
        serde_json::json!({
            "schema": "2.0",
            "config": { "wide_screen_mode": true },
            "body": {
                "elements": [{
                    "tag": "markdown",
                    "content": text,
                }]
            }
        })
    }
}

// ---------------------------------------------------------------------------
// BotPlatform implementation
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl BotPlatform for FeishuBot {
    fn parse_event(&self, payload: &serde_json::Value) -> Result<InboundMessage, BotError> {
        // Feishu event schema v2: { "schema": "2.0", "header": { "event_type": "..." }, "event": { ... } }
        let header = payload
            .get("header")
            .ok_or_else(|| BotError::ParseError("missing 'header' field".into()))?;

        let event_type = header
            .get("event_type")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if event_type != "im.message.receive_v1" {
            return Err(BotError::UnsupportedEvent(event_type.to_string()));
        }

        let event = payload
            .get("event")
            .ok_or_else(|| BotError::ParseError("missing 'event' field".into()))?;

        let message = event
            .get("message")
            .ok_or_else(|| BotError::ParseError("missing 'event.message'".into()))?;
        let sender = event
            .get("sender")
            .ok_or_else(|| BotError::ParseError("missing 'event.sender'".into()))?;

        let message_id = message
            .get("message_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let chat_id = message
            .get("chat_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let chat_type_str = message
            .get("chat_type")
            .and_then(|v| v.as_str())
            .unwrap_or("p2p");
        let message_type = message
            .get("message_type")
            .and_then(|v| v.as_str())
            .unwrap_or("text");
        let raw_content = message
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let root_id = message
            .get("root_id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from);

        // Parse create_time (millisecond timestamp string).
        let create_time_ms = message
            .get("create_time")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(0);
        let timestamp = chrono::DateTime::from_timestamp_millis(create_time_ms)
            .unwrap_or_else(chrono::Utc::now);

        // Extract sender ID.
        let sender_id = sender
            .get("sender_id")
            .and_then(|s| {
                s.get("open_id")
                    .or_else(|| s.get("user_id"))
                    .and_then(|v| v.as_str())
            })
            .unwrap_or("")
            .to_string();

        // Extract text content based on message type.
        let content = parse_feishu_content(raw_content, message_type);

        let chat_type = match chat_type_str {
            "group" => ChatType::Group,
            _ => ChatType::P2p,
        };

        Ok(InboundMessage {
            platform: PlatformKind::Feishu,
            chat_id,
            message_id,
            sender_id,
            sender_name: None, // Resolved later by BotService if needed.
            content,
            chat_type,
            reply_to_message_id: root_id,
            timestamp,
            raw: payload.clone(),
        })
    }

    async fn send_message(&self, msg: &OutboundMessage) -> Result<String, BotError> {
        let token = self.tenant_access_token().await?;
        let card = Self::build_markdown_card(&msg.content);

        // If replying to a message, use the reply API; otherwise create a new message.
        let (url, body) = if let Some(ref reply_to) = msg.reply_to_message_id {
            let url = format!(
                "{}/im/v1/messages/{}/reply",
                self.config.api_base_url(),
                reply_to
            );
            let body = serde_json::json!({
                "content": serde_json::to_string(&card).unwrap_or_default(),
                "msg_type": "interactive",
            });
            (url, body)
        } else {
            let url = format!(
                "{}{}",
                self.config.api_base_url(),
                "/im/v1/messages?receive_id_type=chat_id"
            );
            let body = serde_json::json!({
                "receive_id": msg.chat_id,
                "content": serde_json::to_string(&card).unwrap_or_default(),
                "msg_type": "interactive",
            });
            (url, body)
        };

        let resp = self
            .http
            .post(&url)
            .header("Authorization", format!("Bearer {token}"))
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        let json: serde_json::Value = resp.json().await?;

        if !status.is_success() || json.get("code").and_then(|c| c.as_i64()) != Some(0) {
            let api_msg = json
                .get("msg")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown error");
            warn!(
                code = ?json.get("code"),
                msg = api_msg,
                "Feishu send_message failed"
            );
            return Err(BotError::ApiError(format!(
                "Feishu send failed: {api_msg}"
            )));
        }

        let sent_message_id = json
            .pointer("/data/message_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        debug!(message_id = %sent_message_id, "Feishu message sent");
        Ok(sent_message_id)
    }

    fn verify_signature(
        &self,
        headers: &HashMap<String, String>,
        body: &[u8],
    ) -> Result<(), BotError> {
        let encrypt_key = match &self.config.encrypt_key {
            Some(key) if !key.trim().is_empty() => key.trim(),
            _ => return Ok(()), // No encrypt key configured — skip verification.
        };

        let timestamp = headers
            .get("x-lark-request-timestamp")
            .ok_or(BotError::SignatureInvalid)?;
        let nonce = headers
            .get("x-lark-request-nonce")
            .ok_or(BotError::SignatureInvalid)?;
        let signature = headers
            .get("x-lark-signature")
            .ok_or(BotError::SignatureInvalid)?;

        // Feishu signature = SHA-256(timestamp + nonce + encrypt_key + body)
        let body_str = std::str::from_utf8(body).unwrap_or("");
        let mut hasher = Sha256::new();
        hasher.update(timestamp.as_bytes());
        hasher.update(nonce.as_bytes());
        hasher.update(encrypt_key.as_bytes());
        hasher.update(body_str.as_bytes());

        let computed = format!("{:x}", hasher.finalize());

        if computed == *signature {
            Ok(())
        } else {
            Err(BotError::SignatureInvalid)
        }
    }

    fn handle_challenge(&self, payload: &serde_json::Value) -> Option<serde_json::Value> {
        // Feishu URL verification: payload contains { "challenge": "...", "token": "...", "type": "url_verification" }
        if payload.get("type").and_then(|v| v.as_str()) == Some("url_verification") {
            if let Some(challenge) = payload.get("challenge") {
                return Some(serde_json::json!({ "challenge": challenge }));
            }
        }

        // Schema 2.0 encrypted challenge: { "encrypt": "..." }
        // For now, we do not handle encrypted challenges (requires AES decryption).
        // Users should prefer non-encrypted verification or use the Lark SDK helper.
        if payload.get("encrypt").is_some() && payload.get("header").is_none() {
            warn!("Received encrypted challenge payload; encrypted challenges are not yet supported — configure without encrypt_key for URL verification");
        }

        None
    }

    fn platform_kind(&self) -> PlatformKind {
        PlatformKind::Feishu
    }
}

// ---------------------------------------------------------------------------
// Content parsing helpers
// ---------------------------------------------------------------------------

/// Parse Feishu message content based on message type.
///
/// Inspired by OpenClaw's `parseMessageContent()` in `bot.ts`.
fn parse_feishu_content(raw: &str, message_type: &str) -> String {
    match message_type {
        "text" => {
            // Text messages: JSON `{ "text": "..." }`
            serde_json::from_str::<serde_json::Value>(raw)
                .ok()
                .and_then(|v| v.get("text").and_then(|t| t.as_str()).map(String::from))
                .unwrap_or_else(|| raw.to_string())
        }
        "post" => {
            // Rich text (post): JSON with nested content arrays.
            parse_post_content(raw)
        }
        "image" => "[Image]".to_string(),
        "file" => "[File]".to_string(),
        "audio" => "[Audio]".to_string(),
        "video" | "media" => "[Video]".to_string(),
        "sticker" => "[Sticker]".to_string(),
        "share_chat" => "[Shared Chat]".to_string(),
        "merge_forward" => "[Merged Forward]".to_string(),
        _ => raw.to_string(),
    }
}

/// Extract text from a Feishu post (rich text) message.
///
/// Post messages have locale-keyed content with nested element arrays.
fn parse_post_content(raw: &str) -> String {
    let parsed: serde_json::Value = match serde_json::from_str(raw) {
        Ok(v) => v,
        Err(_) => return raw.to_string(),
    };

    // Try zh_cn, en_us, or the first available locale.
    let post_obj = parsed.as_object();
    let locale_content = post_obj
        .and_then(|obj| {
            obj.get("zh_cn")
                .or_else(|| obj.get("en_us"))
                .or_else(|| obj.values().next())
        });

    let content_arr = locale_content
        .and_then(|lc| lc.get("content"))
        .and_then(|c| c.as_array());

    let Some(paragraphs) = content_arr else {
        return raw.to_string();
    };

    let mut texts = Vec::new();
    for paragraph in paragraphs {
        if let Some(elements) = paragraph.as_array() {
            for elem in elements {
                let tag = elem.get("tag").and_then(|t| t.as_str()).unwrap_or("");
                match tag {
                    "text" => {
                        if let Some(text) = elem.get("text").and_then(|t| t.as_str()) {
                            texts.push(text.to_string());
                        }
                    }
                    "a" => {
                        if let Some(text) = elem.get("text").and_then(|t| t.as_str()) {
                            let href = elem.get("href").and_then(|h| h.as_str()).unwrap_or("");
                            if href.is_empty() {
                                texts.push(text.to_string());
                            } else {
                                texts.push(format!("[{text}]({href})"));
                            }
                        }
                    }
                    "at" => {
                        if let Some(name) = elem.get("user_name").and_then(|n| n.as_str()) {
                            texts.push(format!("@{name}"));
                        }
                    }
                    "md" => {
                        if let Some(text) = elem.get("text").and_then(|t| t.as_str()) {
                            texts.push(text.to_string());
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    texts.join("").trim().to_string()
}

/// Strip `@bot` mentions from the content text.
///
/// Feishu inline mentions use the format `@_user_X` in the text field.
/// The `mentions` array in the event provides the mapping. This function
/// removes the bot's own mention so the remaining text represents the
/// user's actual input.
pub fn strip_bot_mention(content: &str, mentions: &serde_json::Value, bot_open_id: &str) -> String {
    let Some(mentions_arr) = mentions.as_array() else {
        return content.to_string();
    };

    let mut result = content.to_string();
    for mention in mentions_arr {
        let open_id = mention
            .pointer("/id/open_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if open_id == bot_open_id {
            if let Some(key) = mention.get("key").and_then(|k| k.as_str()) {
                result = result.replace(key, "");
            }
        }
    }

    result.trim().to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_text_message() {
        let raw = r#"{"text":"Hello world"}"#;
        let result = parse_feishu_content(raw, "text");
        assert_eq!(result, "Hello world");
    }

    #[test]
    fn test_parse_text_message_with_mention() {
        let raw = r#"{"text":"@_user_1 help me"}"#;
        let result = parse_feishu_content(raw, "text");
        assert_eq!(result, "@_user_1 help me");
    }

    #[test]
    fn test_parse_post_message() {
        let raw = r#"{"zh_cn":{"content":[[{"tag":"text","text":"Hello "},{"tag":"text","text":"world"}]]}}"#;
        let result = parse_feishu_content(raw, "post");
        assert_eq!(result, "Hello world");
    }

    #[test]
    fn test_parse_image_message() {
        let result = parse_feishu_content(r#"{"image_key":"img_xxx"}"#, "image");
        assert_eq!(result, "[Image]");
    }

    #[test]
    fn test_parse_event_text_message() {
        let bot = FeishuBot::new(FeishuBotConfig::default());
        let payload = serde_json::json!({
            "schema": "2.0",
            "header": {
                "event_id": "evt_123",
                "event_type": "im.message.receive_v1",
                "create_time": "1700000000000",
            },
            "event": {
                "sender": {
                    "sender_id": { "open_id": "ou_abc123" },
                    "sender_type": "user",
                },
                "message": {
                    "message_id": "om_xyz",
                    "chat_id": "oc_group1",
                    "chat_type": "group",
                    "message_type": "text",
                    "content": "{\"text\":\"@_user_1 Hello bot\"}",
                    "create_time": "1700000000000",
                    "root_id": "",
                    "mentions": [{
                        "key": "@_user_1",
                        "id": { "open_id": "ou_bot" },
                        "name": "TestBot",
                    }],
                }
            }
        });

        let msg = bot.parse_event(&payload).unwrap();
        assert_eq!(msg.platform, PlatformKind::Feishu);
        assert_eq!(msg.chat_id, "oc_group1");
        assert_eq!(msg.message_id, "om_xyz");
        assert_eq!(msg.sender_id, "ou_abc123");
        assert_eq!(msg.chat_type, ChatType::Group);
        assert_eq!(msg.content, "@_user_1 Hello bot");
    }

    #[test]
    fn test_parse_event_unsupported_type() {
        let bot = FeishuBot::new(FeishuBotConfig::default());
        let payload = serde_json::json!({
            "schema": "2.0",
            "header": {
                "event_type": "im.chat.member.bot.added_v1",
            },
            "event": {}
        });

        let result = bot.parse_event(&payload);
        assert!(matches!(result, Err(BotError::UnsupportedEvent(_))));
    }

    #[test]
    fn test_challenge_detection() {
        let bot = FeishuBot::new(FeishuBotConfig::default());

        let challenge_payload = serde_json::json!({
            "challenge": "abc-123-challenge",
            "token": "verify-token",
            "type": "url_verification",
        });
        let resp = bot.handle_challenge(&challenge_payload);
        assert!(resp.is_some());
        assert_eq!(
            resp.unwrap().get("challenge").unwrap().as_str().unwrap(),
            "abc-123-challenge"
        );

        // Normal event should not be a challenge.
        let normal_payload = serde_json::json!({
            "schema": "2.0",
            "header": { "event_type": "im.message.receive_v1" },
            "event": {}
        });
        assert!(bot.handle_challenge(&normal_payload).is_none());
    }

    #[test]
    fn test_signature_verify_valid() {
        let config = FeishuBotConfig {
            encrypt_key: Some("test-encrypt-key".to_string()),
            ..Default::default()
        };
        let bot = FeishuBot::new(config);

        let timestamp = "1700000000";
        let nonce = "abc123";
        let body = r#"{"event":"test"}"#;

        // Compute expected signature.
        let mut hasher = Sha256::new();
        hasher.update(timestamp.as_bytes());
        hasher.update(nonce.as_bytes());
        hasher.update(b"test-encrypt-key");
        hasher.update(body.as_bytes());
        let expected = format!("{:x}", hasher.finalize());

        let mut headers = HashMap::new();
        headers.insert("x-lark-request-timestamp".to_string(), timestamp.to_string());
        headers.insert("x-lark-request-nonce".to_string(), nonce.to_string());
        headers.insert("x-lark-signature".to_string(), expected);

        assert!(bot.verify_signature(&headers, body.as_bytes()).is_ok());
    }

    #[test]
    fn test_signature_verify_invalid() {
        let config = FeishuBotConfig {
            encrypt_key: Some("test-encrypt-key".to_string()),
            ..Default::default()
        };
        let bot = FeishuBot::new(config);

        let mut headers = HashMap::new();
        headers.insert(
            "x-lark-request-timestamp".to_string(),
            "1700000000".to_string(),
        );
        headers.insert("x-lark-request-nonce".to_string(), "abc123".to_string());
        headers.insert(
            "x-lark-signature".to_string(),
            "invalid_signature".to_string(),
        );

        let result = bot.verify_signature(&headers, b"test body");
        assert!(matches!(result, Err(BotError::SignatureInvalid)));
    }

    #[test]
    fn test_signature_skip_when_no_encrypt_key() {
        let bot = FeishuBot::new(FeishuBotConfig::default());
        let headers = HashMap::new();
        // Should succeed even without any headers when no encrypt_key is configured.
        assert!(bot.verify_signature(&headers, b"anything").is_ok());
    }

    #[test]
    fn test_strip_bot_mention() {
        let mentions = serde_json::json!([{
            "key": "@_user_1",
            "id": { "open_id": "ou_bot" },
            "name": "TestBot",
        }]);
        let result = strip_bot_mention("@_user_1 Hello bot", &mentions, "ou_bot");
        assert_eq!(result, "Hello bot");
    }

    #[test]
    fn test_strip_bot_mention_preserves_other() {
        let mentions = serde_json::json!([
            { "key": "@_user_1", "id": { "open_id": "ou_bot" }, "name": "Bot" },
            { "key": "@_user_2", "id": { "open_id": "ou_human" }, "name": "Alice" },
        ]);
        let result = strip_bot_mention("@_user_1 Hello @_user_2", &mentions, "ou_bot");
        assert_eq!(result, "Hello @_user_2");
    }

    #[test]
    fn test_api_base_url() {
        let feishu = FeishuBotConfig {
            domain: "feishu".to_string(),
            ..Default::default()
        };
        assert_eq!(feishu.api_base_url(), "https://open.feishu.cn/open-apis");

        let lark = FeishuBotConfig {
            domain: "lark".to_string(),
            ..Default::default()
        };
        assert_eq!(lark.api_base_url(), "https://open.larksuite.com/open-apis");

        let custom = FeishuBotConfig {
            domain: "https://custom.example.com".to_string(),
            ..Default::default()
        };
        assert_eq!(
            custom.api_base_url(),
            "https://custom.example.com/open-apis"
        );
    }
}
