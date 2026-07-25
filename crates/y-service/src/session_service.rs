//! Service-owned workspace identity and resume behavior.

use std::path::Path;

use y_core::session::{CreateSessionOptions, SessionFilter, SessionNode, SessionState};
use y_core::types::{AgentId, SessionId};
use y_session::SessionManager;

use crate::workspace::{canonical_workspace_path, WorkspaceService};

/// Shared business operations for interactive session creation and resume.
pub struct SessionService;

impl SessionService {
    /// Create an interactive session bound to one canonical workspace.
    pub async fn create_session(
        manager: &SessionManager,
        options: CreateSessionOptions,
        workspace: &Path,
    ) -> anyhow::Result<SessionNode> {
        let workspace_path = canonical_workspace_path(workspace)?;
        manager
            .create_session_in_workspace(options, Some(&workspace_path))
            .await
            .map_err(Into::into)
    }

    /// List active, user-facing sessions in exactly one canonical workspace.
    pub async fn list_resumable_sessions(
        manager: &SessionManager,
        workspace: &Path,
        agent_id: Option<AgentId>,
    ) -> anyhow::Result<Vec<SessionNode>> {
        let workspace_path = canonical_workspace_path(workspace)?;
        let mut sessions = manager
            .list_sessions(&SessionFilter {
                state: Some(SessionState::Active),
                agent_id,
                workspace_path: Some(workspace_path),
                ..SessionFilter::default()
            })
            .await?;
        sessions.retain(|session| session.session_type.is_user_facing());
        sessions.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
        Ok(sessions)
    }

    /// Resolve a workspace-scoped resume target by ID prefix or title text.
    pub async fn resolve_resume_target(
        manager: &SessionManager,
        workspace: &Path,
        agent_id: Option<AgentId>,
        target: &str,
    ) -> anyhow::Result<Option<SessionNode>> {
        let sessions = Self::list_resumable_sessions(manager, workspace, agent_id).await?;
        let target_lower = target.to_lowercase();
        Ok(sessions.into_iter().find(|session| {
            session.id.as_str().starts_with(target)
                || session
                    .manual_title
                    .as_ref()
                    .or(session.title.as_ref())
                    .is_some_and(|title| title.to_lowercase().contains(&target_lower))
        }))
    }

    /// Assign an existing session to a canonical workspace.
    pub async fn assign_workspace(
        manager: &SessionManager,
        session_id: &SessionId,
        workspace: &Path,
    ) -> anyhow::Result<String> {
        let workspace_path = canonical_workspace_path(workspace)?;
        manager
            .set_workspace_path(session_id, Some(workspace_path.clone()))
            .await?;
        Ok(workspace_path)
    }

    /// Clear an existing session's workspace identity.
    pub async fn unassign_workspace(
        manager: &SessionManager,
        session_id: &SessionId,
    ) -> anyhow::Result<()> {
        manager.set_workspace_path(session_id, None).await?;
        Ok(())
    }

    /// Create a non-destructive branch immediately before one transcript message.
    ///
    /// The child inherits the source session's workspace identity. The selected
    /// message is intentionally excluded so a presentation layer can restore it
    /// to its composer for editing.
    pub async fn branch_before_message(
        manager: &SessionManager,
        source_id: &SessionId,
        message_index: usize,
        title: Option<String>,
    ) -> anyhow::Result<SessionNode> {
        manager
            .fork_session_before_message(source_id, message_index, title)
            .await
            .map_err(Into::into)
    }

    /// Backfill missing `SQLite` workspace identities from the legacy TOML map.
    pub async fn backfill_legacy_assignments(
        manager: &SessionManager,
        workspaces: &WorkspaceService,
    ) -> anyhow::Result<usize> {
        let legacy_paths = workspaces.session_workspace_paths();
        if legacy_paths.is_empty() {
            return Ok(0);
        }

        let sessions = manager.list_sessions(&SessionFilter::default()).await?;
        let mut updated = 0;
        for session in sessions {
            if session.workspace_path.is_some() {
                continue;
            }
            let Some(path) = legacy_paths.get(session.id.as_str()) else {
                continue;
            };
            let Ok(canonical_path) = canonical_workspace_path(Path::new(path)) else {
                continue;
            };
            manager
                .set_workspace_path(&session.id, Some(canonical_path))
                .await?;
            updated += 1;
        }
        Ok(updated)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use y_core::session::{CreateSessionOptions, SessionType};
    use y_core::types::{Message, Role};
    use y_session::SessionConfig;

    use super::*;

    async fn setup_manager() -> (SessionManager, tempfile::TempDir) {
        let config = y_storage::StorageConfig::in_memory();
        let pool = y_storage::create_pool(&config).await.unwrap();
        y_storage::migration::run_embedded_migrations(&pool)
            .await
            .unwrap();

        let session_store = Arc::new(y_storage::SqliteSessionStore::new(pool));
        let transcript_dir = tempfile::tempdir().unwrap();
        let transcript_store =
            Arc::new(y_storage::JsonlTranscriptStore::new(transcript_dir.path()));
        let display_transcript_store = Arc::new(y_storage::JsonlDisplayTranscriptStore::new(
            transcript_dir.path(),
        ));
        let manager = SessionManager::new(
            session_store,
            transcript_store,
            display_transcript_store,
            SessionConfig::default(),
        );
        (manager, transcript_dir)
    }

    fn user_message(content: &str) -> Message {
        Message {
            message_id: y_core::types::generate_message_id(),
            role: Role::User,
            content: content.to_string(),
            tool_call_id: None,
            tool_calls: Vec::new(),
            timestamp: chrono::Utc::now(),
            metadata: serde_json::Value::Null,
        }
    }

    #[tokio::test]
    async fn branch_before_message_preserves_workspace_and_source() {
        let (manager, transcript_dir) = setup_manager().await;
        let source = SessionService::create_session(
            &manager,
            CreateSessionOptions {
                parent_id: None,
                session_type: SessionType::Main,
                agent_id: None,
                title: Some("Original".into()),
            },
            transcript_dir.path(),
        )
        .await
        .unwrap();
        for content in ["first", "second", "third"] {
            manager
                .append_message(&source.id, &user_message(content))
                .await
                .unwrap();
        }

        let branch = SessionService::branch_before_message(
            &manager,
            &source.id,
            1,
            Some("Backtrack".into()),
        )
        .await
        .unwrap();

        let branch_messages = manager.read_display_transcript(&branch.id).await.unwrap();
        assert_eq!(branch_messages.len(), 1);
        assert_eq!(branch_messages[0].content, "first");
        assert_eq!(branch.workspace_path, source.workspace_path);
        assert_eq!(
            manager
                .read_display_transcript(&source.id)
                .await
                .unwrap()
                .len(),
            3
        );
    }
}
