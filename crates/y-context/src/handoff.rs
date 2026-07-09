//! Handoff mechanism: generates structured state documents for session
//! transitions when the context window fills.
//!
//! Unlike compaction (which lossy-summarizes old messages in-place), handoff
//! generates a structured document (Goal / Constraints / Progress / Key
//! Decisions / Critical Context / Next Steps) and starts a fresh context,
//! preserving the decision rationale that flat compaction loses.
//!
//! Design reference: omp `packages/agent/src/compaction/compaction.ts:967-999`
//! and `packages/coding-agent/src/session/agent-session.ts:9999-10194`.

use std::sync::Arc;

use y_core::agent::{AgentDelegator, ContextStrategyHint};
use y_core::session::ChatMessageStore;
use y_core::types::SessionId;

/// Input structure for the handoff document generator subagent.
///
/// Includes the full conversation transcript (serialized) plus any existing
/// compaction summary, so the handoff generator can preserve prior decisions.
#[derive(serde::Serialize)]
struct HandoffInput {
    /// Serialized conversation messages (structured, not flat strings).
    messages: Vec<serde_json::Value>,
    /// Previous compaction summary if one exists in the transcript.
    previous_summary: Option<String>,
    /// Optional custom focus instructions (e.g., "focus on the API integration task").
    custom_focus: Option<String>,
}

/// Result of a handoff generation.
#[derive(Debug, Clone)]
pub struct HandoffResult {
    /// The generated handoff document text.
    pub document: String,
    /// Approximate tokens consumed.
    pub tokens_used: u64,
}

/// Handoff document generator using `AgentDelegator`.
///
/// Delegates to the `handoff-generator` built-in agent, which uses a
/// structured prompt template to produce a Goal/Constraints/Progress/
/// Decisions/Context/NextSteps document.
pub struct HandoffGenerator {
    delegator: Arc<dyn AgentDelegator>,
    max_retries: u32,
}

impl HandoffGenerator {
    /// Create with an agent delegator for subagent-based generation.
    pub fn new(delegator: Arc<dyn AgentDelegator>) -> Self {
        Self {
            delegator,
            max_retries: 2,
        }
    }

    /// Create with custom retry count.
    pub fn with_retries(delegator: Arc<dyn AgentDelegator>, max_retries: u32) -> Self {
        Self {
            delegator,
            max_retries,
        }
    }

    /// Generate a handoff document from the current session messages.
    ///
    /// Reads active messages from the `ChatMessageStore`, serializes them
    /// with structure preserved, and delegates to the `handoff-generator`
    /// subagent. Returns `None` if generation fails after all retries.
    pub async fn generate(
        &self,
        store: &dyn ChatMessageStore,
        session_id: &SessionId,
        previous_summary: Option<&str>,
        custom_focus: Option<&str>,
    ) -> Option<HandoffResult> {
        let messages = store
            .list_active(session_id)
            .await
            .ok()?
            .into_iter()
            .map(|m| {
                serde_json::json!({
                    "role": m.role,
                    "content": m.content,
                    "has_tool_calls": m.has_tool_calls,
                })
            })
            .collect::<Vec<_>>();

        if messages.len() < 2 {
            return None;
        }

        let input = HandoffInput {
            messages,
            previous_summary: previous_summary.map(String::from),
            custom_focus: custom_focus.map(String::from),
        };

        let input_value = serde_json::to_value(&input).ok()?;
        let session_uuid = uuid::Uuid::parse_str(&session_id.0).ok();

        for attempt in 0..self.max_retries {
            match self
                .delegator
                .delegate(
                    "handoff-generator",
                    input_value.clone(),
                    ContextStrategyHint::None,
                    session_uuid,
                )
                .await
            {
                Ok(output) if !output.text.trim().is_empty() => {
                    tracing::info!(
                        attempt,
                        tokens_used = output.tokens_used,
                        "handoff document generated successfully"
                    );
                    return Some(HandoffResult {
                        document: output.text,
                        tokens_used: output.tokens_used,
                    });
                }
                Ok(_) => {
                    tracing::warn!(attempt, "handoff-generator returned empty document");
                }
                Err(e) => {
                    tracing::warn!(attempt, error = %e, "handoff-generator delegation failed");
                }
            }
        }

        tracing::warn!(
            max_retries = self.max_retries,
            "all handoff-generator retries exhausted; falling back to compaction"
        );
        None
    }
}

/// Default handoff prompt template, used by the `handoff-generator` agent.
///
/// Loaded from `config/prompts/handoff_document.txt` at compile time.
/// Mirrors omp's `handoff-document.md` structure.
pub const HANDOFF_PROMPT_TEMPLATE: &str =
    include_str!("../../../config/prompts/handoff_document.txt");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_handoff_prompt_template_has_required_sections() {
        assert!(HANDOFF_PROMPT_TEMPLATE.contains("## Goal"));
        assert!(HANDOFF_PROMPT_TEMPLATE.contains("## Constraints & Preferences"));
        assert!(HANDOFF_PROMPT_TEMPLATE.contains("## Progress"));
        assert!(HANDOFF_PROMPT_TEMPLATE.contains("## Key Decisions"));
        assert!(HANDOFF_PROMPT_TEMPLATE.contains("## Critical Context"));
        assert!(HANDOFF_PROMPT_TEMPLATE.contains("## Next Steps"));
        // Must mention abandoned tasks — the core amnesia fix.
        assert!(HANDOFF_PROMPT_TEMPLATE.contains("ABANDONED"));
    }
}
