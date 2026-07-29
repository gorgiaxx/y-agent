//! Service-owned preparation of prompt and permission context for chat turns.

use std::path::Path;

use y_core::permission_types::PermissionMode;

use crate::chat::{ChatService, PreparedTurn};
use crate::container::ServiceContainer;
use crate::prompt_templates::{PromptTemplateService, SessionPromptConfig};
use crate::workspace::WorkspaceService;

struct ResolvedPreparedTurnContext {
    agent_mode: String,
    working_directory: Option<String>,
    available_tools: Vec<String>,
    custom_system_prompt: Option<String>,
    selected_prompt_sections: Option<Vec<String>>,
    default_permission: Option<PermissionMode>,
}

impl ResolvedPreparedTurnContext {
    fn resolve(
        prepared: &PreparedTurn,
        session_prompt_config: SessionPromptConfig,
        workspace_path: Option<String>,
        fallback_working_directory: Option<String>,
    ) -> Self {
        let (
            agent_mode,
            agent_working_directory,
            available_tools,
            agent_prompt,
            agent_prompt_sections,
            default_permission,
        ) = prepared.agent_config.as_ref().map_or_else(
            || (String::new(), None, Vec::new(), None, None, None),
            |config| {
                (
                    config.agent_mode.clone(),
                    config.working_directory.clone(),
                    if config.features.toolcall && !config.allowed_tools.is_empty() {
                        config.allowed_tools.clone()
                    } else {
                        Vec::new()
                    },
                    config.system_prompt.clone(),
                    (!config.prompt_section_ids.is_empty())
                        .then(|| config.prompt_section_ids.clone()),
                    config.permission_mode,
                )
            },
        );
        let working_directory = normalize_directory(agent_working_directory)
            .or_else(|| normalize_directory(prepared.working_directory.clone()))
            .or_else(|| normalize_directory(workspace_path))
            .or_else(|| normalize_directory(fallback_working_directory));

        Self {
            agent_mode,
            working_directory,
            available_tools,
            custom_system_prompt: session_prompt_config.system_prompt.or(agent_prompt),
            selected_prompt_sections: (!session_prompt_config.prompt_section_ids.is_empty())
                .then_some(session_prompt_config.prompt_section_ids)
                .or(agent_prompt_sections),
            default_permission,
        }
    }
}

impl ChatService {
    /// Apply session, agent, and workspace context to a prepared turn.
    ///
    /// Presentation layers call this after preparing or restoring a turn and
    /// before handing it to the shared chat worker.
    pub async fn apply_prepared_turn_context(
        container: &ServiceContainer,
        config_dir: &Path,
        fallback_working_directory: Option<&Path>,
        prepared: &mut PreparedTurn,
    ) {
        let workspace_path = match container
            .session_manager
            .get_session(&prepared.session_id)
            .await
        {
            Ok(session) => session.workspace_path,
            Err(error) => {
                tracing::warn!(
                    session_id = %prepared.session_id,
                    %error,
                    "failed to load persisted session workspace; checking legacy assignment"
                );
                None
            }
        }
        .or_else(|| {
            WorkspaceService::new(config_dir).resolve_workspace_path(prepared.session_id.as_str())
        });
        let session_prompt_config = PromptTemplateService::get_session_config(
            &container.session_manager,
            &prepared.session_id,
        )
        .await
        .unwrap_or_else(|error| {
            tracing::warn!(
                session_id = %prepared.session_id,
                %error,
                "failed to load session prompt context; using defaults"
            );
            SessionPromptConfig::default()
        });
        let resolved = ResolvedPreparedTurnContext::resolve(
            prepared,
            session_prompt_config,
            workspace_path,
            fallback_working_directory.map(|path| path.to_string_lossy().into_owned()),
        );

        tracing::info!(
            session_id = %prepared.session_id,
            working_directory = ?resolved.working_directory,
            skills = ?prepared.skills,
            knowledge_collections = ?prepared.knowledge_collections,
            has_custom_prompt = resolved.custom_system_prompt.is_some(),
            agent_mode = %resolved.agent_mode,
            "applied prompt context for prepared turn"
        );

        prepared
            .working_directory
            .clone_from(&resolved.working_directory);
        {
            let mut prompt_context = container.prompt_context.write().await;
            prompt_context.agent_mode = resolved.agent_mode;
            prompt_context.working_directory = resolved.working_directory;
            prompt_context.custom_system_prompt = resolved.custom_system_prompt;
            prompt_context.active_skills.clone_from(&prepared.skills);
            prompt_context.available_tools = resolved.available_tools;
            prompt_context.selected_prompt_sections = resolved.selected_prompt_sections;
        }

        if let Some(default_permission) = resolved.default_permission {
            container
                .session_state
                .session_permission_modes
                .write()
                .await
                .entry(prepared.session_id.clone())
                .or_insert(default_permission);
        }
    }
}

fn normalize_directory(path: Option<String>) -> Option<String> {
    path.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    })
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;
    use y_core::permission_types::PermissionMode;
    use y_core::provider::RequestMode;
    use y_core::session::{CreateSessionOptions, SessionType};
    use y_core::trust::TrustTier;
    use y_core::types::SessionId;

    use super::*;
    use crate::chat_types::{OperationMode, SessionAgentConfig, SessionAgentFeatures};

    async fn make_test_container() -> (ServiceContainer, tempfile::TempDir) {
        let config_dir = tempfile::TempDir::new().unwrap();
        let config = crate::config::ServiceConfig {
            storage: y_storage::StorageConfig {
                db_path: ":memory:".to_string(),
                pool_size: 1,
                wal_enabled: false,
                transcript_dir: config_dir.path().join("transcripts"),
                ..y_storage::StorageConfig::default()
            },
            ..Default::default()
        };
        let container = ServiceContainer::from_config(&config)
            .await
            .expect("test container should build");
        (container, config_dir)
    }

    fn prepared_turn(session_id: SessionId) -> PreparedTurn {
        PreparedTurn {
            session_id,
            session_uuid: Uuid::new_v4(),
            history: Vec::new(),
            turn_number: 1,
            user_input: "hello".into(),
            provider_id: None,
            request_mode: RequestMode::TextChat,
            session_created: false,
            working_directory: None,
            knowledge_collections: Vec::new(),
            thinking: None,
            plan_mode: None,
            operation_mode: OperationMode::Default,
            mcp_mode: None,
            mcp_servers: Vec::new(),
            skills: Vec::new(),
            agent_config: None,
            image_generation_options: None,
            pre_turn_message_count: None,
        }
    }

    fn agent_config() -> SessionAgentConfig {
        SessionAgentConfig {
            agent_id: "reviewer".into(),
            agent_name: "reviewer".into(),
            agent_mode: "build".into(),
            working_directory: Some("  /agent/path  ".into()),
            features: SessionAgentFeatures {
                toolcall: true,
                skills: true,
                knowledge: true,
            },
            allowed_tools: vec!["FileRead".into()],
            preset_skills: Vec::new(),
            knowledge_collections: Vec::new(),
            prompt_section_ids: vec!["agent.section".into()],
            system_prompt: Some("agent prompt".into()),
            provider_id: None,
            preferred_models: Vec::new(),
            provider_tags: Vec::new(),
            temperature: None,
            max_completion_tokens: None,
            thinking: None,
            plan_mode: None,
            permission_mode: Some(PermissionMode::DontAsk),
            max_iterations: 1,
            max_tool_calls: 1,
            trust_tier: TrustTier::BuiltIn,
            prune_tool_history: false,
            mcp_mode: None,
            mcp_servers: Vec::new(),
        }
    }

    async fn create_session(container: &ServiceContainer, title: &str) -> SessionId {
        container
            .session_manager
            .create_session(CreateSessionOptions {
                parent_id: None,
                session_type: SessionType::Main,
                agent_id: None,
                title: Some(title.into()),
            })
            .await
            .unwrap()
            .id
    }

    #[tokio::test]
    async fn test_apply_prepared_turn_context_resolves_session_agent_and_workspace_state() {
        let (container, config_dir) = make_test_container().await;
        let session_id = create_session(&container, "turn context").await;
        PromptTemplateService::set_session_config(
            &container.session_manager,
            &session_id,
            &SessionPromptConfig {
                system_prompt: Some("session prompt".into()),
                prompt_section_ids: vec!["session.section".into()],
                template_id: Some("session-template".into()),
            },
        )
        .await
        .unwrap();
        let workspace = WorkspaceService::new(config_dir.path())
            .create("workspace".into(), "/workspace/path".into())
            .unwrap();
        WorkspaceService::new(config_dir.path())
            .assign_session(workspace.id, session_id.0.clone())
            .unwrap();
        let mut prepared = prepared_turn(session_id.clone());
        prepared.skills = vec!["rust-review".into()];
        prepared.agent_config = Some(agent_config());

        ChatService::apply_prepared_turn_context(
            &container,
            config_dir.path(),
            None,
            &mut prepared,
        )
        .await;

        assert_eq!(prepared.working_directory.as_deref(), Some("/agent/path"));
        let context = container.prompt_context.read().await;
        assert_eq!(context.agent_mode, "build");
        assert_eq!(context.working_directory.as_deref(), Some("/agent/path"));
        assert_eq!(
            context.custom_system_prompt.as_deref(),
            Some("session prompt")
        );
        assert_eq!(context.active_skills, vec!["rust-review"]);
        assert_eq!(context.available_tools, vec!["FileRead"]);
        assert_eq!(
            context.selected_prompt_sections.as_deref(),
            Some(["session.section".to_string()].as_slice())
        );
        drop(context);
        assert_eq!(
            container
                .session_state
                .session_permission_modes
                .read()
                .await
                .get(&session_id),
            Some(&PermissionMode::DontAsk)
        );
    }

    #[tokio::test]
    async fn test_apply_prepared_turn_context_uses_fallback_for_unassigned_session() {
        let (container, config_dir) = make_test_container().await;
        let session_id = create_session(&container, "fallback context").await;
        let fallback = config_dir.path().join("tmp");
        let mut prepared = prepared_turn(session_id);

        ChatService::apply_prepared_turn_context(
            &container,
            config_dir.path(),
            Some(&fallback),
            &mut prepared,
        )
        .await;

        assert_eq!(
            prepared.working_directory.as_deref(),
            Some(fallback.to_string_lossy().as_ref())
        );
    }

    #[tokio::test]
    async fn test_apply_prepared_turn_context_prefers_persisted_session_workspace() {
        let (container, config_dir) = make_test_container().await;
        let workspace = config_dir.path().join("persisted-workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let session = crate::SessionService::create_session(
            &container.session_manager,
            CreateSessionOptions {
                parent_id: None,
                session_type: SessionType::Main,
                agent_id: None,
                title: Some("workspace context".into()),
            },
            &workspace,
        )
        .await
        .unwrap();
        let mut prepared = prepared_turn(session.id);

        ChatService::apply_prepared_turn_context(
            &container,
            config_dir.path(),
            None,
            &mut prepared,
        )
        .await;

        let expected = workspace
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert_eq!(
            prepared.working_directory.as_deref(),
            Some(expected.as_str())
        );
        assert_eq!(
            container
                .prompt_context
                .read()
                .await
                .working_directory
                .as_deref(),
            Some(expected.as_str())
        );
    }
}
