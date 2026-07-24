//! Copy-target parsing and transcript extraction for the TUI.

use crate::tui::state::{ChatMessage, MessageRole};

/// Conversation content selected by `/copy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyTarget {
    /// The Nth most recent non-empty assistant response, where 1 is latest.
    AssistantResponse(usize),
    /// The most recent fenced code block in an assistant response.
    LastCodeBlock,
    /// The full visible transcript.
    Transcript,
}

/// Parse `/copy [N|code|transcript]` arguments.
pub fn parse_target(args: &str) -> Result<CopyTarget, String> {
    let trimmed = args.trim();
    if trimmed.is_empty() {
        return Ok(CopyTarget::AssistantResponse(1));
    }

    match trimmed.to_ascii_lowercase().as_str() {
        "code" => Ok(CopyTarget::LastCodeBlock),
        "transcript" | "all" => Ok(CopyTarget::Transcript),
        _ => match trimmed.parse::<usize>() {
            Ok(0) | Err(_) => Err(copy_usage()),
            Ok(n) => Ok(CopyTarget::AssistantResponse(n)),
        },
    }
}

/// Resolve a copy target into clipboard-ready text.
pub fn resolve_target(messages: &[ChatMessage], target: CopyTarget) -> Result<String, String> {
    match target {
        CopyTarget::AssistantResponse(nth) => nth_assistant_response(messages, nth)
            .ok_or_else(|| format!("Assistant response {nth} is not available.")),
        CopyTarget::LastCodeBlock => last_code_block(messages)
            .ok_or_else(|| "No fenced assistant code block is available.".to_string()),
        CopyTarget::Transcript => {
            if messages.is_empty() {
                return Err("No messages to copy.".to_string());
            }
            Ok(format_transcript(messages))
        }
    }
}

fn copy_usage() -> String {
    "Usage: /copy [N|code|transcript]".to_string()
}

fn nth_assistant_response(messages: &[ChatMessage], nth: usize) -> Option<String> {
    messages
        .iter()
        .rev()
        .filter(|message| {
            message.role == MessageRole::Assistant && !message.content.trim().is_empty()
        })
        .nth(nth.saturating_sub(1))
        .map(|message| message.content.clone())
}

fn last_code_block(messages: &[ChatMessage]) -> Option<String> {
    messages
        .iter()
        .rev()
        .filter(|message| message.role == MessageRole::Assistant)
        .find_map(|message| last_fenced_block(&message.content))
}

fn last_fenced_block(content: &str) -> Option<String> {
    let mut blocks = Vec::new();
    let mut current: Option<Vec<&str>> = None;

    for line in content.lines() {
        if line.trim_start().starts_with("```") {
            if let Some(lines) = current.take() {
                blocks.push(lines.join("\n"));
            } else {
                current = Some(Vec::new());
            }
        } else if let Some(lines) = current.as_mut() {
            lines.push(line);
        }
    }

    blocks
        .into_iter()
        .rev()
        .find(|block| !block.trim().is_empty())
}

fn format_transcript(messages: &[ChatMessage]) -> String {
    messages
        .iter()
        .map(|message| {
            let role = match message.role {
                MessageRole::User => "You",
                MessageRole::Assistant => "Assistant",
                MessageRole::System => "System",
                MessageRole::Tool => "Tool",
            };
            format!("[{role}]\n{}", message.content)
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;

    fn message(role: MessageRole, content: &str) -> ChatMessage {
        ChatMessage {
            role,
            content: content.to_string(),
            timestamp: Utc::now(),
            is_streaming: false,
            is_cancelled: false,
            reasoning_content: String::new(),
            reasoning_complete: false,
            tool_calls: Vec::new(),
            segments: Vec::new(),
        }
    }

    #[test]
    fn test_parse_target_defaults_to_latest_assistant_response() {
        assert_eq!(parse_target("").unwrap(), CopyTarget::AssistantResponse(1));
        assert_eq!(parse_target("2").unwrap(), CopyTarget::AssistantResponse(2));
        assert!(parse_target("0").is_err());
    }

    #[test]
    fn test_resolve_target_selects_nth_recent_assistant_response() {
        let messages = vec![
            message(MessageRole::Assistant, "first"),
            message(MessageRole::User, "question"),
            message(MessageRole::Assistant, "latest"),
        ];

        assert_eq!(
            resolve_target(&messages, CopyTarget::AssistantResponse(1)).unwrap(),
            "latest"
        );
        assert_eq!(
            resolve_target(&messages, CopyTarget::AssistantResponse(2)).unwrap(),
            "first"
        );
    }

    #[test]
    fn test_resolve_target_selects_latest_fenced_code_block() {
        let messages = vec![
            message(MessageRole::Assistant, "```rust\nold()\n```"),
            message(MessageRole::Assistant, "text\n```sh\ncargo test\n```\nmore"),
        ];

        assert_eq!(
            resolve_target(&messages, CopyTarget::LastCodeBlock).unwrap(),
            "cargo test"
        );
    }
}
