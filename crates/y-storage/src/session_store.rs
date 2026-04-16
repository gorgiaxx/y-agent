//! SQLite-backed `SessionStore` implementation.

use async_trait::async_trait;
use sqlx::SqlitePool;
use tracing::instrument;

use std::fmt::Write;
use y_core::session::{
    CreateSessionOptions, SessionError, SessionFilter, SessionNode, SessionState, SessionStore,
    SessionType,
};
use y_core::types::{AgentId, SessionId};

/// SQLite-backed session metadata store.
///
/// Stores session tree structure in the `session_metadata` table.
/// Message transcripts are handled separately by `JsonlTranscriptStore`.
#[derive(Debug, Clone)]
pub struct SqliteSessionStore {
    pool: SqlitePool,
}

impl SqliteSessionStore {
    /// Create a new session store backed by the given pool.
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl SessionStore for SqliteSessionStore {
    #[instrument(skip(self), fields(session_type = ?options.session_type))]
    async fn create(&self, options: CreateSessionOptions) -> Result<SessionNode, SessionError> {
        let id = SessionId::new();
        let now_str = chrono::Utc::now()
            .format("%Y-%m-%dT%H:%M:%S%.3fZ")
            .to_string();

        // Determine root_id, depth, and path based on parent.
        let (root_id, depth, path_json) = if let Some(ref parent_id) = options.parent_id {
            let parent = self.get(parent_id).await?;
            let mut path = parent.path.clone();
            path.push(parent.id.clone());
            let path_strs: Vec<&str> = path.iter().map(y_core::types::SessionId::as_str).collect();
            let path_json =
                serde_json::to_string(&path_strs).map_err(|e| SessionError::StorageError {
                    message: format!("serialize path: {e}"),
                })?;
            (parent.root_id.clone(), parent.depth + 1, path_json)
        } else {
            // Root session: path is empty, root_id is self.
            (id.clone(), 0, "[]".to_string())
        };

        let session_type_str = session_type_to_str(&options.session_type);
        let transcript_path = format!("transcripts/{}.jsonl", id.as_str());

        sqlx::query(
            r"INSERT INTO session_metadata
              (id, parent_id, root_id, depth, path, session_type, state, agent_id, title, manual_title, token_count, message_count, transcript_path, created_at, updated_at)
              VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'active', ?7, ?8, NULL, 0, 0, ?9, ?10, ?10)",
        )
        .bind(id.as_str())
        .bind(options.parent_id.as_ref().map(SessionId::as_str))
        .bind(root_id.as_str())
        .bind(i64::from(depth))
        .bind(&path_json)
        .bind(session_type_str)
        .bind(options.agent_id.as_ref().map(AgentId::as_str))
        .bind(options.title.as_deref())
        .bind(&transcript_path)
        .bind(&now_str)
        .execute(&self.pool)
        .await
        .map_err(|e| SessionError::StorageError {
            message: e.to_string(),
        })?;

        self.get(&id).await
    }

    #[instrument(skip(self), fields(session_id = %id))]
    async fn get(&self, id: &SessionId) -> Result<SessionNode, SessionError> {
        let row: Option<SessionRow> = sqlx::query_as(
            r"SELECT id, parent_id, root_id, depth, path, session_type, state,
                     agent_id, title, manual_title, token_count, message_count, created_at, updated_at
              FROM session_metadata WHERE id = ?1",
        )
        .bind(id.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| SessionError::StorageError {
            message: e.to_string(),
        })?;

        match row {
            Some(r) => r.into_session_node(),
            None => Err(SessionError::NotFound { id: id.to_string() }),
        }
    }

    #[instrument(skip(self))]
    async fn list(&self, filter: &SessionFilter) -> Result<Vec<SessionNode>, SessionError> {
        let mut sql = String::from(
            r"SELECT id, parent_id, root_id, depth, path, session_type, state,
                     agent_id, title, manual_title, token_count, message_count, created_at, updated_at
              FROM session_metadata WHERE 1=1",
        );
        let mut binds: Vec<String> = Vec::new();

        if let Some(ref state) = filter.state {
            binds.push(state_to_str(state).to_string());
            write!(&mut sql, " AND state = ?{}", binds.len()).unwrap();
        }

        if let Some(ref session_type) = filter.session_type {
            binds.push(session_type_to_str(session_type).to_string());
            write!(&mut sql, " AND session_type = ?{}", binds.len()).unwrap();
        }

        if let Some(ref agent_id) = filter.agent_id {
            binds.push(agent_id.as_str().to_string());
            write!(&mut sql, " AND agent_id = ?{}", binds.len()).unwrap();
        }

        if let Some(ref root_id) = filter.root_id {
            binds.push(root_id.as_str().to_string());
            write!(&mut sql, " AND root_id = ?{}", binds.len()).unwrap();
        }

        sql.push_str(" ORDER BY created_at ASC");

        let mut query = sqlx::query_as::<_, SessionRow>(&sql);
        for b in &binds {
            query = query.bind(b);
        }

        let rows = query
            .fetch_all(&self.pool)
            .await
            .map_err(|e| SessionError::StorageError {
                message: e.to_string(),
            })?;

        rows.into_iter()
            .map(SessionRow::into_session_node)
            .collect()
    }

    #[instrument(skip(self), fields(session_id = %id, new_state = ?state))]
    async fn set_state(&self, id: &SessionId, state: SessionState) -> Result<(), SessionError> {
        let state_str = state_to_str(&state);

        let result = sqlx::query(
            r"UPDATE session_metadata
              SET state = ?1, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
              WHERE id = ?2",
        )
        .bind(state_str)
        .bind(id.as_str())
        .execute(&self.pool)
        .await
        .map_err(|e| SessionError::StorageError {
            message: e.to_string(),
        })?;

        if result.rows_affected() == 0 {
            return Err(SessionError::NotFound { id: id.to_string() });
        }

        Ok(())
    }

    #[instrument(skip(self), fields(session_id = %id))]
    async fn update_metadata(
        &self,
        id: &SessionId,
        title: Option<String>,
        token_count: u32,
        message_count: u32,
    ) -> Result<(), SessionError> {
        let result = sqlx::query(
            r"UPDATE session_metadata
              SET title = COALESCE(?1, title),
                  token_count = ?2,
                  message_count = ?3,
                  updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
              WHERE id = ?4",
        )
        .bind(title.as_deref())
        .bind(i64::from(token_count))
        .bind(i64::from(message_count))
        .bind(id.as_str())
        .execute(&self.pool)
        .await
        .map_err(|e| SessionError::StorageError {
            message: e.to_string(),
        })?;

        if result.rows_affected() == 0 {
            return Err(SessionError::NotFound { id: id.to_string() });
        }

        Ok(())
    }

    #[instrument(skip(self), fields(parent_id = %id))]
    async fn children(&self, id: &SessionId) -> Result<Vec<SessionNode>, SessionError> {
        let rows: Vec<SessionRow> = sqlx::query_as(
            r"SELECT id, parent_id, root_id, depth, path, session_type, state,
                     agent_id, title, manual_title, token_count, message_count, created_at, updated_at
              FROM session_metadata WHERE parent_id = ?1 ORDER BY created_at ASC",
        )
        .bind(id.as_str())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| SessionError::StorageError {
            message: e.to_string(),
        })?;

        rows.into_iter()
            .map(SessionRow::into_session_node)
            .collect()
    }

    #[instrument(skip(self), fields(session_id = %id))]
    async fn ancestors(&self, id: &SessionId) -> Result<Vec<SessionNode>, SessionError> {
        let node = self.get(id).await?;

        if node.path.is_empty() {
            return Ok(Vec::new());
        }

        // Batch query: fetch all ancestors in a single round-trip.
        let placeholders: Vec<String> = (1..=node.path.len()).map(|i| format!("?{i}")).collect();
        let sql = format!(
            r"SELECT id, parent_id, root_id, depth, path, session_type, state,
                     agent_id, title, manual_title, token_count, message_count, created_at, updated_at
              FROM session_metadata WHERE id IN ({})",
            placeholders.join(", ")
        );

        let mut query = sqlx::query_as::<_, SessionRow>(&sql);
        for ancestor_id in &node.path {
            query = query.bind(ancestor_id.as_str());
        }

        let rows = query
            .fetch_all(&self.pool)
            .await
            .map_err(|e| SessionError::StorageError {
                message: e.to_string(),
            })?;

        // Build a lookup map and reorder to match the original path order.
        let mut by_id: std::collections::HashMap<String, SessionNode> = rows
            .into_iter()
            .map(|r| {
                let node = r.into_session_node()?;
                Ok((node.id.as_str().to_string(), node))
            })
            .collect::<Result<_, SessionError>>()?;

        let mut ancestors = Vec::with_capacity(node.path.len());
        for ancestor_id in &node.path {
            if let Some(ancestor) = by_id.remove(ancestor_id.as_str()) {
                ancestors.push(ancestor);
            }
        }

        Ok(ancestors)
    }

    #[instrument(skip(self), fields(session_id = %id))]
    async fn set_title(&self, id: &SessionId, title: String) -> Result<(), SessionError> {
        let result = sqlx::query(
            r"UPDATE session_metadata
              SET title = ?1, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
              WHERE id = ?2",
        )
        .bind(&title)
        .bind(id.as_str())
        .execute(&self.pool)
        .await
        .map_err(|e| SessionError::StorageError {
            message: e.to_string(),
        })?;

        if result.rows_affected() == 0 {
            return Err(SessionError::NotFound { id: id.to_string() });
        }

        Ok(())
    }

    #[instrument(skip(self), fields(session_id = %id))]
    async fn set_manual_title(
        &self,
        id: &SessionId,
        title: Option<String>,
    ) -> Result<(), SessionError> {
        let result = sqlx::query(
            r"UPDATE session_metadata
              SET manual_title = ?1, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
              WHERE id = ?2",
        )
        .bind(title.as_deref())
        .bind(id.as_str())
        .execute(&self.pool)
        .await
        .map_err(|e| SessionError::StorageError {
            message: e.to_string(),
        })?;

        if result.rows_affected() == 0 {
            return Err(SessionError::NotFound { id: id.to_string() });
        }

        Ok(())
    }

    #[instrument(skip(self), fields(session_id = %id))]
    async fn delete(&self, id: &SessionId) -> Result<(), SessionError> {
        let result = sqlx::query("DELETE FROM session_metadata WHERE id = ?1")
            .bind(id.as_str())
            .execute(&self.pool)
            .await
            .map_err(|e| SessionError::StorageError {
                message: e.to_string(),
            })?;

        if result.rows_affected() == 0 {
            return Err(SessionError::NotFound { id: id.to_string() });
        }

        Ok(())
    }

    #[instrument(skip(self), fields(session_id = %id))]
    async fn get_context_reset_index(&self, id: &SessionId) -> Result<Option<u32>, SessionError> {
        let row: Option<(Option<i64>,)> =
            sqlx::query_as("SELECT context_reset_index FROM session_metadata WHERE id = ?1")
                .bind(id.as_str())
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| SessionError::StorageError {
                    message: e.to_string(),
                })?;

        match row {
            Some((val,)) => Ok(val.and_then(|v| u32::try_from(v).ok())),
            None => Err(SessionError::NotFound { id: id.to_string() }),
        }
    }

    #[instrument(skip(self), fields(session_id = %id, index = ?index))]
    async fn set_context_reset_index(
        &self,
        id: &SessionId,
        index: Option<u32>,
    ) -> Result<(), SessionError> {
        let result = sqlx::query(
            r"UPDATE session_metadata
              SET context_reset_index = ?1,
                  updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
              WHERE id = ?2",
        )
        .bind(index.map(i64::from))
        .bind(id.as_str())
        .execute(&self.pool)
        .await
        .map_err(|e| SessionError::StorageError {
            message: e.to_string(),
        })?;

        if result.rows_affected() == 0 {
            return Err(SessionError::NotFound { id: id.to_string() });
        }

        Ok(())
    }

    #[instrument(skip(self), fields(session_id = %id))]
    async fn get_custom_system_prompt(
        &self,
        id: &SessionId,
    ) -> Result<Option<String>, SessionError> {
        let row: Option<(Option<String>,)> =
            sqlx::query_as("SELECT custom_system_prompt FROM session_metadata WHERE id = ?1")
                .bind(id.as_str())
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| SessionError::StorageError {
                    message: e.to_string(),
                })?;

        match row {
            Some((val,)) => Ok(val),
            None => Err(SessionError::NotFound { id: id.to_string() }),
        }
    }

    #[instrument(skip(self, prompt), fields(session_id = %id, has_prompt = prompt.is_some()))]
    async fn set_custom_system_prompt(
        &self,
        id: &SessionId,
        prompt: Option<String>,
    ) -> Result<(), SessionError> {
        let result = sqlx::query(
            r"UPDATE session_metadata
              SET custom_system_prompt = ?1,
                  updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
              WHERE id = ?2",
        )
        .bind(prompt.as_deref())
        .bind(id.as_str())
        .execute(&self.pool)
        .await
        .map_err(|e| SessionError::StorageError {
            message: e.to_string(),
        })?;

        if result.rows_affected() == 0 {
            return Err(SessionError::NotFound { id: id.to_string() });
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Internal row mapping
// ---------------------------------------------------------------------------

#[derive(sqlx::FromRow)]
struct SessionRow {
    id: String,
    parent_id: Option<String>,
    root_id: String,
    depth: i64,
    path: String,
    session_type: String,
    state: String,
    agent_id: Option<String>,
    title: Option<String>,
    manual_title: Option<String>,
    token_count: i64,
    message_count: i64,
    created_at: String,
    updated_at: String,
}

impl SessionRow {
    fn into_session_node(self) -> Result<SessionNode, SessionError> {
        let path_strs: Vec<String> =
            serde_json::from_str(&self.path).map_err(|e| SessionError::StorageError {
                message: format!("parse path: {e}"),
            })?;
        let path: Vec<SessionId> = path_strs.into_iter().map(SessionId::from_string).collect();

        let session_type = str_to_session_type(&self.session_type)?;
        let state = str_to_state(&self.state)?;

        let created_at = chrono::DateTime::parse_from_rfc3339(&self.created_at)
            .map_or_else(|_| chrono::Utc::now(), |dt| dt.with_timezone(&chrono::Utc));

        let updated_at = chrono::DateTime::parse_from_rfc3339(&self.updated_at)
            .map_or_else(|_| chrono::Utc::now(), |dt| dt.with_timezone(&chrono::Utc));

        Ok(SessionNode {
            id: SessionId::from_string(self.id),
            parent_id: self.parent_id.map(SessionId::from_string),
            root_id: SessionId::from_string(self.root_id),
            depth: u32::try_from(self.depth).unwrap_or(0),
            path,
            session_type,
            state,
            agent_id: self.agent_id.map(AgentId::from_string),
            title: self.title,
            manual_title: self.manual_title,
            channel: None,
            label: None,
            token_count: u32::try_from(self.token_count).unwrap_or(0),
            message_count: u32::try_from(self.message_count).unwrap_or(0),
            last_compaction: None,
            compaction_count: 0,
            created_at,
            updated_at,
        })
    }
}

// ---------------------------------------------------------------------------
// Conversion helpers
// ---------------------------------------------------------------------------

fn session_type_to_str(t: &SessionType) -> &'static str {
    match t {
        SessionType::Main => "main",
        SessionType::Child => "child",
        SessionType::Branch => "branch",
        SessionType::Ephemeral => "ephemeral",
        SessionType::SubAgent => "sub_agent",
        SessionType::Canonical => "canonical",
    }
}

fn str_to_session_type(s: &str) -> Result<SessionType, SessionError> {
    match s {
        "main" => Ok(SessionType::Main),
        "child" => Ok(SessionType::Child),
        "branch" => Ok(SessionType::Branch),
        "ephemeral" => Ok(SessionType::Ephemeral),
        "sub_agent" => Ok(SessionType::SubAgent),
        "canonical" => Ok(SessionType::Canonical),
        other => Err(SessionError::Other {
            message: format!("unknown session_type: {other}"),
        }),
    }
}

fn state_to_str(s: &SessionState) -> &'static str {
    match s {
        SessionState::Active => "active",
        SessionState::Paused => "paused",
        SessionState::Archived => "archived",
        SessionState::Merged => "merged",
        SessionState::Tombstone => "tombstone",
    }
}

fn str_to_state(s: &str) -> Result<SessionState, SessionError> {
    match s {
        "active" => Ok(SessionState::Active),
        "paused" => Ok(SessionState::Paused),
        "archived" => Ok(SessionState::Archived),
        "merged" => Ok(SessionState::Merged),
        "tombstone" => Ok(SessionState::Tombstone),
        other => Err(SessionError::Other {
            message: format!("unknown session state: {other}"),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::StorageConfig;
    use crate::migration::run_embedded_migrations;
    use crate::pool::create_pool;

    async fn setup() -> SqliteSessionStore {
        let config = StorageConfig::in_memory();
        let pool = create_pool(&config).await.unwrap();
        run_embedded_migrations(&pool).await.unwrap();
        SqliteSessionStore::new(pool)
    }

    #[tokio::test]
    async fn test_session_create_root() {
        let store = setup().await;
        let node = store
            .create(CreateSessionOptions {
                parent_id: None,
                session_type: SessionType::Main,
                agent_id: None,
                title: Some("Test Session".into()),
            })
            .await
            .unwrap();

        assert_eq!(node.root_id, node.id);
        assert_eq!(node.depth, 0);
        assert!(node.path.is_empty());
        assert_eq!(node.session_type, SessionType::Main);
        assert_eq!(node.state, SessionState::Active);
    }

    #[tokio::test]
    async fn test_session_create_child() {
        let store = setup().await;
        let parent = store
            .create(CreateSessionOptions {
                parent_id: None,
                session_type: SessionType::Main,
                agent_id: None,
                title: None,
            })
            .await
            .unwrap();

        let child = store
            .create(CreateSessionOptions {
                parent_id: Some(parent.id.clone()),
                session_type: SessionType::Child,
                agent_id: None,
                title: None,
            })
            .await
            .unwrap();

        assert_eq!(child.parent_id, Some(parent.id.clone()));
        assert_eq!(child.depth, 1);
        assert_eq!(child.path, vec![parent.id.clone()]);
        assert_eq!(child.root_id, parent.root_id);
    }

    #[tokio::test]
    async fn test_session_create_branch() {
        let store = setup().await;
        let main = store
            .create(CreateSessionOptions {
                parent_id: None,
                session_type: SessionType::Main,
                agent_id: None,
                title: None,
            })
            .await
            .unwrap();

        let branch = store
            .create(CreateSessionOptions {
                parent_id: Some(main.id.clone()),
                session_type: SessionType::Branch,
                agent_id: None,
                title: None,
            })
            .await
            .unwrap();

        assert_eq!(branch.session_type, SessionType::Branch);
        assert_eq!(branch.root_id, main.root_id);
    }

    #[tokio::test]
    async fn test_session_get_by_id() {
        let store = setup().await;
        let created = store
            .create(CreateSessionOptions {
                parent_id: None,
                session_type: SessionType::Main,
                agent_id: None,
                title: Some("Findable".into()),
            })
            .await
            .unwrap();

        let found = store.get(&created.id).await.unwrap();
        assert_eq!(found.id, created.id);
        assert_eq!(found.title, Some("Findable".into()));
    }

    #[tokio::test]
    async fn test_session_get_not_found() {
        let store = setup().await;
        let result = store.get(&SessionId::from_string("nonexistent")).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), SessionError::NotFound { .. }));
    }

    #[tokio::test]
    async fn test_session_list_by_state() {
        let store = setup().await;
        let s1 = store
            .create(CreateSessionOptions {
                parent_id: None,
                session_type: SessionType::Main,
                agent_id: None,
                title: None,
            })
            .await
            .unwrap();

        store.set_state(&s1.id, SessionState::Paused).await.unwrap();

        let _s2 = store
            .create(CreateSessionOptions {
                parent_id: None,
                session_type: SessionType::Main,
                agent_id: None,
                title: None,
            })
            .await
            .unwrap();

        let paused = store
            .list(&SessionFilter {
                state: Some(SessionState::Paused),
                ..Default::default()
            })
            .await
            .unwrap();

        assert_eq!(paused.len(), 1);
        assert_eq!(paused[0].id, s1.id);
    }

    #[tokio::test]
    async fn test_session_list_by_agent() {
        let store = setup().await;
        let agent = AgentId::from_string("agent-1");
        let _s1 = store
            .create(CreateSessionOptions {
                parent_id: None,
                session_type: SessionType::Main,
                agent_id: Some(agent.clone()),
                title: None,
            })
            .await
            .unwrap();

        let _s2 = store
            .create(CreateSessionOptions {
                parent_id: None,
                session_type: SessionType::Main,
                agent_id: None,
                title: None,
            })
            .await
            .unwrap();

        let agent_sessions = store
            .list(&SessionFilter {
                agent_id: Some(agent),
                ..Default::default()
            })
            .await
            .unwrap();

        assert_eq!(agent_sessions.len(), 1);
    }

    #[tokio::test]
    async fn test_session_set_state() {
        let store = setup().await;
        let session = store
            .create(CreateSessionOptions {
                parent_id: None,
                session_type: SessionType::Main,
                agent_id: None,
                title: None,
            })
            .await
            .unwrap();

        store
            .set_state(&session.id, SessionState::Paused)
            .await
            .unwrap();

        let updated = store.get(&session.id).await.unwrap();
        assert_eq!(updated.state, SessionState::Paused);
    }

    #[tokio::test]
    async fn test_session_update_metadata() {
        let store = setup().await;
        let session = store
            .create(CreateSessionOptions {
                parent_id: None,
                session_type: SessionType::Main,
                agent_id: None,
                title: None,
            })
            .await
            .unwrap();

        store
            .update_metadata(&session.id, Some("New Title".into()), 500, 10)
            .await
            .unwrap();

        let updated = store.get(&session.id).await.unwrap();
        assert_eq!(updated.title, Some("New Title".into()));
        assert_eq!(updated.token_count, 500);
        assert_eq!(updated.message_count, 10);
    }

    #[tokio::test]
    async fn test_session_children() {
        let store = setup().await;
        let parent = store
            .create(CreateSessionOptions {
                parent_id: None,
                session_type: SessionType::Main,
                agent_id: None,
                title: None,
            })
            .await
            .unwrap();

        let _c1 = store
            .create(CreateSessionOptions {
                parent_id: Some(parent.id.clone()),
                session_type: SessionType::Child,
                agent_id: None,
                title: None,
            })
            .await
            .unwrap();

        let _c2 = store
            .create(CreateSessionOptions {
                parent_id: Some(parent.id.clone()),
                session_type: SessionType::Child,
                agent_id: None,
                title: None,
            })
            .await
            .unwrap();

        let children = store.children(&parent.id).await.unwrap();
        assert_eq!(children.len(), 2);
    }

    #[tokio::test]
    async fn test_session_ancestors() {
        let store = setup().await;
        let root = store
            .create(CreateSessionOptions {
                parent_id: None,
                session_type: SessionType::Main,
                agent_id: None,
                title: Some("Root".into()),
            })
            .await
            .unwrap();

        let child = store
            .create(CreateSessionOptions {
                parent_id: Some(root.id.clone()),
                session_type: SessionType::Child,
                agent_id: None,
                title: Some("Child".into()),
            })
            .await
            .unwrap();

        let grandchild = store
            .create(CreateSessionOptions {
                parent_id: Some(child.id.clone()),
                session_type: SessionType::Child,
                agent_id: None,
                title: Some("Grandchild".into()),
            })
            .await
            .unwrap();

        let ancestors = store.ancestors(&grandchild.id).await.unwrap();
        assert_eq!(ancestors.len(), 2);
        assert_eq!(ancestors[0].id, root.id);
        assert_eq!(ancestors[1].id, child.id);
    }
}
