//! Service-owned workspace identity and resume behavior.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use serde::{Deserialize, Serialize};
use y_core::session::{CreateSessionOptions, SessionFilter, SessionNode, SessionState};
use y_core::types::{AgentId, SessionId};
use y_session::SessionManager;

use crate::workspace::{canonical_workspace_path, WorkspaceService};

/// Shared business operations for interactive session creation and resume.
pub struct SessionService;

#[derive(Debug, Clone)]
pub struct SessionHubItem {
    pub session: SessionNode,
    pub pinned: bool,
    pub quick_slot: Option<u8>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct SessionHubPreferences {
    #[serde(default)]
    pinned: HashSet<String>,
    #[serde(default)]
    quick_slots: BTreeMap<u8, String>,
}

impl SessionService {
    const PUBLIC_SESSION_PREFIX: &'static str = "ses_";

    /// Render a typed public reference without changing the stored identifier.
    pub fn public_session_reference(session_id: &SessionId) -> String {
        format!("{}{session_id}", Self::PUBLIC_SESSION_PREFIX)
    }

    /// Accept either the typed public reference or a legacy raw identifier.
    pub fn raw_session_target(target: &str) -> &str {
        target
            .strip_prefix(Self::PUBLIC_SESSION_PREFIX)
            .unwrap_or(target)
    }

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
        let raw_target = Self::raw_session_target(target);
        if let Some(exact) = sessions
            .iter()
            .find(|session| session.id.as_str() == raw_target)
        {
            return Ok(Some(exact.clone()));
        }

        let target_lower = raw_target.to_lowercase();
        let matches = sessions
            .into_iter()
            .filter(|session| {
                session.id.as_str().starts_with(raw_target)
                    || session
                        .manual_title
                        .as_ref()
                        .or(session.title.as_ref())
                        .is_some_and(|title| title.to_lowercase().contains(&target_lower))
            })
            .collect::<Vec<_>>();

        match matches.len() {
            0 => Ok(None),
            1 => Ok(matches.into_iter().next()),
            count => anyhow::bail!(
                "ambiguous session target '{target}' matched {count} sessions; use a longer ID prefix"
            ),
        }
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

    pub async fn rename_session(
        manager: &SessionManager,
        session_id: &SessionId,
        title: &str,
    ) -> anyhow::Result<()> {
        let title = title.trim();
        if title.is_empty() {
            anyhow::bail!("session title cannot be empty");
        }
        if title.chars().count() > 120 {
            anyhow::bail!("session title cannot exceed 120 characters");
        }
        manager
            .set_manual_title(session_id, Some(title.to_string()))
            .await
            .map_err(Into::into)
    }

    pub async fn archive_session(
        manager: &SessionManager,
        session_id: &SessionId,
    ) -> anyhow::Result<()> {
        manager
            .transition_state(session_id, SessionState::Archived)
            .await
            .map_err(Into::into)
    }

    pub async fn delete_session(
        manager: &SessionManager,
        session_id: &SessionId,
    ) -> anyhow::Result<()> {
        manager.delete_session(session_id).await.map_err(Into::into)
    }

    pub async fn fork_session(
        manager: &SessionManager,
        source_id: &SessionId,
        message_index: usize,
        title: Option<String>,
    ) -> anyhow::Result<SessionNode> {
        manager
            .fork_session(source_id, message_index, title)
            .await
            .map_err(Into::into)
    }

    pub async fn list_session_hub(
        manager: &SessionManager,
        workspace: &Path,
        preferences_path: &Path,
    ) -> anyhow::Result<Vec<SessionHubItem>> {
        let workspace_path = canonical_workspace_path(workspace)?;
        let preferences = load_hub_preferences(preferences_path).await?;
        let mut sessions = manager
            .list_sessions(&SessionFilter {
                workspace_path: Some(workspace_path),
                ..SessionFilter::default()
            })
            .await?;
        sessions.retain(|session| {
            session.session_type.is_user_facing()
                && matches!(session.state, SessionState::Active | SessionState::Archived)
        });
        let mut items = sessions
            .into_iter()
            .map(|session| {
                let id = session.id.to_string();
                SessionHubItem {
                    pinned: preferences.pinned.contains(&id),
                    quick_slot: preferences
                        .quick_slots
                        .iter()
                        .find_map(|(slot, assigned)| (assigned == &id).then_some(*slot)),
                    session,
                }
            })
            .collect::<Vec<_>>();
        items.sort_by(|left, right| {
            right
                .pinned
                .cmp(&left.pinned)
                .then_with(|| right.session.updated_at.cmp(&left.session.updated_at))
        });
        Ok(items)
    }

    pub async fn set_pinned(
        preferences_path: &Path,
        session_id: &SessionId,
        pinned: bool,
    ) -> anyhow::Result<()> {
        let mut preferences = load_hub_preferences(preferences_path).await?;
        if pinned {
            preferences.pinned.insert(session_id.to_string());
        } else {
            preferences.pinned.remove(session_id.as_str());
        }
        save_hub_preferences(preferences_path, &preferences).await
    }

    pub async fn assign_quick_slot(
        preferences_path: &Path,
        slot: u8,
        session_id: &SessionId,
    ) -> anyhow::Result<()> {
        if !(1..=9).contains(&slot) {
            anyhow::bail!("quick slot must be between 1 and 9");
        }
        let mut preferences = load_hub_preferences(preferences_path).await?;
        let id = session_id.to_string();
        preferences
            .quick_slots
            .retain(|_, assigned| assigned != &id);
        preferences.quick_slots.insert(slot, id);
        save_hub_preferences(preferences_path, &preferences).await
    }

    /// Remove pin and quick-slot references for a deleted session.
    pub async fn remove_hub_preferences(
        preferences_path: &Path,
        session_id: &SessionId,
    ) -> anyhow::Result<()> {
        let mut preferences = load_hub_preferences(preferences_path).await?;
        preferences.pinned.remove(session_id.as_str());
        preferences
            .quick_slots
            .retain(|_, assigned| assigned != session_id.as_str());
        save_hub_preferences(preferences_path, &preferences).await
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

async fn load_hub_preferences(path: &Path) -> anyhow::Result<SessionHubPreferences> {
    match tokio::fs::read(path).await {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(Into::into),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(SessionHubPreferences::default())
        }
        Err(error) => Err(error.into()),
    }
}

async fn save_hub_preferences(
    path: &Path,
    preferences: &SessionHubPreferences,
) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let temporary = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(preferences)?;
    tokio::fs::write(&temporary, bytes).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        tokio::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o600)).await?;
    }
    tokio::fs::rename(temporary, path).await?;
    Ok(())
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

    #[tokio::test]
    async fn session_hub_operations_rename_archive_pin_and_assign_slot() {
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
        let preferences = transcript_dir.path().join("session-hub.json");

        SessionService::rename_session(&manager, &source.id, "Renamed")
            .await
            .unwrap();
        SessionService::set_pinned(&preferences, &source.id, true)
            .await
            .unwrap();
        SessionService::assign_quick_slot(&preferences, 2, &source.id)
            .await
            .unwrap();

        let hub = SessionService::list_session_hub(&manager, transcript_dir.path(), &preferences)
            .await
            .unwrap();
        assert_eq!(hub.len(), 1);
        assert_eq!(hub[0].session.manual_title.as_deref(), Some("Renamed"));
        assert!(hub[0].pinned);
        assert_eq!(hub[0].quick_slot, Some(2));

        SessionService::archive_session(&manager, &source.id)
            .await
            .unwrap();
        let hub = SessionService::list_session_hub(&manager, transcript_dir.path(), &preferences)
            .await
            .unwrap();
        assert_eq!(hub[0].session.state, SessionState::Archived);
    }

    #[tokio::test]
    async fn assigning_a_quick_slot_replaces_the_previous_session() {
        let (manager, transcript_dir) = setup_manager().await;
        let first = SessionService::create_session(
            &manager,
            CreateSessionOptions {
                parent_id: None,
                session_type: SessionType::Main,
                agent_id: None,
                title: Some("First".into()),
            },
            transcript_dir.path(),
        )
        .await
        .unwrap();
        let second = SessionService::create_session(
            &manager,
            CreateSessionOptions {
                parent_id: None,
                session_type: SessionType::Main,
                agent_id: None,
                title: Some("Second".into()),
            },
            transcript_dir.path(),
        )
        .await
        .unwrap();
        let preferences = transcript_dir.path().join("session-hub.json");
        SessionService::assign_quick_slot(&preferences, 1, &first.id)
            .await
            .unwrap();
        SessionService::assign_quick_slot(&preferences, 1, &second.id)
            .await
            .unwrap();

        let hub = SessionService::list_session_hub(&manager, transcript_dir.path(), &preferences)
            .await
            .unwrap();
        assert_eq!(
            hub.iter()
                .find(|item| item.quick_slot == Some(1))
                .unwrap()
                .session
                .id,
            second.id
        );
    }

    #[tokio::test]
    async fn deleted_session_preferences_are_removed() {
        let (manager, transcript_dir) = setup_manager().await;
        let source = SessionService::create_session(
            &manager,
            CreateSessionOptions {
                parent_id: None,
                session_type: SessionType::Main,
                agent_id: None,
                title: Some("Disposable".into()),
            },
            transcript_dir.path(),
        )
        .await
        .unwrap();
        let preferences = transcript_dir.path().join("session-hub.json");
        SessionService::set_pinned(&preferences, &source.id, true)
            .await
            .unwrap();
        SessionService::assign_quick_slot(&preferences, 1, &source.id)
            .await
            .unwrap();

        SessionService::remove_hub_preferences(&preferences, &source.id)
            .await
            .unwrap();

        let saved = load_hub_preferences(&preferences).await.unwrap();
        assert!(!saved.pinned.contains(source.id.as_str()));
        assert!(saved.quick_slots.is_empty());
    }

    #[test]
    fn public_session_reference_is_typed_and_reversible() {
        let id = SessionId::from_string("0197dd0d-9f61-7a73-9aef-8d7e349a54f2");

        let public = SessionService::public_session_reference(&id);

        assert_eq!(public, "ses_0197dd0d-9f61-7a73-9aef-8d7e349a54f2");
        assert_eq!(SessionService::raw_session_target(&public), id.as_str());
        assert_eq!(SessionService::raw_session_target(id.as_str()), id.as_str());
    }

    #[tokio::test]
    async fn resolve_resume_target_accepts_public_session_reference() {
        let (manager, transcript_dir) = setup_manager().await;
        let source = SessionService::create_session(
            &manager,
            CreateSessionOptions {
                parent_id: None,
                session_type: SessionType::Main,
                agent_id: None,
                title: Some("CVE analysis".into()),
            },
            transcript_dir.path(),
        )
        .await
        .unwrap();
        let public = SessionService::public_session_reference(&source.id);

        let resolved =
            SessionService::resolve_resume_target(&manager, transcript_dir.path(), None, &public)
                .await
                .unwrap()
                .unwrap();

        assert_eq!(resolved.id, source.id);
    }

    #[tokio::test]
    async fn resolve_resume_target_rejects_ambiguous_titles() {
        let (manager, transcript_dir) = setup_manager().await;
        for title in ["CVE analysis alpha", "CVE analysis beta"] {
            SessionService::create_session(
                &manager,
                CreateSessionOptions {
                    parent_id: None,
                    session_type: SessionType::Main,
                    agent_id: None,
                    title: Some(title.into()),
                },
                transcript_dir.path(),
            )
            .await
            .unwrap();
        }

        let error = SessionService::resolve_resume_target(
            &manager,
            transcript_dir.path(),
            None,
            "CVE analysis",
        )
        .await
        .unwrap_err();

        assert!(error.to_string().contains("ambiguous session target"));
    }
}
