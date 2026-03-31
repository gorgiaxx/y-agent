//! Discord Gateway (WebSocket) client.
//!
//! Maintains a persistent WebSocket connection to `wss://gateway.discord.gg`
//! so the bot appears **online** and receives real-time events such as
//! `MESSAGE_CREATE`.
//!
//! Protocol summary (Discord Gateway v10):
//! 1. Connect to `wss://gateway.discord.gg/?v=10&encoding=json`
//! 2. Receive **Hello** (op 10) with `heartbeat_interval`
//! 3. Send **Identify** (op 2) with bot token + intents
//! 4. Receive **Ready** (op 0, t=READY) -- bot is now online
//! 5. Send **Heartbeat** (op 1) at the given interval
//! 6. Receive **Dispatch** (op 0) events like `MESSAGE_CREATE`
//! 7. On disconnect: reconnect with **Resume** (op 6)

use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tracing::{debug, info, warn};

use super::discord::DiscordBotConfig;
use super::{ChatType, InboundMessage, PlatformKind};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const GATEWAY_URL: &str = "wss://gateway.discord.gg/?v=10&encoding=json";

/// Gateway intents (bitfield):
/// - GUILDS            (1 << 0)  = 1
/// - `GUILD_MESSAGES`    (1 << 9)  = 512
/// - `DIRECT_MESSAGES`   (1 << 12) = 4096
/// - `MESSAGE_CONTENT`   (1 << 15) = 32768  (privileged)
const GATEWAY_INTENTS: u64 = 1 | 512 | 4096 | 32768;

/// Gateway opcodes.
mod op {
    pub const DISPATCH: u64 = 0;
    pub const HEARTBEAT: u64 = 1;
    pub const IDENTIFY: u64 = 2;
    pub const RESUME: u64 = 6;
    pub const RECONNECT: u64 = 7;
    pub const INVALID_SESSION: u64 = 9;
    pub const HELLO: u64 = 10;
    pub const HEARTBEAT_ACK: u64 = 11;
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Handle to a running Discord Gateway connection.
///
/// Drop this handle to shut down the gateway task.
pub struct GatewayHandle {
    /// Receive parsed inbound messages from the gateway.
    pub rx: mpsc::UnboundedReceiver<InboundMessage>,
    /// Abort handle for the background task.
    _abort: tokio::task::AbortHandle,
}

/// Start the Discord Gateway connection in a background task.
///
/// Returns a [`GatewayHandle`] whose `rx` field yields `InboundMessage`
/// values for every `MESSAGE_CREATE` event received from Discord.
///
/// The task automatically reconnects on disconnection with exponential
/// backoff, and uses Resume when possible to avoid missing events.
pub fn start_gateway(config: Arc<DiscordBotConfig>) -> GatewayHandle {
    let (tx, rx) = mpsc::unbounded_channel();

    let task = tokio::spawn(gateway_loop(config, tx));
    let abort = task.abort_handle();

    GatewayHandle { rx, _abort: abort }
}

// ---------------------------------------------------------------------------
// Connection state
// ---------------------------------------------------------------------------

/// Persistent state across reconnections (for Resume).
struct SessionState {
    session_id: Option<String>,
    resume_gateway_url: Option<String>,
    sequence: Option<u64>,
}

impl SessionState {
    fn new() -> Self {
        Self {
            session_id: None,
            resume_gateway_url: None,
            sequence: None,
        }
    }

    fn can_resume(&self) -> bool {
        self.session_id.is_some() && self.sequence.is_some()
    }

    fn clear(&mut self) {
        self.session_id = None;
        self.resume_gateway_url = None;
        self.sequence = None;
    }
}

// ---------------------------------------------------------------------------
// Main loop
// ---------------------------------------------------------------------------

/// Outer loop: connect -> run -> reconnect with backoff.
async fn gateway_loop(config: Arc<DiscordBotConfig>, tx: mpsc::UnboundedSender<InboundMessage>) {
    let mut state = SessionState::new();
    let mut backoff_secs: u64 = 1;

    loop {
        let url = state
            .resume_gateway_url
            .as_deref()
            .unwrap_or(GATEWAY_URL)
            .to_string();

        info!(url = %url, "Discord Gateway: connecting");

        match run_session(&config, &tx, &mut state, &url).await {
            Ok(()) => {
                // Clean disconnect (e.g. shutdown).
                info!("Discord Gateway: session ended cleanly");
                break;
            }
            Err(e) => {
                warn!(
                    error = %e,
                    backoff_secs,
                    "Discord Gateway: session error, reconnecting"
                );
                tokio::time::sleep(std::time::Duration::from_secs(backoff_secs)).await;
                // Exponential backoff capped at 60s.
                backoff_secs = (backoff_secs * 2).min(60);
            }
        }
    }
}

/// A single gateway session: connect, identify/resume, then read events.
async fn run_session(
    config: &DiscordBotConfig,
    tx: &mpsc::UnboundedSender<InboundMessage>,
    state: &mut SessionState,
    url: &str,
) -> Result<(), GatewayError> {
    // 1. WebSocket connect.
    let (ws_stream, _response) = tokio_tungstenite::connect_async(url)
        .await
        .map_err(|e| GatewayError::Connection(e.to_string()))?;

    let (mut sink, mut stream) = ws_stream.split();

    // 2. Receive Hello (op 10).
    let heartbeat_interval = recv_hello(&mut stream).await?;
    info!(
        heartbeat_interval_ms = heartbeat_interval,
        "Discord Gateway: received Hello"
    );

    // 3. Send Identify or Resume.
    if state.can_resume() {
        send_resume(
            &mut sink,
            &config.token,
            state.session_id.as_deref().unwrap_or_default(),
            state.sequence.unwrap_or(0),
        )
        .await?;
        debug!("Discord Gateway: sent Resume");
    } else {
        send_identify(&mut sink, &config.token).await?;
        debug!("Discord Gateway: sent Identify");
    }

    // Reset backoff on successful connection.
    // (caller manages backoff, but we can signal success via Ok path)

    // 4. Event loop with heartbeat.
    let heartbeat_dur = std::time::Duration::from_millis(heartbeat_interval);
    let mut heartbeat_timer = tokio::time::interval(heartbeat_dur);
    // Skip the first immediate tick.
    heartbeat_timer.tick().await;

    let mut got_ack = true;

    loop {
        tokio::select! {
            // Heartbeat tick.
            _ = heartbeat_timer.tick() => {
                if !got_ack {
                    warn!("Discord Gateway: missed Heartbeat ACK, reconnecting");
                    return Err(GatewayError::HeartbeatTimeout);
                }
                send_heartbeat(&mut sink, state.sequence).await?;
                got_ack = false;
            }

            // Incoming WebSocket message.
            msg = stream.next() => {
                let Some(msg) = msg else {
                    return Err(GatewayError::Connection("stream ended".into()));
                };
                let msg = msg.map_err(|e| GatewayError::Connection(e.to_string()))?;

                match msg {
                    WsMessage::Text(text) => {
                        let action = handle_event(
                            &text, tx, state,
                        );
                        match action {
                            EventAction::Continue => {}
                            EventAction::HeartbeatAck => { got_ack = true; }
                            EventAction::Reconnect => {
                                info!("Discord Gateway: server requested reconnect");
                                return Err(GatewayError::Reconnect);
                            }
                            EventAction::InvalidSession(resumable) => {
                                warn!(resumable, "Discord Gateway: invalid session");
                                if !resumable {
                                    state.clear();
                                }
                                return Err(GatewayError::InvalidSession);
                            }
                        }
                    }
                    WsMessage::Close(frame) => {
                        info!(?frame, "Discord Gateway: received Close");
                        return Err(GatewayError::Closed);
                    }
                    // Ping/Pong/Binary -- ignore.
                    _ => {}
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Gateway protocol helpers
// ---------------------------------------------------------------------------

type WsSink = futures_util::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    WsMessage,
>;

type WsStream = futures_util::stream::SplitStream<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
>;

/// Wait for the Hello event and extract `heartbeat_interval`.
async fn recv_hello(stream: &mut WsStream) -> Result<u64, GatewayError> {
    // Read up to 5 messages looking for Hello.
    for _ in 0..5 {
        let Some(msg) = stream.next().await else {
            return Err(GatewayError::Connection("stream ended before Hello".into()));
        };
        let msg = msg.map_err(|e| GatewayError::Connection(e.to_string()))?;

        if let WsMessage::Text(ref text) = msg {
            if let Ok(payload) = serde_json::from_str::<serde_json::Value>(text) {
                if payload.get("op").and_then(serde_json::Value::as_u64) == Some(op::HELLO) {
                    let interval = payload
                        .get("d")
                        .and_then(|d| d.get("heartbeat_interval"))
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(41250);
                    return Ok(interval);
                }
            }
        }
    }
    Err(GatewayError::Protocol("did not receive Hello".into()))
}

/// Send the Identify payload (op 2).
async fn send_identify(sink: &mut WsSink, token: &str) -> Result<(), GatewayError> {
    let payload = serde_json::json!({
        "op": op::IDENTIFY,
        "d": {
            "token": token,
            "intents": GATEWAY_INTENTS,
            "properties": {
                "os": std::env::consts::OS,
                "browser": "y-agent",
                "device": "y-agent",
            },
        },
    });
    send_json(sink, &payload).await
}

/// Send the Resume payload (op 6).
async fn send_resume(
    sink: &mut WsSink,
    token: &str,
    session_id: &str,
    seq: u64,
) -> Result<(), GatewayError> {
    let payload = serde_json::json!({
        "op": op::RESUME,
        "d": {
            "token": token,
            "session_id": session_id,
            "seq": seq,
        },
    });
    send_json(sink, &payload).await
}

/// Send a Heartbeat (op 1).
async fn send_heartbeat(sink: &mut WsSink, seq: Option<u64>) -> Result<(), GatewayError> {
    let payload = serde_json::json!({
        "op": op::HEARTBEAT,
        "d": seq,
    });
    send_json(sink, &payload).await
}

/// Serialize and send a JSON payload.
async fn send_json(sink: &mut WsSink, value: &serde_json::Value) -> Result<(), GatewayError> {
    let text = serde_json::to_string(value)
        .map_err(|e| GatewayError::Protocol(format!("JSON serialize: {e}")))?;
    sink.send(WsMessage::Text(text.into()))
        .await
        .map_err(|e| GatewayError::Connection(e.to_string()))
}

// ---------------------------------------------------------------------------
// Event dispatch
// ---------------------------------------------------------------------------

enum EventAction {
    Continue,
    HeartbeatAck,
    Reconnect,
    InvalidSession(bool),
}

/// Parse a gateway event and take appropriate action.
fn handle_event(
    text: &str,
    tx: &mpsc::UnboundedSender<InboundMessage>,
    state: &mut SessionState,
) -> EventAction {
    let payload: serde_json::Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => {
            warn!(error = %e, "Discord Gateway: failed to parse event JSON");
            return EventAction::Continue;
        }
    };

    let opcode = payload
        .get("op")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(99);

    // Update sequence number for heartbeat and resume.
    if let Some(seq) = payload.get("s").and_then(serde_json::Value::as_u64) {
        state.sequence = Some(seq);
    }

    match opcode {
        op::DISPATCH => {
            let event_type = payload
                .get("t")
                .and_then(|v| v.as_str())
                .unwrap_or_default();

            match event_type {
                "READY" => {
                    // Extract session_id and resume_gateway_url.
                    if let Some(d) = payload.get("d") {
                        state.session_id = d
                            .get("session_id")
                            .and_then(|v| v.as_str())
                            .map(String::from);
                        state.resume_gateway_url = d
                            .get("resume_gateway_url")
                            .and_then(|v| v.as_str())
                            .map(|u| format!("{u}/?v=10&encoding=json"));

                        let user_tag = d
                            .get("user")
                            .and_then(|u| u.get("username"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown");
                        info!(
                            session_id = ?state.session_id,
                            bot = user_tag,
                            "Discord Gateway: READY -- bot is online"
                        );
                    }
                }
                "RESUMED" => {
                    info!("Discord Gateway: RESUMED -- connection restored");
                }
                "MESSAGE_CREATE" => {
                    if let Some(d) = payload.get("d") {
                        handle_message_create(d, tx);
                    }
                }
                _ => {
                    debug!(event_type, "Discord Gateway: ignoring dispatch event");
                }
            }
            EventAction::Continue
        }
        op::HEARTBEAT_ACK => EventAction::HeartbeatAck,
        op::HEARTBEAT => {
            // Server-requested heartbeat (rare). We handle it on the next tick.
            EventAction::Continue
        }
        op::RECONNECT => EventAction::Reconnect,
        op::INVALID_SESSION => {
            let resumable = payload
                .get("d")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            EventAction::InvalidSession(resumable)
        }
        _ => {
            debug!(opcode, "Discord Gateway: unknown opcode");
            EventAction::Continue
        }
    }
}

/// Parse a `MESSAGE_CREATE` dispatch event and forward as `InboundMessage`.
fn handle_message_create(data: &serde_json::Value, tx: &mpsc::UnboundedSender<InboundMessage>) {
    // Skip messages from bots (including ourselves).
    let is_bot = data
        .get("author")
        .and_then(|a| a.get("bot"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    if is_bot {
        return;
    }

    let message_id = data
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();

    let channel_id = data
        .get("channel_id")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();

    let sender_id = data
        .get("author")
        .and_then(|a| a.get("id"))
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();

    let sender_name = data
        .get("author")
        .and_then(|a| a.get("username"))
        .and_then(|v| v.as_str())
        .map(String::from);

    let content = data
        .get("content")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();

    // Skip empty messages (e.g. image-only, embed-only).
    if content.trim().is_empty() {
        return;
    }

    let chat_type = if data.get("guild_id").and_then(|v| v.as_str()).is_some() {
        ChatType::Group
    } else {
        ChatType::P2p
    };

    let reply_to_message_id = data
        .get("message_reference")
        .and_then(|r| r.get("message_id"))
        .and_then(|v| v.as_str())
        .map(String::from);

    let timestamp = data
        .get("timestamp")
        .and_then(|v| v.as_str())
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map_or_else(chrono::Utc::now, |dt| dt.with_timezone(&chrono::Utc));

    let msg = InboundMessage {
        platform: PlatformKind::Discord,
        chat_id: channel_id,
        message_id: message_id.clone(),
        sender_id,
        sender_name,
        content,
        chat_type,
        reply_to_message_id,
        attachments: vec![],
        timestamp,
        raw: data.clone(),
    };

    debug!(message_id = %message_id, "Discord Gateway: forwarding MESSAGE_CREATE");

    if tx.send(msg).is_err() {
        warn!("Discord Gateway: message channel closed, dropping event");
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
enum GatewayError {
    #[error("connection error: {0}")]
    Connection(String),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("heartbeat timeout")]
    HeartbeatTimeout,
    #[error("server requested reconnect")]
    Reconnect,
    #[error("invalid session")]
    InvalidSession,
    #[error("connection closed")]
    Closed,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_state_resume_logic() {
        let mut state = SessionState::new();
        assert!(!state.can_resume());

        state.session_id = Some("sess_123".into());
        assert!(!state.can_resume()); // no sequence yet

        state.sequence = Some(42);
        assert!(state.can_resume());

        state.clear();
        assert!(!state.can_resume());
    }

    #[test]
    fn test_handle_event_heartbeat_ack() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut state = SessionState::new();
        let json = r#"{"op": 11}"#;

        let action = handle_event(json, &tx, &mut state);
        assert!(matches!(action, EventAction::HeartbeatAck));
    }

    #[test]
    fn test_handle_event_reconnect() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut state = SessionState::new();
        let json = r#"{"op": 7}"#;

        let action = handle_event(json, &tx, &mut state);
        assert!(matches!(action, EventAction::Reconnect));
    }

    #[test]
    fn test_handle_event_invalid_session_resumable() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut state = SessionState::new();
        let json = r#"{"op": 9, "d": true}"#;

        let action = handle_event(json, &tx, &mut state);
        assert!(matches!(action, EventAction::InvalidSession(true)));
    }

    #[test]
    fn test_handle_event_invalid_session_non_resumable() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut state = SessionState::new();
        let json = r#"{"op": 9, "d": false}"#;

        let action = handle_event(json, &tx, &mut state);
        assert!(matches!(action, EventAction::InvalidSession(false)));
    }

    #[test]
    fn test_handle_event_ready() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut state = SessionState::new();
        let json = serde_json::json!({
            "op": 0,
            "s": 1,
            "t": "READY",
            "d": {
                "session_id": "abc123",
                "resume_gateway_url": "wss://resume.example.com",
                "user": { "username": "TestBot" },
            }
        })
        .to_string();

        let action = handle_event(&json, &tx, &mut state);
        assert!(matches!(action, EventAction::Continue));
        assert_eq!(state.session_id.as_deref(), Some("abc123"));
        assert_eq!(state.sequence, Some(1));
        assert!(state
            .resume_gateway_url
            .as_deref()
            .unwrap()
            .contains("resume.example.com"));
    }

    #[test]
    fn test_handle_event_message_create() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut state = SessionState::new();
        let json = serde_json::json!({
            "op": 0,
            "s": 5,
            "t": "MESSAGE_CREATE",
            "d": {
                "id": "msg_1",
                "channel_id": "ch_1",
                "author": {
                    "id": "user_1",
                    "username": "Alice",
                },
                "content": "Hello bot!",
                "timestamp": "2024-06-15T12:00:00.000000+00:00",
                "guild_id": "guild_1",
            }
        })
        .to_string();

        let action = handle_event(&json, &tx, &mut state);
        assert!(matches!(action, EventAction::Continue));
        assert_eq!(state.sequence, Some(5));

        let msg = rx.try_recv().unwrap();
        assert_eq!(msg.platform, PlatformKind::Discord);
        assert_eq!(msg.message_id, "msg_1");
        assert_eq!(msg.content, "Hello bot!");
        assert_eq!(msg.chat_type, ChatType::Group);
    }

    #[test]
    fn test_handle_event_message_create_skips_bots() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut state = SessionState::new();
        let json = serde_json::json!({
            "op": 0,
            "s": 6,
            "t": "MESSAGE_CREATE",
            "d": {
                "id": "msg_2",
                "channel_id": "ch_1",
                "author": {
                    "id": "bot_1",
                    "username": "OtherBot",
                    "bot": true,
                },
                "content": "I am a bot",
                "timestamp": "2024-06-15T12:00:00.000000+00:00",
            }
        })
        .to_string();

        handle_event(&json, &tx, &mut state);
        // Bot messages should not be forwarded.
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn test_handle_event_sequence_tracking() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut state = SessionState::new();
        assert!(state.sequence.is_none());

        let json1 = r#"{"op": 0, "s": 10, "t": "GUILD_CREATE", "d": {}}"#;
        handle_event(json1, &tx, &mut state);
        assert_eq!(state.sequence, Some(10));

        let json2 = r#"{"op": 0, "s": 15, "t": "TYPING_START", "d": {}}"#;
        handle_event(json2, &tx, &mut state);
        assert_eq!(state.sequence, Some(15));
    }
}
