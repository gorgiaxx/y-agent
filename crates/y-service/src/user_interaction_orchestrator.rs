//! User interaction orchestrator for the `AskUser` tool.
//!
//! Intercepts `AskUser` tool calls in the agent execution loop, emits a
//! [`TurnEvent::UserInteractionRequest`] so the presentation layer can
//! render the questions, and blocks until the user answers via a `oneshot`
//! channel.
//!
//! For non-interactive contexts (bot platforms, sub-agents without a progress
//! channel), the orchestrator falls back to formatting the questions as
//! plain text in the tool result so the LLM can include them in its reply.
//!
//! ## Flow
//!
//! ```text
//! LLM -> AskUser(questions)
//!     -> orchestrator validates arguments
//!     -> emits TurnEvent::UserInteractionRequest { interaction_id, questions }
//!     -> awaits oneshot::Receiver (with 3-min timeout)
//!     -> formats answers via AskUserTool::format_answers_for_llm()
//!     -> returns ToolOutput to the LLM
//! ```

use std::time::Duration;

use y_core::tool::{ToolError, ToolOutput};
use y_tools::builtin::user_interaction::AskUserTool;

use crate::chat::{PendingInteractions, TurnEvent, TurnEventSender};

/// Timeout for awaiting user answers (3 minutes).
const INTERACTION_TIMEOUT: Duration = Duration::from_secs(180);

/// Orchestrator that handles `AskUser` tool calls.
///
/// This is a static-methods-only type (no instances), following the same
/// pattern as [`crate::tool_search_orchestrator::ToolSearchOrchestrator`].
pub struct UserInteractionOrchestrator;

impl UserInteractionOrchestrator {
    /// Handle an `AskUser` tool call.
    ///
    /// When a progress channel and `pending_interactions` map are available
    /// (interactive GUI/CLI sessions), this method:
    /// 1. Validates the arguments via `AskUserTool`
    /// 2. Emits `TurnEvent::UserInteractionRequest`
    /// 3. Awaits the user's answer with a 3-minute timeout
    /// 4. Returns the formatted answer as `ToolOutput`
    ///
    /// When no progress channel is available (bot platforms, sub-agents),
    /// falls back to returning the questions as plain text for the LLM
    /// to include in its response.
    pub async fn handle(
        arguments: &serde_json::Value,
        pending_interactions: &PendingInteractions,
        progress: Option<&TurnEventSender>,
    ) -> Result<ToolOutput, ToolError> {
        // 1. Extract and validate questions.
        let questions =
            arguments
                .get("questions")
                .cloned()
                .ok_or_else(|| ToolError::ValidationError {
                    message: "missing required 'questions' field".into(),
                })?;

        AskUserTool::validate_questions_public(&questions)?;

        // 2. If no progress channel, fall back to plain-text rendering.
        let Some(tx) = progress else {
            return Ok(Self::format_as_plain_text(&questions));
        };

        // 3. Generate interaction ID and create oneshot channel.
        let interaction_id = uuid::Uuid::new_v4().to_string();

        let (answer_tx, answer_rx) = tokio::sync::oneshot::channel();

        // Register the answer sender.
        {
            let mut map = pending_interactions.lock().await;
            map.insert(interaction_id.clone(), answer_tx);
        }

        // 4. Emit the event to the presentation layer.
        let _ = tx.send(TurnEvent::UserInteractionRequest {
            interaction_id: interaction_id.clone(),
            questions: questions.clone(),
        });

        // 5. Await user answer with timeout.
        let answer = match tokio::time::timeout(INTERACTION_TIMEOUT, answer_rx).await {
            Ok(Ok(answer)) => answer,
            Ok(Err(_)) => {
                // Channel closed without answer (e.g. UI dismissed).
                Self::cleanup_pending(&interaction_id, pending_interactions).await;
                return Ok(Self::user_declined_response());
            }
            Err(_) => {
                // Timeout expired.
                Self::cleanup_pending(&interaction_id, pending_interactions).await;
                return Ok(Self::user_timeout_response());
            }
        };

        // 6. Format the answer for the LLM.
        Self::format_answer_output(&questions, &answer)
    }

    /// Deliver a user's answer to the pending interaction.
    ///
    /// Called by the presentation layer (Tauri `chat_answer_question` command)
    /// to unblock the orchestrator's awaiting `oneshot::Receiver`.
    ///
    /// Returns `true` if the answer was delivered successfully.
    pub async fn deliver_answer(
        interaction_id: &str,
        answer: serde_json::Value,
        pending_interactions: &PendingInteractions,
    ) -> bool {
        let sender = {
            let mut map = pending_interactions.lock().await;
            map.remove(interaction_id)
        };

        match sender {
            Some(tx) => tx.send(answer).is_ok(),
            None => {
                tracing::warn!(
                    interaction_id = %interaction_id,
                    "deliver_answer: no pending interaction found (may have timed out)"
                );
                false
            }
        }
    }

    /// Remove a pending interaction entry without delivering an answer.
    async fn cleanup_pending(interaction_id: &str, pending: &PendingInteractions) {
        let mut map = pending.lock().await;
        map.remove(interaction_id);
    }

    /// Build a `ToolOutput` for when the user declines or dismisses the dialog.
    fn user_declined_response() -> ToolOutput {
        ToolOutput {
            content: serde_json::json!({
                "status": "declined",
                "message": "The user dismissed the question without answering. \
                    Please proceed using your best judgment."
            }),
            success: true,
            warnings: vec![],
            metadata: serde_json::Value::Null,
        }
    }

    /// Build a `ToolOutput` for when the interaction times out.
    fn user_timeout_response() -> ToolOutput {
        ToolOutput {
            content: serde_json::json!({
                "status": "timeout",
                "message": "The user did not respond within the time limit. \
                    Please proceed using your best judgment."
            }),
            success: true,
            warnings: vec![],
            metadata: serde_json::Value::Null,
        }
    }

    /// Format validated answers from the user into a `ToolOutput` for the LLM.
    fn format_answer_output(
        questions: &serde_json::Value,
        answer: &serde_json::Value,
    ) -> Result<ToolOutput, ToolError> {
        let answers = answer
            .get("answers")
            .and_then(|v| v.as_object())
            .ok_or_else(|| ToolError::RuntimeError {
                name: "AskUser".into(),
                message: "answer payload missing 'answers' object".into(),
            })?;

        let formatted = AskUserTool::format_answers_for_llm(answers);

        Ok(ToolOutput {
            content: serde_json::json!({
                "status": "answered",
                "questions": questions,
                "answers": answer.get("answers"),
                "formatted": formatted,
            }),
            success: true,
            warnings: vec![],
            metadata: serde_json::Value::Null,
        })
    }

    /// Fallback: format questions as numbered plain text for non-interactive
    /// contexts (bot platforms, sub-agents).
    ///
    /// The LLM should include this text in its response so the user can
    /// answer naturally in a follow-up message.
    fn format_as_plain_text(questions: &serde_json::Value) -> ToolOutput {
        use std::fmt::Write;
        let empty = Vec::new();
        let arr = questions.as_array().unwrap_or(&empty);
        let mut text = String::from("I have questions for you:\n\n");

        for (qi, q) in arr.iter().enumerate() {
            let question_text = q
                .get("question")
                .and_then(|v| v.as_str())
                .unwrap_or("(question)");
            let multi = q
                .get("multi_select")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            let _ = writeln!(
                text,
                "[{}] {}{}",
                qi + 1,
                question_text,
                if multi { " (select multiple)" } else { "" }
            );

            if let Some(options) = q.get("options").and_then(|v| v.as_array()) {
                for (oi, opt) in options.iter().enumerate() {
                    let label = opt.as_str().unwrap_or("?");
                    let letter = (b'A' + u8::try_from(oi).unwrap_or(0)) as char;
                    let _ = writeln!(text, "    {letter}) {label}");
                }
            }

            text.push_str("    *) Other - Type your own answer\n\n");
        }

        text.push_str("Please reply with your choice (e.g., \"1A\" or \"1: chrono\").\n");

        ToolOutput {
            content: serde_json::json!({
                "status": "pending_bot",
                "message": text,
                "questions": questions,
            }),
            success: true,
            warnings: vec![],
            metadata: serde_json::Value::Null,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_questions() -> serde_json::Value {
        serde_json::json!([{
            "question": "Which library?",
            "options": ["chrono", "time"]
        }])
    }

    #[test]
    fn test_format_as_plain_text() {
        let questions = sample_questions();
        let output = UserInteractionOrchestrator::format_as_plain_text(&questions);
        assert!(output.success);
        let msg = output.content["message"].as_str().unwrap();
        assert!(msg.contains("[1] Which library?"));
        assert!(msg.contains("A) chrono"));
        assert!(msg.contains("B) time"));
        assert!(msg.contains("*) Other"));
    }

    #[test]
    fn test_format_answer_output_valid() {
        let questions = sample_questions();
        let answer = serde_json::json!({
            "answers": {
                "Which library?": "chrono"
            }
        });
        let result =
            UserInteractionOrchestrator::format_answer_output(&questions, &answer).unwrap();
        assert!(result.success);
        assert_eq!(result.content["status"], "answered");
        let formatted = result.content["formatted"].as_str().unwrap();
        assert!(formatted.contains("chrono"));
    }

    #[test]
    fn test_format_answer_output_missing_answers() {
        let questions = sample_questions();
        let answer = serde_json::json!({});
        let result = UserInteractionOrchestrator::format_answer_output(&questions, &answer);
        assert!(result.is_err());
    }

    #[test]
    fn test_user_declined_response() {
        let output = UserInteractionOrchestrator::user_declined_response();
        assert!(output.success);
        assert_eq!(output.content["status"], "declined");
    }

    #[test]
    fn test_user_timeout_response() {
        let output = UserInteractionOrchestrator::user_timeout_response();
        assert!(output.success);
        assert_eq!(output.content["status"], "timeout");
    }

    #[tokio::test]
    async fn test_deliver_answer_success() {
        let pending: PendingInteractions =
            std::sync::Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));

        let (tx, rx) = tokio::sync::oneshot::channel();
        {
            let mut map = pending.lock().await;
            map.insert("test-id".into(), tx);
        }

        let answer = serde_json::json!({"answers": {"Q": "A"}});
        let delivered =
            UserInteractionOrchestrator::deliver_answer("test-id", answer.clone(), &pending).await;
        assert!(delivered);

        let received = rx.await.unwrap();
        assert_eq!(received, answer);
    }

    #[tokio::test]
    async fn test_deliver_answer_not_found() {
        let pending: PendingInteractions =
            std::sync::Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));

        let delivered =
            UserInteractionOrchestrator::deliver_answer("missing", serde_json::json!({}), &pending)
                .await;
        assert!(!delivered);
    }

    #[tokio::test]
    async fn test_handle_no_progress_falls_back() {
        let pending: PendingInteractions =
            std::sync::Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));

        let args = serde_json::json!({
            "questions": [{
                "question": "Pick one?",
                "options": ["A", "B"]
            }]
        });

        let result = UserInteractionOrchestrator::handle(&args, &pending, None)
            .await
            .unwrap();
        assert!(result.success);
        assert_eq!(result.content["status"], "pending_bot");
    }

    #[tokio::test]
    async fn test_handle_with_progress_delivers_answer() {
        let pending: PendingInteractions =
            std::sync::Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        let args = serde_json::json!({
            "questions": [{
                "question": "Pick one?",
                "options": ["A", "B"]
            }]
        });

        let pending_clone = pending.clone();
        let handle = tokio::spawn(async move {
            UserInteractionOrchestrator::handle(&args, &pending_clone, Some(&tx)).await
        });

        // Wait for the event.
        let event = rx.recv().await.unwrap();
        let interaction_id = match event {
            TurnEvent::UserInteractionRequest { interaction_id, .. } => interaction_id,
            other => panic!("unexpected event: {other:?}"),
        };

        // Deliver answer.
        let answer = serde_json::json!({
            "answers": {"Pick one?": "A"}
        });
        UserInteractionOrchestrator::deliver_answer(&interaction_id, answer, &pending).await;

        let result = handle.await.unwrap().unwrap();
        assert!(result.success);
        assert_eq!(result.content["status"], "answered");
    }
}
