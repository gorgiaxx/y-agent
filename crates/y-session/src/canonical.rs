//! Canonical sessions — cross-channel session management.
//!
//! Design reference: session-design.md §Canonical Sessions
//!
//! A canonical session merges messages from multiple channels (CLI, API, Web)
//! into a unified view. Each channel produces a child session, and the
//! canonical session is the logical parent that aggregates the conversation.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;

use y_core::session::{SessionStore, TranscriptStore};
use y_core::types::{Message, Role, SessionId};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// The channel through which a message was received.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Channel {
    Cli,
    Api,
    Web,
    Custom(String),
}

impl std::fmt::Display for Channel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Channel::Cli => write!(f, "cli"),
            Channel::Api => write!(f, "api"),
            Channel::Web => write!(f, "web"),
            Channel::Custom(name) => write!(f, "{name}"),
        }
    }
}

/// A timestamped, channel-tagged message for canonical ordering.
#[derive(Debug, Clone)]
pub struct CanonicalMessage {
    /// The original message.
    pub message: Message,
    /// The source channel.
    pub channel: Channel,
    /// Ordering timestamp (monotonic within the canonical session).
    pub sequence: u64,
}

/// Configuration for canonical session behavior.
#[derive(Debug, Clone)]
pub struct CanonicalConfig {
    /// Maximum number of channels per canonical session.
    pub max_channels: usize,
    /// Whether to auto-merge messages from all channels.
    pub auto_merge: bool,
}

impl Default for CanonicalConfig {
    fn default() -> Self {
        Self {
            max_channels: 8,
            auto_merge: true,
        }
    }
}

// ---------------------------------------------------------------------------
// CanonicalSessionManager
// ---------------------------------------------------------------------------

/// Manages canonical (cross-channel) sessions.
///
/// Each canonical session is identified by a `SessionId` and can have
/// multiple channel-specific sub-sessions. Messages from all channels
/// are merged into a unified transcript in chronological order.
pub struct CanonicalSessionManager {
    /// Map from canonical session ID to channel → child session ID.
    channel_map: RwLock<HashMap<SessionId, ChannelState>>,
    /// Session store for persistence.
    _session_store: Arc<dyn SessionStore>,
    /// Transcript store for persistence.
    _transcript_store: Arc<dyn TranscriptStore>,
    /// Sequence counter per canonical session.
    sequence_counters: RwLock<HashMap<SessionId, u64>>,
    /// Configuration.
    config: CanonicalConfig,
}

/// Internal state for a canonical session's channels.
#[derive(Debug, Clone)]
struct ChannelState {
    /// Map from channel to child session ID.
    channels: HashMap<Channel, SessionId>,
    /// Merged canonical transcript.
    transcript: Vec<CanonicalMessage>,
}

impl CanonicalSessionManager {
    /// Create a new canonical session manager.
    pub fn new(
        session_store: Arc<dyn SessionStore>,
        transcript_store: Arc<dyn TranscriptStore>,
        config: CanonicalConfig,
    ) -> Self {
        Self {
            channel_map: RwLock::new(HashMap::new()),
            _session_store: session_store,
            _transcript_store: transcript_store,
            sequence_counters: RwLock::new(HashMap::new()),
            config,
        }
    }

    /// Register a new channel for a canonical session.
    ///
    /// Creates the child session for the channel if it doesn't exist.
    pub async fn register_channel(
        &self,
        canonical_id: &SessionId,
        channel: Channel,
        child_session_id: SessionId,
    ) -> Result<(), CanonicalError> {
        let mut map = self.channel_map.write().await;
        let state = map
            .entry(canonical_id.clone())
            .or_insert_with(|| ChannelState {
                channels: HashMap::new(),
                transcript: Vec::new(),
            });

        if state.channels.len() >= self.config.max_channels {
            return Err(CanonicalError::TooManyChannels {
                canonical_id: canonical_id.clone(),
                max: self.config.max_channels,
            });
        }

        state.channels.insert(channel, child_session_id);
        Ok(())
    }

    /// Append a message from a specific channel to the canonical transcript.
    pub async fn append_message(
        &self,
        canonical_id: &SessionId,
        channel: Channel,
        message: Message,
    ) -> Result<u64, CanonicalError> {
        // Get the next sequence number.
        let sequence = {
            let mut counters = self.sequence_counters.write().await;
            let counter = counters.entry(canonical_id.clone()).or_insert(0);
            *counter += 1;
            *counter
        };

        let canonical_msg = CanonicalMessage {
            message,
            channel,
            sequence,
        };

        let mut map = self.channel_map.write().await;
        let state = map
            .get_mut(canonical_id)
            .ok_or_else(|| CanonicalError::NotFound {
                canonical_id: canonical_id.clone(),
            })?;

        state.transcript.push(canonical_msg);

        Ok(sequence)
    }

    /// Get the merged canonical transcript in sequence order.
    pub async fn canonical_transcript(
        &self,
        canonical_id: &SessionId,
    ) -> Result<Vec<CanonicalMessage>, CanonicalError> {
        let map = self.channel_map.read().await;
        let state = map
            .get(canonical_id)
            .ok_or_else(|| CanonicalError::NotFound {
                canonical_id: canonical_id.clone(),
            })?;

        let mut transcript = state.transcript.clone();
        transcript.sort_by_key(|m| m.sequence);
        Ok(transcript)
    }

    /// Get the child session ID for a specific channel.
    pub async fn channel_session(
        &self,
        canonical_id: &SessionId,
        channel: &Channel,
    ) -> Result<SessionId, CanonicalError> {
        let map = self.channel_map.read().await;
        let state = map
            .get(canonical_id)
            .ok_or_else(|| CanonicalError::NotFound {
                canonical_id: canonical_id.clone(),
            })?;

        state
            .channels
            .get(channel)
            .cloned()
            .ok_or_else(|| CanonicalError::ChannelNotFound {
                canonical_id: canonical_id.clone(),
                channel: format!("{channel}"),
            })
    }

    /// List all channels registered for a canonical session.
    pub async fn list_channels(
        &self,
        canonical_id: &SessionId,
    ) -> Result<Vec<Channel>, CanonicalError> {
        let map = self.channel_map.read().await;
        let state = map
            .get(canonical_id)
            .ok_or_else(|| CanonicalError::NotFound {
                canonical_id: canonical_id.clone(),
            })?;

        Ok(state.channels.keys().cloned().collect())
    }

    /// Get the canonical transcript filtered for a specific role.
    pub async fn messages_by_role(
        &self,
        canonical_id: &SessionId,
        role: &Role,
    ) -> Result<Vec<CanonicalMessage>, CanonicalError> {
        let transcript = self.canonical_transcript(canonical_id).await?;
        Ok(transcript
            .into_iter()
            .filter(|m| &m.message.role == role)
            .collect())
    }

    /// Get the total message count for a canonical session.
    pub async fn message_count(&self, canonical_id: &SessionId) -> Result<usize, CanonicalError> {
        let map = self.channel_map.read().await;
        let state = map
            .get(canonical_id)
            .ok_or_else(|| CanonicalError::NotFound {
                canonical_id: canonical_id.clone(),
            })?;
        Ok(state.transcript.len())
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors from canonical session operations.
#[derive(Debug, thiserror::Error)]
pub enum CanonicalError {
    #[error("canonical session not found: {canonical_id}")]
    NotFound { canonical_id: SessionId },

    #[error("channel not found: {channel} in canonical session {canonical_id}")]
    ChannelNotFound {
        canonical_id: SessionId,
        channel: String,
    },

    #[error("too many channels for canonical session {canonical_id} (max: {max})")]
    TooManyChannels { canonical_id: SessionId, max: usize },
}

#[cfg(test)]
mod tests {
    use super::*;
    use y_core::session::{
        CreateSessionOptions, SessionError, SessionFilter, SessionNode, SessionState,
    };
    use y_core::types::Role;

    // -----------------------------------------------------------------------
    // Mock stores (matching the actual trait signatures)
    // -----------------------------------------------------------------------

    struct MockSessionStore;
    struct MockTranscriptStore;

    #[async_trait::async_trait]
    impl SessionStore for MockSessionStore {
        async fn create(
            &self,
            _options: CreateSessionOptions,
        ) -> Result<SessionNode, SessionError> {
            Err(SessionError::Other {
                message: "mock".into(),
            })
        }
        async fn get(&self, _id: &SessionId) -> Result<SessionNode, SessionError> {
            Err(SessionError::NotFound { id: "mock".into() })
        }
        async fn list(&self, _filter: &SessionFilter) -> Result<Vec<SessionNode>, SessionError> {
            Ok(vec![])
        }
        async fn set_state(
            &self,
            _id: &SessionId,
            _state: SessionState,
        ) -> Result<(), SessionError> {
            Ok(())
        }
        async fn update_metadata(
            &self,
            _id: &SessionId,
            _title: Option<String>,
            _token_count: u32,
            _message_count: u32,
        ) -> Result<(), SessionError> {
            Ok(())
        }
        async fn children(&self, _id: &SessionId) -> Result<Vec<SessionNode>, SessionError> {
            Ok(vec![])
        }
        async fn ancestors(&self, _id: &SessionId) -> Result<Vec<SessionNode>, SessionError> {
            Ok(vec![])
        }
        async fn set_title(&self, _id: &SessionId, _title: String) -> Result<(), SessionError> {
            Ok(())
        }
        async fn set_manual_title(
            &self,
            _id: &SessionId,
            _title: Option<String>,
        ) -> Result<(), SessionError> {
            Ok(())
        }
        async fn delete(&self, _id: &SessionId) -> Result<(), SessionError> {
            Ok(())
        }
        async fn get_context_reset_index(
            &self,
            _id: &SessionId,
        ) -> Result<Option<u32>, SessionError> {
            Ok(None)
        }
        async fn set_context_reset_index(
            &self,
            _id: &SessionId,
            _index: Option<u32>,
        ) -> Result<(), SessionError> {
            Ok(())
        }
        async fn get_custom_system_prompt(
            &self,
            _id: &SessionId,
        ) -> Result<Option<String>, SessionError> {
            Ok(None)
        }
        async fn set_custom_system_prompt(
            &self,
            _id: &SessionId,
            _prompt: Option<String>,
        ) -> Result<(), SessionError> {
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl TranscriptStore for MockTranscriptStore {
        async fn append(
            &self,
            _session_id: &SessionId,
            _message: &Message,
        ) -> Result<(), SessionError> {
            Ok(())
        }
        async fn read_all(&self, _session_id: &SessionId) -> Result<Vec<Message>, SessionError> {
            Ok(vec![])
        }
        async fn read_last(
            &self,
            _session_id: &SessionId,
            _count: usize,
        ) -> Result<Vec<Message>, SessionError> {
            Ok(vec![])
        }
        async fn message_count(&self, _session_id: &SessionId) -> Result<usize, SessionError> {
            Ok(0)
        }
        async fn truncate(
            &self,
            _session_id: &SessionId,
            _keep_count: usize,
        ) -> Result<usize, SessionError> {
            Ok(0)
        }
    }

    fn make_manager() -> CanonicalSessionManager {
        CanonicalSessionManager::new(
            Arc::new(MockSessionStore),
            Arc::new(MockTranscriptStore),
            CanonicalConfig::default(),
        )
    }

    fn make_message(content: &str, role: Role) -> Message {
        Message {
            message_id: y_core::types::generate_message_id(),
            role,
            content: content.into(),
            timestamp: chrono::Utc::now(),
            tool_calls: vec![],
            tool_call_id: None,
            metadata: serde_json::json!({}),
        }
    }

    // -----------------------------------------------------------------------
    // Tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_register_channel() {
        let mgr = make_manager();
        let canonical_id = SessionId::new();
        let child_id = SessionId::new();

        let result = mgr
            .register_channel(&canonical_id, Channel::Cli, child_id)
            .await;
        assert!(result.is_ok());

        let channels = mgr.list_channels(&canonical_id).await.unwrap();
        assert_eq!(channels.len(), 1);
    }

    #[tokio::test]
    async fn test_register_multiple_channels() {
        let mgr = make_manager();
        let canonical_id = SessionId::new();

        mgr.register_channel(&canonical_id, Channel::Cli, SessionId::new())
            .await
            .unwrap();
        mgr.register_channel(&canonical_id, Channel::Api, SessionId::new())
            .await
            .unwrap();
        mgr.register_channel(&canonical_id, Channel::Web, SessionId::new())
            .await
            .unwrap();

        let channels = mgr.list_channels(&canonical_id).await.unwrap();
        assert_eq!(channels.len(), 3);
    }

    #[tokio::test]
    async fn test_max_channels_enforced() {
        let config = CanonicalConfig {
            max_channels: 2,
            auto_merge: true,
        };
        let mgr = CanonicalSessionManager::new(
            Arc::new(MockSessionStore),
            Arc::new(MockTranscriptStore),
            config,
        );

        let canonical_id = SessionId::new();
        mgr.register_channel(&canonical_id, Channel::Cli, SessionId::new())
            .await
            .unwrap();
        mgr.register_channel(&canonical_id, Channel::Api, SessionId::new())
            .await
            .unwrap();

        // Third channel should fail.
        let result = mgr
            .register_channel(&canonical_id, Channel::Web, SessionId::new())
            .await;
        assert!(matches!(
            result,
            Err(CanonicalError::TooManyChannels { .. })
        ));
    }

    #[tokio::test]
    async fn test_append_and_read_transcript() {
        let mgr = make_manager();
        let canonical_id = SessionId::new();

        // Register a channel first.
        mgr.register_channel(&canonical_id, Channel::Cli, SessionId::new())
            .await
            .unwrap();

        // Append messages from different channels.
        mgr.append_message(
            &canonical_id,
            Channel::Cli,
            make_message("hello from CLI", Role::User),
        )
        .await
        .unwrap();

        mgr.register_channel(&canonical_id, Channel::Api, SessionId::new())
            .await
            .unwrap();

        mgr.append_message(
            &canonical_id,
            Channel::Api,
            make_message("hello from API", Role::User),
        )
        .await
        .unwrap();

        let transcript = mgr.canonical_transcript(&canonical_id).await.unwrap();
        assert_eq!(transcript.len(), 2);
        assert_eq!(transcript[0].sequence, 1);
        assert_eq!(transcript[1].sequence, 2);
    }

    #[tokio::test]
    async fn test_append_to_nonexistent_fails() {
        let mgr = make_manager();
        let result = mgr
            .append_message(
                &SessionId::new(),
                Channel::Cli,
                make_message("test", Role::User),
            )
            .await;
        assert!(matches!(result, Err(CanonicalError::NotFound { .. })));
    }

    #[tokio::test]
    async fn test_channel_session_lookup() {
        let mgr = make_manager();
        let canonical_id = SessionId::new();
        let child_id = SessionId::new();

        mgr.register_channel(&canonical_id, Channel::Cli, child_id.clone())
            .await
            .unwrap();

        let found = mgr
            .channel_session(&canonical_id, &Channel::Cli)
            .await
            .unwrap();
        assert_eq!(found, child_id);
    }

    #[tokio::test]
    async fn test_channel_session_not_found() {
        let mgr = make_manager();
        let canonical_id = SessionId::new();

        mgr.register_channel(&canonical_id, Channel::Cli, SessionId::new())
            .await
            .unwrap();

        let result = mgr.channel_session(&canonical_id, &Channel::Api).await;
        assert!(matches!(
            result,
            Err(CanonicalError::ChannelNotFound { .. })
        ));
    }

    #[tokio::test]
    async fn test_messages_by_role() {
        let mgr = make_manager();
        let canonical_id = SessionId::new();

        mgr.register_channel(&canonical_id, Channel::Cli, SessionId::new())
            .await
            .unwrap();

        mgr.append_message(
            &canonical_id,
            Channel::Cli,
            make_message("user msg", Role::User),
        )
        .await
        .unwrap();

        mgr.append_message(
            &canonical_id,
            Channel::Cli,
            make_message("assistant msg", Role::Assistant),
        )
        .await
        .unwrap();

        let user_msgs = mgr
            .messages_by_role(&canonical_id, &Role::User)
            .await
            .unwrap();
        assert_eq!(user_msgs.len(), 1);

        let assistant_msgs = mgr
            .messages_by_role(&canonical_id, &Role::Assistant)
            .await
            .unwrap();
        assert_eq!(assistant_msgs.len(), 1);
    }

    #[tokio::test]
    async fn test_message_count() {
        let mgr = make_manager();
        let canonical_id = SessionId::new();

        mgr.register_channel(&canonical_id, Channel::Cli, SessionId::new())
            .await
            .unwrap();

        assert_eq!(mgr.message_count(&canonical_id).await.unwrap(), 0);

        mgr.append_message(
            &canonical_id,
            Channel::Cli,
            make_message("msg1", Role::User),
        )
        .await
        .unwrap();
        mgr.append_message(
            &canonical_id,
            Channel::Cli,
            make_message("msg2", Role::Assistant),
        )
        .await
        .unwrap();

        assert_eq!(mgr.message_count(&canonical_id).await.unwrap(), 2);
    }
}
