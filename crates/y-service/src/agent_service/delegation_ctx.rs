//! Ambient delegation-interaction context propagated via task-local.
//!
//! When the LLM calls the `Task` tool, the delegation runs inside the same
//! async task as the parent turn. This task-local carries the parent turn's
//! session identity and its progress / cancellation channels across the
//! `AgentDelegator` boundary (which cannot carry y-service types) so that a
//! delegated sub-agent executes *under the parent session*.
//!
//! Consequences:
//! - The permission gatekeeper resolves the parent session's permission and
//!   operation modes, so HITL prompts surface on the active session and
//!   "allow all for session" sticks to it.
//! - Sub-agent tool progress and cancellation are wired to the parent turn.
//!
//! Mirrors the `DIAGNOSTICS_CTX` task-local in `y-diagnostics`. Set (scoped)
//! at the `Task` interception in `tool_dispatch`; read in `ServiceAgentRunner`.

use tokio_util::sync::CancellationToken;
use y_core::types::SessionId;

use crate::agent_service::TurnEventSender;

/// Parent-turn interaction context shared with a delegated sub-agent.
#[derive(Clone)]
pub(crate) struct DelegationInteractionCtx {
    /// The parent (active) session the delegation should run under.
    pub session_id: SessionId,
    /// Progress channel of the parent turn, for streaming sub-agent events
    /// and surfacing HITL permission requests.
    pub progress: Option<TurnEventSender>,
    /// Cancellation token of the parent turn's execution subtree.
    pub cancel: Option<CancellationToken>,
}

tokio::task_local! {
    pub(crate) static DELEGATION_INTERACTION_CTX: DelegationInteractionCtx;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn ctx_is_visible_inside_scope() {
        let ctx = DelegationInteractionCtx {
            session_id: SessionId("sess-1".into()),
            progress: None,
            cancel: Some(CancellationToken::new()),
        };

        DELEGATION_INTERACTION_CTX
            .scope(ctx, async {
                let seen = DELEGATION_INTERACTION_CTX
                    .try_with(|c| c.session_id.clone())
                    .expect("ctx should be present inside scope");
                assert_eq!(seen, SessionId("sess-1".into()));
            })
            .await;
    }

    #[tokio::test]
    async fn ctx_is_absent_outside_scope() {
        let result = DELEGATION_INTERACTION_CTX.try_with(|c| c.session_id.clone());
        assert!(result.is_err(), "ctx must not leak outside its scope");
    }
}
