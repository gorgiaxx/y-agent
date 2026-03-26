//! Mock storage implementations for `CheckpointStorage`, `SessionStore`, and
//! `TranscriptStore`.

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::RwLock;

use y_core::checkpoint::{CheckpointError, CheckpointStorage, WorkflowCheckpoint};
use y_core::session::{
    CreateSessionOptions, SessionError, SessionFilter, SessionNode, SessionState, SessionStore,
    TranscriptStore,
};
use y_core::types::{Message, SessionId, WorkflowId};

// ---------------------------------------------------------------------------
// MockCheckpointStorage
// ---------------------------------------------------------------------------

/// In-memory checkpoint storage for tests.
#[derive(Debug, Default)]
pub struct MockCheckpointStorage {
    data: RwLock<HashMap<String, WorkflowCheckpoint>>,
}

impl MockCheckpointStorage {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl CheckpointStorage for MockCheckpointStorage {
    async fn write_pending(
        &self,
        workflow_id: &WorkflowId,
        session_id: &SessionId,
        step_number: u64,
        state: &serde_json::Value,
    ) -> Result<(), CheckpointError> {
        let mut map = self.data.write().unwrap();
        let key = workflow_id.to_string();
        let cp = map.entry(key).or_insert_with(|| WorkflowCheckpoint {
            workflow_id: workflow_id.clone(),
            session_id: session_id.clone(),
            step_number: 0,
            status: y_core::checkpoint::CheckpointStatus::Running,
            committed_state: serde_json::Value::Null,
            pending_state: None,
            interrupt_data: None,
            versions_seen: serde_json::Value::Object(serde_json::Map::new()),
            created_at: y_core::types::now(),
            updated_at: y_core::types::now(),
        });
        cp.step_number = step_number;
        cp.pending_state = Some(state.clone());
        cp.updated_at = y_core::types::now();
        Ok(())
    }

    async fn commit(
        &self,
        workflow_id: &WorkflowId,
        _step_number: u64,
    ) -> Result<(), CheckpointError> {
        let mut map = self.data.write().unwrap();
        let key = workflow_id.to_string();
        let cp = map.get_mut(&key).ok_or(CheckpointError::NotFound {
            workflow_id: key.clone(),
        })?;
        if let Some(pending) = cp.pending_state.take() {
            cp.committed_state = pending;
        }
        cp.updated_at = y_core::types::now();
        Ok(())
    }

    async fn read_committed(
        &self,
        workflow_id: &WorkflowId,
    ) -> Result<Option<WorkflowCheckpoint>, CheckpointError> {
        let map = self.data.read().unwrap();
        Ok(map.get(&workflow_id.to_string()).cloned())
    }

    async fn set_interrupted(
        &self,
        workflow_id: &WorkflowId,
        interrupt_data: serde_json::Value,
    ) -> Result<(), CheckpointError> {
        let mut map = self.data.write().unwrap();
        let key = workflow_id.to_string();
        let cp = map.get_mut(&key).ok_or(CheckpointError::NotFound {
            workflow_id: key.clone(),
        })?;
        cp.status = y_core::checkpoint::CheckpointStatus::Interrupted;
        cp.interrupt_data = Some(interrupt_data);
        Ok(())
    }

    async fn set_completed(&self, workflow_id: &WorkflowId) -> Result<(), CheckpointError> {
        let mut map = self.data.write().unwrap();
        let key = workflow_id.to_string();
        let cp = map.get_mut(&key).ok_or(CheckpointError::NotFound {
            workflow_id: key.clone(),
        })?;
        cp.status = y_core::checkpoint::CheckpointStatus::Completed;
        Ok(())
    }

    async fn set_failed(
        &self,
        workflow_id: &WorkflowId,
        _error: &str,
    ) -> Result<(), CheckpointError> {
        let mut map = self.data.write().unwrap();
        let key = workflow_id.to_string();
        let cp = map.get_mut(&key).ok_or(CheckpointError::NotFound {
            workflow_id: key.clone(),
        })?;
        cp.status = y_core::checkpoint::CheckpointStatus::Failed;
        Ok(())
    }

    async fn prune(
        &self,
        _workflow_id: &WorkflowId,
        _keep_after_step: u64,
    ) -> Result<u64, CheckpointError> {
        Ok(0) // no-op for mock
    }
}

// ---------------------------------------------------------------------------
// MockSessionStore
// ---------------------------------------------------------------------------

/// In-memory session store for tests.
#[derive(Debug, Default)]
pub struct MockSessionStore {
    sessions: RwLock<HashMap<String, SessionNode>>,
}

impl MockSessionStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl SessionStore for MockSessionStore {
    async fn create(&self, options: CreateSessionOptions) -> Result<SessionNode, SessionError> {
        let id = SessionId::new();
        let root_id = options.parent_id.as_ref().map_or_else(
            || id.clone(),
            |pid| {
                let map = self.sessions.read().unwrap();
                map.get(&pid.to_string())
                    .map_or_else(|| pid.clone(), |p| p.root_id.clone())
            },
        );

        let node = SessionNode {
            id: id.clone(),
            parent_id: options.parent_id,
            root_id,
            depth: 0,
            path: vec![id.clone()],
            session_type: options.session_type,
            state: SessionState::Active,
            agent_id: options.agent_id,
            title: options.title,
            channel: None,
            label: None,
            token_count: 0,
            message_count: 0,
            last_compaction: None,
            compaction_count: 0,
            created_at: y_core::types::now(),
            updated_at: y_core::types::now(),
        };

        self.sessions
            .write()
            .unwrap()
            .insert(id.to_string(), node.clone());
        Ok(node)
    }

    async fn get(&self, id: &SessionId) -> Result<SessionNode, SessionError> {
        let map = self.sessions.read().unwrap();
        map.get(&id.to_string())
            .cloned()
            .ok_or(SessionError::NotFound { id: id.to_string() })
    }

    async fn list(&self, filter: &SessionFilter) -> Result<Vec<SessionNode>, SessionError> {
        let map = self.sessions.read().unwrap();
        let results: Vec<SessionNode> = map
            .values()
            .filter(|s| filter.state.as_ref().is_none_or(|st| s.state == *st))
            .filter(|s| {
                filter
                    .session_type
                    .as_ref()
                    .is_none_or(|t| s.session_type == *t)
            })
            .cloned()
            .collect();
        Ok(results)
    }

    async fn set_state(&self, id: &SessionId, state: SessionState) -> Result<(), SessionError> {
        let mut map = self.sessions.write().unwrap();
        let node = map
            .get_mut(&id.to_string())
            .ok_or(SessionError::NotFound { id: id.to_string() })?;
        node.state = state;
        Ok(())
    }

    async fn update_metadata(
        &self,
        id: &SessionId,
        title: Option<String>,
        token_count: u32,
        message_count: u32,
    ) -> Result<(), SessionError> {
        let mut map = self.sessions.write().unwrap();
        let node = map
            .get_mut(&id.to_string())
            .ok_or(SessionError::NotFound { id: id.to_string() })?;
        if let Some(t) = title {
            node.title = Some(t);
        }
        node.token_count = token_count;
        node.message_count = message_count;
        Ok(())
    }

    async fn children(&self, id: &SessionId) -> Result<Vec<SessionNode>, SessionError> {
        let map = self.sessions.read().unwrap();
        let id_str = id.to_string();
        Ok(map
            .values()
            .filter(|s| s.parent_id.as_ref().map(ToString::to_string) == Some(id_str.clone()))
            .cloned()
            .collect())
    }

    async fn ancestors(&self, id: &SessionId) -> Result<Vec<SessionNode>, SessionError> {
        let map = self.sessions.read().unwrap();
        let mut result = vec![];
        let mut current_id = Some(id.clone());
        while let Some(cid) = current_id.take() {
            if let Some(node) = map.get(&cid.to_string()) {
                result.push(node.clone());
                current_id.clone_from(&node.parent_id);
            } else {
                break;
            }
        }
        result.reverse();
        Ok(result)
    }

    async fn set_title(&self, id: &SessionId, title: String) -> Result<(), SessionError> {
        let mut map = self.sessions.write().unwrap();
        let node = map
            .get_mut(&id.to_string())
            .ok_or(SessionError::NotFound { id: id.to_string() })?;
        node.title = Some(title);
        Ok(())
    }

    async fn delete(&self, id: &SessionId) -> Result<(), SessionError> {
        let mut map = self.sessions.write().unwrap();
        if map.remove(&id.to_string()).is_none() {
            return Err(SessionError::NotFound { id: id.to_string() });
        }
        Ok(())
    }

    async fn get_context_reset_index(&self, _id: &SessionId) -> Result<Option<u32>, SessionError> {
        Ok(None)
    }

    async fn set_context_reset_index(
        &self,
        _id: &SessionId,
        _index: Option<u32>,
    ) -> Result<(), SessionError> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// MockTranscriptStore
// ---------------------------------------------------------------------------

/// In-memory transcript store for tests.
#[derive(Debug, Default)]
pub struct MockTranscriptStore {
    transcripts: RwLock<HashMap<String, Vec<Message>>>,
}

impl MockTranscriptStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl TranscriptStore for MockTranscriptStore {
    async fn append(&self, session_id: &SessionId, message: &Message) -> Result<(), SessionError> {
        let mut map = self.transcripts.write().unwrap();
        map.entry(session_id.to_string())
            .or_default()
            .push(message.clone());
        Ok(())
    }

    async fn read_all(&self, session_id: &SessionId) -> Result<Vec<Message>, SessionError> {
        let map = self.transcripts.read().unwrap();
        Ok(map
            .get(&session_id.to_string())
            .cloned()
            .unwrap_or_default())
    }

    async fn read_last(
        &self,
        session_id: &SessionId,
        count: usize,
    ) -> Result<Vec<Message>, SessionError> {
        let map = self.transcripts.read().unwrap();
        let msgs = map
            .get(&session_id.to_string())
            .cloned()
            .unwrap_or_default();
        Ok(msgs.into_iter().rev().take(count).rev().collect())
    }

    async fn message_count(&self, session_id: &SessionId) -> Result<usize, SessionError> {
        let map = self.transcripts.read().unwrap();
        Ok(map
            .get(&session_id.to_string())
            .map_or(0, std::vec::Vec::len))
    }

    async fn truncate(
        &self,
        session_id: &SessionId,
        keep_count: usize,
    ) -> Result<usize, SessionError> {
        let mut map = self.transcripts.write().unwrap();
        let msgs = map.entry(session_id.to_string()).or_default();
        if keep_count >= msgs.len() {
            return Ok(0);
        }
        let removed = msgs.len() - keep_count;
        msgs.truncate(keep_count);
        Ok(removed)
    }
}

// ---------------------------------------------------------------------------
// MockDisplayTranscriptStore
// ---------------------------------------------------------------------------

/// In-memory display transcript store for tests.
#[derive(Debug, Default)]
pub struct MockDisplayTranscriptStore {
    transcripts: RwLock<HashMap<String, Vec<Message>>>,
}

impl MockDisplayTranscriptStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl y_core::session::DisplayTranscriptStore for MockDisplayTranscriptStore {
    async fn append(&self, session_id: &SessionId, message: &Message) -> Result<(), SessionError> {
        let mut map = self.transcripts.write().unwrap();
        map.entry(session_id.to_string())
            .or_default()
            .push(message.clone());
        Ok(())
    }

    async fn read_all(&self, session_id: &SessionId) -> Result<Vec<Message>, SessionError> {
        let map = self.transcripts.read().unwrap();
        Ok(map
            .get(&session_id.to_string())
            .cloned()
            .unwrap_or_default())
    }

    async fn message_count(&self, session_id: &SessionId) -> Result<usize, SessionError> {
        let map = self.transcripts.read().unwrap();
        Ok(map
            .get(&session_id.to_string())
            .map_or(0, std::vec::Vec::len))
    }

    async fn truncate(
        &self,
        session_id: &SessionId,
        keep_count: usize,
    ) -> Result<usize, SessionError> {
        let mut map = self.transcripts.write().unwrap();
        let msgs = map.entry(session_id.to_string()).or_default();
        if keep_count >= msgs.len() {
            return Ok(0);
        }
        let removed = msgs.len() - keep_count;
        msgs.truncate(keep_count);
        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use y_core::session::SessionType;

    #[tokio::test]
    async fn test_checkpoint_write_commit_read() {
        let store = MockCheckpointStorage::new();
        let wid = WorkflowId::new();
        let sid = SessionId::new();
        let state = serde_json::json!({"step": 1});

        store.write_pending(&wid, &sid, 1, &state).await.unwrap();
        store.commit(&wid, 1).await.unwrap();

        let cp = store.read_committed(&wid).await.unwrap().unwrap();
        assert_eq!(cp.committed_state, state);
    }

    #[tokio::test]
    async fn test_session_create_and_get() {
        let store = MockSessionStore::new();
        let node = store
            .create(CreateSessionOptions {
                parent_id: None,
                session_type: SessionType::Main,
                agent_id: None,
                title: Some("test session".into()),
            })
            .await
            .unwrap();

        let fetched = store.get(&node.id).await.unwrap();
        assert_eq!(fetched.title, Some("test session".into()));
    }

    #[tokio::test]
    async fn test_transcript_append_and_read() {
        let store = MockTranscriptStore::new();
        let sid = SessionId::new();
        let msg = crate::fixtures::make_user_message("hello");

        store.append(&sid, &msg).await.unwrap();
        store.append(&sid, &msg).await.unwrap();

        let all = store.read_all(&sid).await.unwrap();
        assert_eq!(all.len(), 2);

        let last = store.read_last(&sid, 1).await.unwrap();
        assert_eq!(last.len(), 1);

        let count = store.message_count(&sid).await.unwrap();
        assert_eq!(count, 2);
    }
}
