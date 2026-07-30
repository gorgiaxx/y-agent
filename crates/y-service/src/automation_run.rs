//! Service-owned preparation for headless automation and agent-to-agent runs.

use std::path::PathBuf;

use y_core::provider::ThinkingConfig;
use y_core::session::{CreateSessionOptions, SessionNode, SessionType};
use y_core::types::AgentId;

use crate::chat::{ChatService, PrepareTurnRequest, PreparedTurn};
use crate::chat_types::OperationMode;
use crate::{ServiceContainer, SessionService};

/// Inputs shared by headless CLI and future A2A transports.
#[derive(Debug, Default)]
pub struct AutomationRunRequest {
    pub session_target: Option<String>,
    pub continue_last: bool,
    /// Searchable title assigned when creating a session.
    pub session_name: Option<String>,
    /// `None` selects ordinary chat for a new session and restores the binding
    /// for a resumed session.
    pub agent_id: Option<String>,
    pub user_input: String,
    pub workspace: PathBuf,
    pub provider_id: Option<String>,
    pub model: Option<String>,
    pub skills: Option<Vec<String>>,
    pub knowledge_collections: Option<Vec<String>>,
    pub thinking: Option<ThinkingConfig>,
    pub plan_mode: Option<String>,
    pub operation_mode: Option<OperationMode>,
}

#[derive(Debug)]
pub struct PreparedAutomationRun {
    pub turn: PreparedTurn,
    pub session_reference: String,
    pub resumed: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum AutomationRunError {
    #[error("prompt cannot be empty")]
    EmptyPrompt,
    #[error("--session and --continue cannot be combined")]
    ConflictingResumeSelectors,
    #[error("session name is only valid when creating a new session")]
    SessionNameOnResume,
    #[error("session name must contain 1 to 120 characters")]
    InvalidSessionName,
    #[error("session not found: {0}")]
    SessionNotFound(String),
    #[error("agent not found: {0}")]
    AgentNotFound(String),
    #[error("agent '{0}' is not user-callable")]
    AgentNotUserCallable(String),
    #[error("agent selection is creation-only; resumed session is bound to {bound}")]
    AgentBindingConflict { bound: String },
    #[error("failed to resolve or create session: {0}")]
    Session(String),
    #[error("failed to prepare turn: {0}")]
    Prepare(String),
}

pub struct AutomationRunService;

impl AutomationRunService {
    pub async fn prepare(
        container: &ServiceContainer,
        request: AutomationRunRequest,
    ) -> Result<PreparedAutomationRun, AutomationRunError> {
        if request.user_input.trim().is_empty() {
            return Err(AutomationRunError::EmptyPrompt);
        }
        if request.session_target.is_some() && request.continue_last {
            return Err(AutomationRunError::ConflictingResumeSelectors);
        }
        if request.session_name.is_some()
            && (request.session_target.is_some() || request.continue_last)
        {
            return Err(AutomationRunError::SessionNameOnResume);
        }

        let (session, resumed) = Self::resolve_session(container, &request).await?;
        Self::validate_resumed_binding(&session, request.agent_id.as_deref(), resumed)?;

        let mut turn = ChatService::prepare_turn(
            container,
            PrepareTurnRequest {
                session_id: Some(session.id.clone()),
                user_input: request.user_input,
                provider_id: request.provider_id,
                skills: request.skills,
                knowledge_collections: request.knowledge_collections,
                thinking: request.thinking,
                plan_mode: request.plan_mode,
                operation_mode: request.operation_mode,
                ..PrepareTurnRequest::default()
            },
        )
        .await
        .map_err(|error| AutomationRunError::Prepare(error.to_string()))?;

        if let Some(model) = request.model {
            turn.preferred_models = vec![model];
        }
        turn.working_directory = Some(
            crate::workspace::canonical_workspace_path(&request.workspace)
                .map_err(|error| AutomationRunError::Session(error.to_string()))?,
        );
        let session_reference = SessionService::public_session_reference(&turn.session_id);

        Ok(PreparedAutomationRun {
            turn,
            session_reference,
            resumed,
        })
    }

    async fn resolve_session(
        container: &ServiceContainer,
        request: &AutomationRunRequest,
    ) -> Result<(SessionNode, bool), AutomationRunError> {
        if let Some(target) = request.session_target.as_deref() {
            let session = SessionService::resolve_resume_target(
                &container.session_manager,
                &request.workspace,
                None,
                target,
            )
            .await
            .map_err(|error| AutomationRunError::Session(error.to_string()))?
            .ok_or_else(|| AutomationRunError::SessionNotFound(target.to_string()))?;
            return Ok((session, true));
        }

        if request.continue_last {
            let session = SessionService::list_resumable_sessions(
                &container.session_manager,
                &request.workspace,
                None,
            )
            .await
            .map_err(|error| AutomationRunError::Session(error.to_string()))?
            .into_iter()
            .next()
            .ok_or_else(|| AutomationRunError::SessionNotFound("most recent".into()))?;
            return Ok((session, true));
        }

        let agent_id = Self::validate_new_agent(container, request.agent_id.as_deref()).await?;
        let title = Self::session_title(request.session_name.as_deref(), &request.user_input)?;
        let session = SessionService::create_session(
            &container.session_manager,
            CreateSessionOptions {
                parent_id: None,
                session_type: SessionType::Main,
                agent_id,
                title: Some(title),
            },
            &request.workspace,
        )
        .await
        .map_err(|error| AutomationRunError::Session(error.to_string()))?;
        Ok((session, false))
    }

    fn session_title(requested: Option<&str>, prompt: &str) -> Result<String, AutomationRunError> {
        if let Some(requested) = requested {
            let title = requested.trim();
            let length = title.chars().count();
            if length == 0 || length > 120 {
                return Err(AutomationRunError::InvalidSessionName);
            }
            return Ok(title.to_string());
        }

        let normalized = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
        let mut title = normalized.chars().take(80).collect::<String>();
        if normalized.chars().count() > 80 {
            title.push_str("...");
        }
        Ok(title)
    }

    async fn validate_new_agent(
        container: &ServiceContainer,
        requested: Option<&str>,
    ) -> Result<Option<AgentId>, AutomationRunError> {
        let Some(agent_id) = requested else {
            return Ok(None);
        };
        let registry = container.agent_registry.lock().await;
        let definition = registry
            .get(agent_id)
            .ok_or_else(|| AutomationRunError::AgentNotFound(agent_id.to_string()))?;
        if !definition.user_callable {
            return Err(AutomationRunError::AgentNotUserCallable(
                agent_id.to_string(),
            ));
        }
        Ok(Some(AgentId::from_string(agent_id)))
    }

    fn validate_resumed_binding(
        session: &SessionNode,
        requested: Option<&str>,
        resumed: bool,
    ) -> Result<(), AutomationRunError> {
        let Some(requested) = requested.filter(|_| resumed) else {
            return Ok(());
        };
        if session.agent_id.as_ref().map(AgentId::as_str) == Some(requested) {
            return Ok(());
        }
        let bound = session
            .agent_id
            .as_ref()
            .map_or_else(|| "chat".to_string(), ToString::to_string);
        Err(AutomationRunError::AgentBindingConflict { bound })
    }
}
