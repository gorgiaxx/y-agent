//! HITL (Human-in-the-Loop) escalation protocol.
//!
//! When a guardrail requires human approval (permission=Ask, risk threshold
//! exceeded), the HITL protocol pauses execution, sends a prompt to the user,
//! and waits for a response within a configurable timeout.
//!
//! Extended in Phase 3 to support:
//! - `ApproveAlways` / `DenyAlways` responses that persist rules
//! - Permission suggestions attached to the request

use tokio::sync::{mpsc, oneshot};
use tokio::time::{timeout, Duration};

use y_core::permission_types::PermissionRuleSource;

use crate::config::HitlConfig;
use crate::error::GuardrailError;

/// A request for human approval.
#[derive(Debug, Clone)]
pub struct HitlRequest {
    /// Unique ID for this escalation.
    pub request_id: String,
    /// What tool/action requires approval.
    pub tool_name: String,
    /// Why approval is needed.
    pub reason: String,
    /// Risk score (if available).
    pub risk_score: Option<f32>,
    /// Human-readable context for the user.
    pub context: String,
    /// Suggested "always" options the UI can present to the user.
    ///
    /// For example: "Always allow `FileRead`", "Always deny `ShellExec`(rm -rf:*)".
    /// Each suggestion maps to a `PermissionUpdate` that can be persisted.
    pub suggestions: Vec<String>,
}

/// User's response to an HITL request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HitlResponse {
    /// User approves the action (this time only).
    Approve,
    /// User approves the action and persists an "always allow" rule.
    ApproveAlways {
        /// Where to persist the rule (global or project).
        destination: PermissionRuleSource,
    },
    /// User denies the action, with optional reason.
    Deny { reason: String },
    /// User denies the action and persists an "always deny" rule.
    DenyAlways {
        /// Where to persist the rule (global or project).
        destination: PermissionRuleSource,
        /// Reason for denial.
        reason: String,
    },
}

impl HitlResponse {
    /// Whether this response approves the action.
    pub fn is_approved(&self) -> bool {
        matches!(
            self,
            HitlResponse::Approve | HitlResponse::ApproveAlways { .. }
        )
    }

    /// Whether this response requests rule persistence.
    pub fn is_always(&self) -> bool {
        matches!(
            self,
            HitlResponse::ApproveAlways { .. } | HitlResponse::DenyAlways { .. }
        )
    }
}

/// The HITL protocol handles escalation between the agent and a human.
///
/// Communication uses a channel pair:
/// - The protocol sends `HitlRequest` to the user-facing handler
/// - The user-facing handler sends `HitlResponse` back via a oneshot channel
#[derive(Debug)]
pub struct HitlProtocol {
    config: HitlConfig,
    /// Sender for outgoing HITL requests (to the user-facing handler).
    request_tx: mpsc::Sender<(HitlRequest, oneshot::Sender<HitlResponse>)>,
}

/// Handle for the user-facing side that receives and responds to HITL requests.
#[derive(Debug)]
pub struct HitlHandler {
    /// Receiver for incoming HITL requests.
    pub request_rx: mpsc::Receiver<(HitlRequest, oneshot::Sender<HitlResponse>)>,
}

impl HitlProtocol {
    /// Create a new HITL protocol pair (protocol, handler).
    ///
    /// The `HitlHandler` should be given to the user-facing system (CLI, UI, etc.)
    /// to receive and respond to escalation requests.
    pub fn new(config: HitlConfig) -> (Self, HitlHandler) {
        let (request_tx, request_rx) = mpsc::channel(16);

        let protocol = Self { config, request_tx };
        let handler = HitlHandler { request_rx };

        (protocol, handler)
    }

    /// Escalate an action for human approval.
    ///
    /// Returns the full `HitlResponse` so the caller can handle
    /// `ApproveAlways`/`DenyAlways` rule persistence.
    pub async fn escalate_full(
        &self,
        request: HitlRequest,
    ) -> Result<HitlResponse, GuardrailError> {
        let (response_tx, response_rx) = oneshot::channel();

        self.request_tx
            .send((request, response_tx))
            .await
            .map_err(|_| GuardrailError::Other {
                message: "HITL handler disconnected".to_string(),
            })?;

        let duration = Duration::from_millis(self.config.timeout_ms);

        match timeout(duration, response_rx).await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(_)) => Err(GuardrailError::Other {
                message: "HITL response channel dropped".to_string(),
            }),
            Err(_) => Err(GuardrailError::HitlTimeout {
                timeout_ms: self.config.timeout_ms,
            }),
        }
    }

    /// Escalate an action for human approval (simplified API).
    ///
    /// Returns `Ok(())` if approved, `Err(GuardrailError)` if denied or timed out.
    /// Use `escalate_full()` to get the full response for rule persistence.
    pub async fn escalate(&self, request: HitlRequest) -> Result<(), GuardrailError> {
        let response = self.escalate_full(request).await?;

        if response.is_approved() {
            Ok(())
        } else {
            let reason = match response {
                HitlResponse::Deny { reason } | HitlResponse::DenyAlways { reason, .. } => reason,
                _ => "denied".to_string(),
            };
            Err(GuardrailError::HitlDenied { reason })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quick_config() -> HitlConfig {
        HitlConfig { timeout_ms: 100 }
    }

    fn test_request() -> HitlRequest {
        HitlRequest {
            request_id: "req-1".to_string(),
            tool_name: "ShellExec".to_string(),
            reason: "dangerous tool".to_string(),
            risk_score: Some(0.9),
            context: "executing `rm -rf /tmp/test`".to_string(),
            suggestions: vec!["Always allow ShellExec".to_string()],
        }
    }

    /// T-GUARD-005-01: HITL escalation pauses execution (request is sent).
    #[tokio::test]
    async fn test_hitl_escalation_pauses_execution() {
        let (protocol, mut handler) = HitlProtocol::new(quick_config());

        // Spawn escalation in background
        let escalate_task = tokio::spawn(async move { protocol.escalate(test_request()).await });

        // Verify request arrives at handler (proving execution is "paused" waiting)
        let (request, response_tx) = handler
            .request_rx
            .recv()
            .await
            .expect("should receive request");

        assert_eq!(request.tool_name, "ShellExec");

        // Approve so the task completes
        response_tx.send(HitlResponse::Approve).ok();
        let result = escalate_task.await.unwrap();
        assert!(result.is_ok());
    }

    /// T-GUARD-005-02: User approves → execution continues.
    #[tokio::test]
    async fn test_hitl_user_approves() {
        let (protocol, mut handler) = HitlProtocol::new(quick_config());

        let escalate_task = tokio::spawn(async move { protocol.escalate(test_request()).await });

        let (_request, response_tx) = handler.request_rx.recv().await.unwrap();
        response_tx.send(HitlResponse::Approve).ok();

        let result = escalate_task.await.unwrap();
        assert!(result.is_ok(), "approved escalation should succeed");
    }

    /// T-GUARD-005-03: User denies → execution aborted.
    #[tokio::test]
    async fn test_hitl_user_denies() {
        let (protocol, mut handler) = HitlProtocol::new(quick_config());

        let escalate_task = tokio::spawn(async move { protocol.escalate(test_request()).await });

        let (_request, response_tx) = handler.request_rx.recv().await.unwrap();
        response_tx
            .send(HitlResponse::Deny {
                reason: "not safe".to_string(),
            })
            .ok();

        let result = escalate_task.await.unwrap();
        assert!(result.is_err());
        match result.unwrap_err() {
            GuardrailError::HitlDenied { reason } => {
                assert_eq!(reason, "not safe");
            }
            other => panic!("expected HitlDenied, got {other:?}"),
        }
    }

    /// T-GUARD-005-04: No response within timeout → defaults to deny.
    #[tokio::test]
    async fn test_hitl_timeout() {
        let config = HitlConfig { timeout_ms: 50 };
        let (protocol, _handler) = HitlProtocol::new(config);

        let result = protocol.escalate(test_request()).await;
        assert!(result.is_err());
        assert!(
            matches!(result.unwrap_err(), GuardrailError::HitlTimeout { .. }),
            "should timeout"
        );
    }
}
