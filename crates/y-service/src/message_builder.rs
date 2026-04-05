//! Shared message building utilities.
//!
//! Extracted from `ChatService` and `AgentService` to avoid duplication.
//! Both services delegate to the free function [`build_chat_messages`].

use y_context::{AssembledContext, ContextCategory};
use y_core::types::Message;

/// Build LLM messages by prepending a system prompt from assembled context.
///
/// Filters context items by category (`SystemPrompt`, Skills, Knowledge, Tools),
/// joins their content into a single system message, and prepends it to the
/// conversation history.
pub fn build_chat_messages(assembled: &AssembledContext, history: &[Message]) -> Vec<Message> {
    use y_core::types::Role;

    let system_parts: Vec<&str> = assembled
        .items
        .iter()
        .filter(|item| {
            matches!(
                item.category,
                ContextCategory::SystemPrompt
                    | ContextCategory::Skills
                    | ContextCategory::Knowledge
                    | ContextCategory::Tools
            )
        })
        .map(|item| item.content.as_str())
        .collect();

    let mut messages = Vec::with_capacity(history.len() + 1);

    if !system_parts.is_empty() {
        let system_content = system_parts.join("\n\n");
        messages.push(Message {
            message_id: y_core::types::generate_message_id(),
            role: Role::System,
            content: system_content,
            tool_call_id: None,
            tool_calls: vec![],
            timestamp: y_core::types::now(),
            metadata: serde_json::Value::Null,
        });
    }

    messages.extend_from_slice(history);
    messages
}

#[cfg(test)]
mod tests {
    use super::*;
    use y_core::types::Role;

    fn system_item(content: &str) -> ContextItem {
        ContextItem {
            category: ContextCategory::SystemPrompt,
            content: content.to_string(),
            token_estimate: 0,
            priority: 0,
        }
    }

    fn user_msg(content: &str) -> Message {
        Message {
            message_id: y_core::types::generate_message_id(),
            role: Role::User,
            content: content.to_string(),
            tool_call_id: None,
            tool_calls: vec![],
            timestamp: y_core::types::now(),
            metadata: serde_json::Value::Null,
        }
    }

    #[test]
    fn prepends_system_from_assembled_context() {
        let assembled = AssembledContext {
            items: vec![system_item("You are helpful.")],
            request: None,
        };
        let history = vec![user_msg("Hello")];

        let messages = build_chat_messages(&assembled, &history);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, Role::System);
        assert_eq!(messages[0].content, "You are helpful.");
        assert_eq!(messages[1].content, "Hello");
    }

    #[test]
    fn no_system_when_context_empty() {
        let assembled = AssembledContext {
            items: vec![],
            request: None,
        };
        let history = vec![user_msg("Hi")];

        let messages = build_chat_messages(&assembled, &history);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "Hi");
    }

    #[test]
    fn joins_multiple_system_items() {
        let assembled = AssembledContext {
            items: vec![
                system_item("Part A"),
                ContextItem {
                    category: ContextCategory::Skills,
                    content: "Part B".to_string(),
                    token_estimate: 0,
                    priority: 0,
                },
            ],
            request: None,
        };
        let history = vec![];

        let messages = build_chat_messages(&assembled, &history);
        assert_eq!(messages.len(), 1);
        assert!(messages[0].content.contains("Part A"));
        assert!(messages[0].content.contains("Part B"));
    }
}
