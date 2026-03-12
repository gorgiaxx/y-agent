//! Context injection: strategy-based context preparation for delegations.
//!
//! Design reference: multi-agent-design.md §Context Sharing Strategies
//!
//! Each strategy controls how much conversation history a delegated agent
//! receives:
//! - `None`: only the delegation prompt
//! - `Summary`: LLM-generated summary of conversation context
//! - `Filtered`: messages filtered by role, recency, or keyword
//! - `Full`: complete conversation history (truncated to token limit)

use crate::agent::definition::ContextStrategy;

// ---------------------------------------------------------------------------
// Message type (lightweight, for context injection purposes)
// ---------------------------------------------------------------------------

/// A simplified message for context injection.
///
/// This is intentionally decoupled from `y-core::Message` to avoid
/// tight coupling. Integration will map between these types.
#[derive(Debug, Clone)]
pub struct ContextMessage {
    /// Role: "system", "user", "assistant", "tool".
    pub role: String,
    /// Message content.
    pub content: String,
    /// Approximate token count for budget tracking.
    pub token_estimate: usize,
}

impl ContextMessage {
    /// Create a new context message.
    pub fn new(role: &str, content: &str) -> Self {
        // Rough estimate: ~4 chars per token
        let token_estimate = content.len() / 4 + 1;
        Self {
            role: role.to_string(),
            content: content.to_string(),
            token_estimate,
        }
    }

    /// Create a system message.
    pub fn system(content: &str) -> Self {
        Self::new("system", content)
    }

    /// Create a user message.
    pub fn user(content: &str) -> Self {
        Self::new("user", content)
    }

    /// Create an assistant message.
    pub fn assistant(content: &str) -> Self {
        Self::new("assistant", content)
    }
}

// ---------------------------------------------------------------------------
// Context injection
// ---------------------------------------------------------------------------

/// Apply a context strategy to produce the messages for a delegated agent.
///
/// # Arguments
///
/// * `strategy` - The context sharing strategy to apply.
/// * `delegation_prompt` - The task/prompt being delegated.
/// * `conversation` - The parent conversation history.
/// * `max_tokens` - Maximum token budget for context.
///
/// # Returns
///
/// A vector of `ContextMessage`s to provide to the delegated agent.
pub fn apply_context(
    strategy: ContextStrategy,
    delegation_prompt: &str,
    conversation: &[ContextMessage],
    max_tokens: usize,
) -> Vec<ContextMessage> {
    match strategy {
        ContextStrategy::None => apply_none(delegation_prompt),
        ContextStrategy::Summary => apply_summary(delegation_prompt, conversation, max_tokens),
        ContextStrategy::Filtered => apply_filtered(delegation_prompt, conversation, max_tokens),
        ContextStrategy::Full => apply_full(delegation_prompt, conversation, max_tokens),
    }
}

/// None strategy: only the delegation prompt, no parent context.
fn apply_none(delegation_prompt: &str) -> Vec<ContextMessage> {
    vec![ContextMessage::user(delegation_prompt)]
}

/// Summary strategy: generates a summary of the conversation.
///
/// In the full implementation, this would call the LLM to summarize.
/// For now, it creates a synthetic summary from the last few messages.
fn apply_summary(
    delegation_prompt: &str,
    conversation: &[ContextMessage],
    max_tokens: usize,
) -> Vec<ContextMessage> {
    let mut result = Vec::new();

    if !conversation.is_empty() {
        // Build a summary from the conversation
        let mut summary_parts: Vec<String> = Vec::new();
        let mut token_budget = max_tokens.saturating_sub(delegation_prompt.len() / 4 + 50);

        for msg in conversation.iter().rev() {
            if token_budget < msg.token_estimate {
                break;
            }
            summary_parts.push(format!("[{}]: {}", msg.role, truncate(&msg.content, 200)));
            token_budget = token_budget.saturating_sub(msg.token_estimate);
        }

        summary_parts.reverse();

        if !summary_parts.is_empty() {
            let summary = format!(
                "Context summary from parent conversation:\n{}",
                summary_parts.join("\n")
            );
            result.push(ContextMessage::system(&summary));
        }
    }

    result.push(ContextMessage::user(delegation_prompt));
    result
}

/// Filtered strategy: select messages by role and recency.
///
/// Prioritizes: recent messages, user/assistant messages, messages matching
/// the delegation topic.
fn apply_filtered(
    delegation_prompt: &str,
    conversation: &[ContextMessage],
    max_tokens: usize,
) -> Vec<ContextMessage> {
    let mut result = Vec::new();
    let prompt_tokens = delegation_prompt.len() / 4 + 1;
    let mut remaining_budget = max_tokens.saturating_sub(prompt_tokens);

    // Filter: user and assistant messages only, most recent first
    let relevant_messages: Vec<&ContextMessage> = conversation
        .iter()
        .rev()
        .filter(|m| m.role == "user" || m.role == "assistant")
        .collect();

    let mut selected: Vec<ContextMessage> = Vec::new();

    for msg in relevant_messages {
        if remaining_budget < msg.token_estimate {
            break;
        }
        selected.push(msg.clone());
        remaining_budget = remaining_budget.saturating_sub(msg.token_estimate);
    }

    // Reverse to restore chronological order
    selected.reverse();

    if !selected.is_empty() {
        result.push(ContextMessage::system(
            "Relevant messages from parent conversation:",
        ));
        result.extend(selected);
    }

    result.push(ContextMessage::user(delegation_prompt));
    result
}

/// Full strategy: complete conversation history, truncated to fit budget.
fn apply_full(
    delegation_prompt: &str,
    conversation: &[ContextMessage],
    max_tokens: usize,
) -> Vec<ContextMessage> {
    let prompt_tokens = delegation_prompt.len() / 4 + 1;
    let mut remaining_budget = max_tokens.saturating_sub(prompt_tokens);

    // Take as many messages as fit, starting from the most recent
    let mut selected: Vec<ContextMessage> = Vec::new();
    for msg in conversation.iter().rev() {
        if remaining_budget < msg.token_estimate {
            break;
        }
        selected.push(msg.clone());
        remaining_budget = remaining_budget.saturating_sub(msg.token_estimate);
    }

    selected.reverse();
    selected.push(ContextMessage::user(delegation_prompt));
    selected
}

/// Truncate a string to a maximum character length, adding "..." if truncated.
fn truncate(s: &str, max_chars: usize) -> String {
    if s.len() <= max_chars {
        s.to_string()
    } else {
        format!("{}...", &s[..max_chars])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_conversation() -> Vec<ContextMessage> {
        vec![
            ContextMessage::user("What is Rust?"),
            ContextMessage::assistant("Rust is a systems programming language."),
            ContextMessage::user("Tell me about its ownership model."),
            ContextMessage::assistant(
                "Rust's ownership model ensures memory safety without a garbage collector.",
            ),
            ContextMessage::user("How does borrowing work?"),
            ContextMessage::assistant(
                "Borrowing lets you reference data without taking ownership. \
                 There are immutable and mutable borrows.",
            ),
        ]
    }

    /// T-MA-R3-04: `NoneStrategy` returns only delegation prompt.
    #[test]
    fn test_none_strategy_only_prompt() {
        let result = apply_context(
            ContextStrategy::None,
            "Analyze the code",
            &sample_conversation(),
            4096,
        );

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].role, "user");
        assert_eq!(result[0].content, "Analyze the code");
    }

    /// T-MA-R3-05: `FilteredStrategy` filters by recency.
    #[test]
    fn test_filtered_strategy_recency() {
        let conv = sample_conversation();

        // Test with large budget first: should include all messages
        let result_full = apply_context(
            ContextStrategy::Filtered,
            "Summarize the discussion",
            &conv,
            10000,
        );
        let last = result_full.last().unwrap();
        assert_eq!(last.role, "user");
        assert_eq!(last.content, "Summarize the discussion");

        // Test with small budget: should have fewer conversation messages
        let result_small = apply_context(
            ContextStrategy::Filtered,
            "Summarize the discussion",
            &conv,
            60,
        );
        // Should always end with delegation prompt
        let last_small = result_small.last().unwrap();
        assert_eq!(last_small.content, "Summarize the discussion");

        // Smaller budget should yield fewer total messages
        assert!(result_small.len() <= result_full.len());
    }

    /// T-MA-R3-06: `FullStrategy` truncates to `max_tokens` limit.
    #[test]
    fn test_full_strategy_truncates() {
        let conv = sample_conversation();
        let result_large = apply_context(
            ContextStrategy::Full,
            "Continue the discussion",
            &conv,
            10000, // large budget
        );

        // Should include all messages + delegation prompt
        assert_eq!(result_large.len(), conv.len() + 1);

        let result_small = apply_context(
            ContextStrategy::Full,
            "Continue the discussion",
            &conv,
            50, // very small budget
        );

        // Should include fewer messages due to budget
        assert!(result_small.len() < result_large.len());
        // Should always end with the delegation prompt
        let last = result_small.last().unwrap();
        assert_eq!(last.role, "user");
        assert_eq!(last.content, "Continue the discussion");
    }

    /// Summary strategy includes context summary.
    #[test]
    fn test_summary_strategy() {
        let conv = sample_conversation();
        let result = apply_context(ContextStrategy::Summary, "Draft a report", &conv, 4096);

        // Should have a system summary + delegation prompt
        assert!(result.len() >= 2);
        let system_msg = result.iter().find(|m| m.role == "system");
        assert!(system_msg.is_some());
        assert!(system_msg.unwrap().content.contains("Context summary"));

        let last = result.last().unwrap();
        assert_eq!(last.content, "Draft a report");
    }

    /// Empty conversation with None strategy.
    #[test]
    fn test_none_strategy_empty_conversation() {
        let result = apply_context(ContextStrategy::None, "Do something", &[], 4096);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].content, "Do something");
    }

    /// Empty conversation with Full strategy.
    #[test]
    fn test_full_strategy_empty_conversation() {
        let result = apply_context(ContextStrategy::Full, "Do something", &[], 4096);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].content, "Do something");
    }
}
