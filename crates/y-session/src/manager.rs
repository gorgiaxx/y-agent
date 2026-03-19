//! `SessionManager` — high-level facade for session lifecycle management.

use std::sync::Arc;
use std::sync::RwLock;

use tracing::instrument;

use y_core::agent::{AgentDelegator, ContextStrategyHint};
use y_core::session::{
    CreateSessionOptions, DisplayTranscriptStore, SessionFilter, SessionNode, SessionState,
    SessionStore, TranscriptStore,
};
use y_core::types::{Message, SessionId};

use crate::config::SessionConfig;
use crate::error::SessionManagerError;
use crate::state_machine::StateMachine;

/// High-level session management facade.
///
/// Combines state machine validation, session store, and transcript store
/// into a unified API. All state transitions go through the state machine
/// to ensure validity.
pub struct SessionManager {
    session_store: Arc<dyn SessionStore>,
    transcript_store: Arc<dyn TranscriptStore>,
    display_transcript_store: Arc<dyn DisplayTranscriptStore>,
    config: RwLock<SessionConfig>,
}

impl SessionManager {
    /// Create a new session manager.
    pub fn new(
        session_store: Arc<dyn SessionStore>,
        transcript_store: Arc<dyn TranscriptStore>,
        display_transcript_store: Arc<dyn DisplayTranscriptStore>,
        config: SessionConfig,
    ) -> Self {
        Self {
            session_store,
            transcript_store,
            display_transcript_store,
            config: RwLock::new(config),
        }
    }

    /// Hot-reload the session configuration.
    pub fn reload_config(&self, new_config: SessionConfig) {
        let mut guard = self.config.write().unwrap();
        *guard = new_config;
        tracing::info!("Session config hot-reloaded");
    }

    /// Create a new root session.
    #[instrument(skip(self))]
    pub async fn create_session(
        &self,
        options: CreateSessionOptions,
    ) -> Result<SessionNode, SessionManagerError> {
        // Validate depth if creating a child.
        if let Some(ref parent_id) = options.parent_id {
            let parent = self.session_store.get(parent_id).await?;
            let max_depth = self.config.read().unwrap().max_depth;
            if parent.depth >= max_depth {
                return Err(SessionManagerError::Config {
                    message: format!("maximum tree depth {max_depth} exceeded"),
                });
            }
        }

        let node = self.session_store.create(options).await?;
        Ok(node)
    }

    /// Get a session by ID.
    #[instrument(skip(self), fields(session_id = %id))]
    pub async fn get_session(&self, id: &SessionId) -> Result<SessionNode, SessionManagerError> {
        Ok(self.session_store.get(id).await?)
    }

    /// List sessions matching filters.
    pub async fn list_sessions(
        &self,
        filter: &SessionFilter,
    ) -> Result<Vec<SessionNode>, SessionManagerError> {
        Ok(self.session_store.list(filter).await?)
    }

    /// Transition a session's state, validating the transition.
    #[instrument(skip(self), fields(session_id = %id, new_state = ?new_state))]
    pub async fn transition_state(
        &self,
        id: &SessionId,
        new_state: SessionState,
    ) -> Result<(), SessionManagerError> {
        let current = self.session_store.get(id).await?;
        StateMachine::validate_transition(&current.state, &new_state)?;
        self.session_store.set_state(id, new_state).await?;
        Ok(())
    }

    /// Append a message to a session's transcript.
    ///
    /// Dual-writes to both the display transcript (GUI-facing) and the
    /// context transcript (LLM-facing). Display store is written first.
    #[instrument(skip(self, message), fields(session_id = %session_id))]
    pub async fn append_message(
        &self,
        session_id: &SessionId,
        message: &Message,
    ) -> Result<(), SessionManagerError> {
        // Verify session exists and is active.
        let session = self.session_store.get(session_id).await?;
        if session.state != SessionState::Active {
            return Err(SessionManagerError::InvalidTransition {
                from: format!("{:?}", session.state),
                to: "append message (requires Active)".into(),
            });
        }

        // Write to display transcript first (append-only, GUI source of truth).
        self.display_transcript_store
            .append(session_id, message)
            .await
            .map_err(|e| SessionManagerError::Transcript {
                message: format!("display transcript: {e}"),
            })?;

        // Write to context transcript (LLM source of truth).
        self.transcript_store
            .append(session_id, message)
            .await
            .map_err(|e| SessionManagerError::Transcript {
                message: e.to_string(),
            })?;

        // Update session metadata counters.
        let count = self
            .transcript_store
            .message_count(session_id)
            .await
            .map_err(|e| SessionManagerError::Transcript {
                message: e.to_string(),
            })?;

        self.session_store
            .update_metadata(session_id, None, session.token_count, count as u32)
            .await?;

        Ok(())
    }

    /// Read all messages from the context transcript (for LLM context assembly).
    pub async fn read_transcript(
        &self,
        session_id: &SessionId,
    ) -> Result<Vec<Message>, SessionManagerError> {
        self.transcript_store
            .read_all(session_id)
            .await
            .map_err(|e| SessionManagerError::Transcript {
                message: e.to_string(),
            })
    }

    /// Read all messages from the display transcript (for GUI display).
    ///
    /// The display transcript is never compacted, so this always returns
    /// the full conversation history as seen by the user.
    pub async fn read_display_transcript(
        &self,
        session_id: &SessionId,
    ) -> Result<Vec<Message>, SessionManagerError> {
        self.display_transcript_store
            .read_all(session_id)
            .await
            .map_err(|e| SessionManagerError::Transcript {
                message: format!("display transcript: {e}"),
            })
    }

    /// Read the last N messages from a session's transcript.
    pub async fn read_last_messages(
        &self,
        session_id: &SessionId,
        count: usize,
    ) -> Result<Vec<Message>, SessionManagerError> {
        self.transcript_store
            .read_last(session_id, count)
            .await
            .map_err(|e| SessionManagerError::Transcript {
                message: e.to_string(),
            })
    }

    /// Create a branch session from an existing one.
    ///
    /// The branch is a new child of the specified session with type Branch.
    #[instrument(skip(self), fields(parent_id = %parent_id))]
    pub async fn branch(
        &self,
        parent_id: &SessionId,
        title: Option<String>,
    ) -> Result<SessionNode, SessionManagerError> {
        let parent = self.session_store.get(parent_id).await?;
        let max_depth = self.config.read().unwrap().max_depth;
        if parent.depth >= max_depth {
            return Err(SessionManagerError::Config {
                message: format!("maximum tree depth {max_depth} exceeded"),
            });
        }

        let branch = self
            .session_store
            .create(CreateSessionOptions {
                parent_id: Some(parent_id.clone()),
                session_type: y_core::session::SessionType::Branch,
                agent_id: parent.agent_id.clone(),
                title,
            })
            .await?;

        Ok(branch)
    }

    /// Get children of a session.
    pub async fn children(
        &self,
        session_id: &SessionId,
    ) -> Result<Vec<SessionNode>, SessionManagerError> {
        Ok(self.session_store.children(session_id).await?)
    }

    /// Get ancestors of a session (path from root to parent).
    pub async fn ancestors(
        &self,
        session_id: &SessionId,
    ) -> Result<Vec<SessionNode>, SessionManagerError> {
        Ok(self.session_store.ancestors(session_id).await?)
    }

    /// Update only the session title.
    #[instrument(skip(self), fields(session_id = %id))]
    pub async fn update_title(
        &self,
        id: &SessionId,
        title: String,
    ) -> Result<(), SessionManagerError> {
        self.session_store.set_title(id, title).await?;
        Ok(())
    }

    /// Hard-delete a session: removes the metadata row, clears both transcripts.
    ///
    /// This is irreversible. Any in-progress runs for this session should have
    /// completed or been cancelled before calling this.
    #[instrument(skip(self), fields(session_id = %id))]
    pub async fn delete_session(&self, id: &SessionId) -> Result<(), SessionManagerError> {
        // Remove message transcripts first (best-effort; continue if they fail).
        let _ = self.display_transcript_store.truncate(id, 0).await;
        let _ = self.transcript_store.truncate(id, 0).await;
        // Hard-delete the session metadata row.
        self.session_store.delete(id).await?;
        Ok(())
    }

    /// Generate a session title by summarizing the conversation via agent delegation.
    ///
    /// Delegates to the `title-generator` built-in agent defined in
    /// `config/agents/title-generator.toml`. The agent's system prompt,
    /// model preferences, and temperature are managed externally.
    #[instrument(skip(self, delegator, messages), fields(session_id = %session_id))]
    pub async fn generate_title(
        &self,
        delegator: &dyn AgentDelegator,
        session_id: &SessionId,
        messages: &[Message],
    ) -> Result<String, SessionManagerError> {
        // Take at most the last 6 messages to keep the input compact.
        let context: Vec<_> = messages
            .iter()
            .rev()
            .take(6)
            .rev()
            .map(|m| serde_json::json!({ "role": format!("{:?}", m.role), "content": m.content }))
            .collect();

        let input = serde_json::json!({ "messages": context });

        let output = delegator
            .delegate("title-generator", input, ContextStrategyHint::None)
            .await
            .map_err(|e| SessionManagerError::Other {
                message: format!("title generation delegation failed: {e}"),
            })?;

        let title = output
            .text
            .trim()
            .trim_matches('"')
            .trim_matches('\'')
            .to_string();

        if title.is_empty() {
            return Err(SessionManagerError::Other {
                message: "title-generator agent returned empty title".into(),
            });
        }

        // Persist the generated title.
        self.update_title(session_id, title.clone()).await?;

        Ok(title)
    }

    /// Check if a session should trigger compaction.
    pub fn should_compact(&self, session: &SessionNode) -> bool {
        session.token_count >= self.config.read().unwrap().compaction_threshold
    }

    /// Get a reference to the underlying context transcript store.
    pub fn transcript_store(&self) -> &dyn TranscriptStore {
        &*self.transcript_store
    }

    /// Get a reference to the underlying display transcript store.
    pub fn display_transcript_store(&self) -> &dyn DisplayTranscriptStore {
        &*self.display_transcript_store
    }

    /// Get a snapshot of the session configuration.
    pub fn config(&self) -> SessionConfig {
        self.config.read().unwrap().clone()
    }
}

impl std::fmt::Debug for SessionManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionManager")
            .field("config", &*self.config.read().unwrap())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use y_core::session::SessionType;
    use y_core::types::Role;

    async fn setup() -> SessionManager {
        let config = y_storage::StorageConfig::in_memory();
        let pool = y_storage::create_pool(&config).await.unwrap();
        y_storage::migration::run_embedded_migrations(&pool)
            .await
            .unwrap();

        let session_store = Arc::new(y_storage::SqliteSessionStore::new(pool));
        let transcript_dir = tempfile::tempdir().unwrap();
        let transcript_path = transcript_dir.path().to_path_buf();
        let transcript_store = Arc::new(y_storage::JsonlTranscriptStore::new(&transcript_path));
        let display_transcript_store = Arc::new(y_storage::JsonlDisplayTranscriptStore::new(
            &transcript_path,
        ));

        SessionManager::new(
            session_store,
            transcript_store,
            display_transcript_store,
            SessionConfig::default(),
        )
    }

    fn test_msg(content: &str) -> Message {
        Message {
            message_id: y_core::types::generate_message_id(),
            role: Role::User,
            content: content.into(),
            tool_call_id: None,
            tool_calls: vec![],
            timestamp: chrono::Utc::now(),
            metadata: serde_json::Value::Null,
        }
    }

    #[tokio::test]
    async fn test_manager_create_session() {
        let mgr = setup().await;
        let session = mgr
            .create_session(CreateSessionOptions {
                parent_id: None,
                session_type: SessionType::Main,
                agent_id: None,
                title: Some("Test".into()),
            })
            .await
            .unwrap();

        assert_eq!(session.session_type, SessionType::Main);
        assert_eq!(session.state, SessionState::Active);
    }

    #[tokio::test]
    async fn test_manager_transition_active_to_paused() {
        let mgr = setup().await;
        let session = mgr
            .create_session(CreateSessionOptions {
                parent_id: None,
                session_type: SessionType::Main,
                agent_id: None,
                title: None,
            })
            .await
            .unwrap();

        mgr.transition_state(&session.id, SessionState::Paused)
            .await
            .unwrap();

        let updated = mgr.get_session(&session.id).await.unwrap();
        assert_eq!(updated.state, SessionState::Paused);
    }

    #[tokio::test]
    async fn test_manager_invalid_transition_archived_to_active() {
        let mgr = setup().await;
        let session = mgr
            .create_session(CreateSessionOptions {
                parent_id: None,
                session_type: SessionType::Main,
                agent_id: None,
                title: None,
            })
            .await
            .unwrap();

        mgr.transition_state(&session.id, SessionState::Archived)
            .await
            .unwrap();

        let result = mgr
            .transition_state(&session.id, SessionState::Active)
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_manager_append_and_read_messages() {
        let mgr = setup().await;
        let session = mgr
            .create_session(CreateSessionOptions {
                parent_id: None,
                session_type: SessionType::Main,
                agent_id: None,
                title: None,
            })
            .await
            .unwrap();

        mgr.append_message(&session.id, &test_msg("hello"))
            .await
            .unwrap();
        mgr.append_message(&session.id, &test_msg("world"))
            .await
            .unwrap();

        let transcript = mgr.read_transcript(&session.id).await.unwrap();
        assert_eq!(transcript.len(), 2);
        assert_eq!(transcript[0].content, "hello");
        assert_eq!(transcript[1].content, "world");
    }

    #[tokio::test]
    async fn test_manager_append_to_paused_session_fails() {
        let mgr = setup().await;
        let session = mgr
            .create_session(CreateSessionOptions {
                parent_id: None,
                session_type: SessionType::Main,
                agent_id: None,
                title: None,
            })
            .await
            .unwrap();

        mgr.transition_state(&session.id, SessionState::Paused)
            .await
            .unwrap();

        let result = mgr.append_message(&session.id, &test_msg("fail")).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_manager_branch() {
        let mgr = setup().await;
        let main = mgr
            .create_session(CreateSessionOptions {
                parent_id: None,
                session_type: SessionType::Main,
                agent_id: None,
                title: None,
            })
            .await
            .unwrap();

        let branch = mgr.branch(&main.id, Some("Branch 1".into())).await.unwrap();

        assert_eq!(branch.session_type, SessionType::Branch);
        assert_eq!(branch.parent_id, Some(main.id.clone()));
        assert_eq!(branch.depth, 1);
    }

    #[tokio::test]
    async fn test_manager_max_depth_enforced() {
        let td = tempfile::tempdir().unwrap();
        let tp = td.path().to_path_buf();
        let mgr = SessionManager::new(
            {
                let config = y_storage::StorageConfig::in_memory();
                let pool = y_storage::create_pool(&config).await.unwrap();
                y_storage::migration::run_embedded_migrations(&pool)
                    .await
                    .unwrap();
                Arc::new(y_storage::SqliteSessionStore::new(pool))
            },
            Arc::new(y_storage::JsonlTranscriptStore::new(&tp)),
            Arc::new(y_storage::JsonlDisplayTranscriptStore::new(&tp)),
            SessionConfig {
                max_depth: 2,
                ..Default::default()
            },
        );

        let root = mgr
            .create_session(CreateSessionOptions {
                parent_id: None,
                session_type: SessionType::Main,
                agent_id: None,
                title: None,
            })
            .await
            .unwrap();

        let child = mgr
            .create_session(CreateSessionOptions {
                parent_id: Some(root.id.clone()),
                session_type: SessionType::Child,
                agent_id: None,
                title: None,
            })
            .await
            .unwrap();

        let grandchild = mgr
            .create_session(CreateSessionOptions {
                parent_id: Some(child.id.clone()),
                session_type: SessionType::Child,
                agent_id: None,
                title: None,
            })
            .await
            .unwrap();

        // grandchild is at depth 2 (max_depth), so creating another child should fail.
        let result = mgr
            .create_session(CreateSessionOptions {
                parent_id: Some(grandchild.id.clone()),
                session_type: SessionType::Child,
                agent_id: None,
                title: None,
            })
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_manager_compaction_threshold() {
        let mgr = setup().await;
        let mut session = mgr
            .create_session(CreateSessionOptions {
                parent_id: None,
                session_type: SessionType::Main,
                agent_id: None,
                title: None,
            })
            .await
            .unwrap();

        assert!(!mgr.should_compact(&session));

        // Simulate high token count.
        session.token_count = 150_000;
        assert!(mgr.should_compact(&session));
    }
}
