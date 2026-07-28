//! Copy-target parsing and transcript extraction for the TUI.

use std::fmt::Write as _;

use crate::tui::state::{ChatMessage, MessageRole, ToolCallInfo, ToolCallStatus};

/// Semantic category shown in the copy selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyItemKind {
    AssistantResponse,
    CodeBlock,
    ToolInput,
    ToolResult,
    Command,
    Path,
    Transcript,
}

/// Clipboard-ready item displayed by the full-screen copy selector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyItem {
    pub kind: CopyItemKind,
    pub label: String,
    pub detail: String,
    pub content: String,
}

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

/// Build searchable copy targets from the visible conversation.
pub fn discover_copy_items(messages: &[ChatMessage]) -> Vec<CopyItem> {
    let mut items = Vec::new();

    for (recent_index, message) in messages
        .iter()
        .rev()
        .filter(|message| message.role == MessageRole::Assistant)
        .enumerate()
    {
        let response_number = recent_index + 1;
        if !message.content.trim().is_empty() {
            items.push(CopyItem {
                kind: CopyItemKind::AssistantResponse,
                label: format!("Assistant response {response_number}"),
                detail: first_non_empty_line(&message.content),
                content: message.content.clone(),
            });
        }

        for (block_index, (language, block)) in fenced_blocks_with_language(&message.content)
            .into_iter()
            .enumerate()
        {
            items.push(CopyItem {
                kind: CopyItemKind::CodeBlock,
                label: format!("Response {response_number} / code {}", block_index + 1),
                detail: if language.is_empty() {
                    "Fenced code".into()
                } else {
                    language.clone()
                },
                content: block.clone(),
            });
            if matches!(language.as_str(), "sh" | "bash" | "shell" | "zsh" | "fish") {
                items.push(CopyItem {
                    kind: CopyItemKind::Command,
                    label: format!("Response {response_number} / command {}", block_index + 1),
                    detail: "Runnable shell command".into(),
                    content: block,
                });
            }
        }

        for tool in message.tool_calls.iter().rev() {
            if !tool.input_preview.trim().is_empty() {
                items.push(CopyItem {
                    kind: CopyItemKind::ToolInput,
                    label: format!("{} input", tool.name),
                    detail: format!("Tool call in response {response_number}"),
                    content: tool.input_preview.clone(),
                });
            }
            if let Ok(input) = serde_json::from_str::<serde_json::Value>(&tool.input_preview) {
                if let Some(command) = input.get("command").and_then(serde_json::Value::as_str) {
                    items.push(CopyItem {
                        kind: CopyItemKind::Command,
                        label: format!("Response {response_number} / {} command", tool.name),
                        detail: "Tool command".into(),
                        content: command.to_string(),
                    });
                }
                for field in ["path", "file_path"] {
                    if let Some(path) = input.get(field).and_then(serde_json::Value::as_str) {
                        items.push(CopyItem {
                            kind: CopyItemKind::Path,
                            label: format!("Response {response_number} / {} path", tool.name),
                            detail: field.replace('_', " "),
                            content: path.to_string(),
                        });
                    }
                }
            }
            if !tool.result_preview.trim().is_empty() {
                items.push(CopyItem {
                    kind: CopyItemKind::ToolResult,
                    label: format!("{} result", tool.name),
                    detail: format!("Tool call in response {response_number}"),
                    content: tool.result_preview.clone(),
                });
            }
        }
    }

    if !messages.is_empty() {
        items.push(CopyItem {
            kind: CopyItemKind::Transcript,
            label: "Complete transcript".into(),
            detail: format!("{} visible messages", messages.len()),
            content: format_transcript(messages),
        });
    }

    items
}

/// Format one tool call as a self-contained clipboard record.
pub fn format_tool_call_for_copy(tool: &ToolCallInfo) -> String {
    let status = match tool.status {
        ToolCallStatus::Running => "running",
        ToolCallStatus::Succeeded => "succeeded",
        ToolCallStatus::Failed => "failed",
    };
    let timing = tool
        .duration_ms
        .map_or_else(String::new, |duration| format!(", {duration}ms"));
    let mut output = format!("[Tool: {}] ({status}{timing})", tool.name);
    if !tool.input_preview.trim().is_empty() {
        let _ = write!(output, "\nInput:\n{}", tool.input_preview);
    }
    if !tool.result_preview.trim().is_empty() {
        let _ = write!(output, "\nResult:\n{}", tool.result_preview);
    }
    output
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
    fenced_blocks(content)
        .into_iter()
        .rev()
        .find(|block| !block.trim().is_empty())
}

fn fenced_blocks(content: &str) -> Vec<String> {
    fenced_blocks_with_language(content)
        .into_iter()
        .map(|(_, block)| block)
        .collect()
}

fn fenced_blocks_with_language(content: &str) -> Vec<(String, String)> {
    let mut blocks = Vec::new();
    let mut current: Option<(String, Vec<&str>)> = None;

    for line in content.lines() {
        if line.trim_start().starts_with("```") {
            if let Some((language, lines)) = current.take() {
                blocks.push((language, lines.join("\n")));
            } else {
                let language = line
                    .trim_start()
                    .trim_start_matches("```")
                    .trim()
                    .to_ascii_lowercase();
                current = Some((language, Vec::new()));
            }
        } else if let Some((_, lines)) = current.as_mut() {
            lines.push(line);
        }
    }

    blocks
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
            let mut section = format!("[{role}]\n{}", message.content);
            for tool in &message.tool_calls {
                let status = match tool.status {
                    ToolCallStatus::Running => "running",
                    ToolCallStatus::Succeeded => "succeeded",
                    ToolCallStatus::Failed => "failed",
                };
                let _ = write!(section, "\n\n[Tool: {}] ({status})", tool.name);
                if !tool.input_preview.trim().is_empty() {
                    let _ = write!(section, "\nInput:\n{}", tool.input_preview);
                }
                if !tool.result_preview.trim().is_empty() {
                    let _ = write!(section, "\nResult:\n{}", tool.result_preview);
                }
            }
            section
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn first_non_empty_line(content: &str) -> String {
    content
        .lines()
        .find(|line| !line.trim().is_empty())
        .map_or_else(String::new, |line| line.trim().chars().take(80).collect())
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;
    use crate::tui::state::ToolCallDisplayMode;

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

    #[test]
    fn test_discover_copy_items_includes_tool_inputs_results_and_transcript() {
        let mut assistant = message(MessageRole::Assistant, "Run this:\n```sh\ncargo test\n```");
        assistant.tool_calls.push(ToolCallInfo {
            tool_call_id: "call-shell-1".into(),
            name: "ShellExec".into(),
            status: ToolCallStatus::Succeeded,
            duration_ms: Some(30),
            input_preview: r#"{"command":"cargo test"}"#.into(),
            result_preview: "42 passed".into(),
            agent_name: "chat-turn".into(),
            url_meta: None,
            metadata: None,
            display_mode: ToolCallDisplayMode::Preview,
        });
        let messages = vec![message(MessageRole::User, "test it"), assistant];

        let items = discover_copy_items(&messages);

        assert!(items.iter().any(|item| {
            item.kind == CopyItemKind::ToolInput && item.content.contains("cargo test")
        }));
        assert!(items
            .iter()
            .any(|item| { item.kind == CopyItemKind::ToolResult && item.content == "42 passed" }));
        assert!(items
            .iter()
            .any(|item| { item.kind == CopyItemKind::CodeBlock && item.content == "cargo test" }));
        assert!(items
            .iter()
            .any(|item| item.kind == CopyItemKind::Transcript));
    }

    #[test]
    fn test_transcript_copy_includes_tool_details() {
        let mut assistant = message(MessageRole::Assistant, "Done");
        assistant.tool_calls.push(ToolCallInfo {
            tool_call_id: "call-edit-1".into(),
            name: "FileEdit".into(),
            status: ToolCallStatus::Succeeded,
            duration_ms: Some(12),
            input_preview: r#"{"path":"src/main.rs"}"#.into(),
            result_preview: "updated src/main.rs".into(),
            agent_name: "chat-turn".into(),
            url_meta: None,
            metadata: None,
            display_mode: ToolCallDisplayMode::Preview,
        });

        let transcript = resolve_target(&[assistant], CopyTarget::Transcript).unwrap();

        assert!(transcript.contains("[Tool: FileEdit]"));
        assert!(transcript.contains("src/main.rs"));
        assert!(transcript.contains("updated src/main.rs"));
    }

    #[test]
    fn test_discover_copy_items_extracts_runnable_commands_and_paths() {
        let mut assistant = message(
            MessageRole::Assistant,
            "Run it:\n```sh\ncargo test -p y-cli\n```",
        );
        assistant.tool_calls.push(ToolCallInfo {
            tool_call_id: "tool-1".into(),
            name: "FileRead".into(),
            status: ToolCallStatus::Succeeded,
            duration_ms: Some(1),
            input_preview: r#"{"path":"/tmp/report.txt"}"#.into(),
            result_preview: "ok".into(),
            agent_name: "root".into(),
            url_meta: None,
            metadata: None,
            display_mode: ToolCallDisplayMode::Preview,
        });

        let items = discover_copy_items(&[assistant]);

        assert!(items.iter().any(|item| {
            item.kind == CopyItemKind::Command && item.content == "cargo test -p y-cli"
        }));
        assert!(items
            .iter()
            .any(|item| { item.kind == CopyItemKind::Path && item.content == "/tmp/report.txt" }));
    }

    #[test]
    fn test_format_tool_call_for_copy_includes_input_and_result() {
        let tool = ToolCallInfo {
            tool_call_id: "call-shell-1".into(),
            name: "ShellExec".into(),
            status: ToolCallStatus::Succeeded,
            duration_ms: Some(14),
            input_preview: r#"{"command":"cargo test"}"#.into(),
            result_preview: "42 passed".into(),
            agent_name: "chat-turn".into(),
            url_meta: None,
            metadata: None,
            display_mode: ToolCallDisplayMode::Preview,
        };

        let copied = format_tool_call_for_copy(&tool);

        assert!(copied.contains("[Tool: ShellExec]"));
        assert!(copied.contains("cargo test"));
        assert!(copied.contains("42 passed"));
    }
}
