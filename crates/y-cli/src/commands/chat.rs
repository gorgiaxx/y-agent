//! Interactive chat session command.

use anyhow::Result;
use std::collections::HashMap;
use std::io::{self, BufRead, Write};

use y_core::provider::ProviderPool;
use y_core::session::{CreateSessionOptions, SessionType};
use y_core::tool::ToolRegistry;
use y_core::types::{Message, Role, SessionId};
use y_service::PromptContext;

use crate::orchestrator::{self, TurnInput};
use crate::output;
use crate::wire::AppServices;

/// Run an interactive chat session.
pub async fn run(services: &AppServices, session_id: Option<&str>, _agent: &str) -> Result<()> {
    // Check if providers are available.
    let provider_statuses = services.provider_pool().await.provider_statuses().await;
    if provider_statuses.is_empty() {
        output::print_warning("No LLM providers configured.");
        output::print_info(
            "To enable LLM interaction, configure providers in providers.toml and set API key environment variables.",
        );
        output::print_info("Example: export OPENAI_API_KEY=\"sk-...\"");
        output::print_info("Running in echo mode (no LLM responses).\n");
    } else {
        let active = provider_statuses.iter().filter(|s| !s.is_frozen).count();
        output::print_info(&format!(
            "{} provider(s) available ({} active)",
            provider_statuses.len(),
            active
        ));
    }

    // Create or resume a session.
    let session = if let Some(id) = session_id {
        let sid = SessionId(id.to_string());
        match services.session_manager.get_session(&sid).await {
            Ok(s) => {
                output::print_info(&format!(
                    "Resuming session: {} ({} messages)",
                    s.id.0, s.message_count
                ));
                s
            }
            Err(e) => {
                output::print_error(&format!("Session not found: {e}"));
                return Ok(());
            }
        }
    } else {
        let options = CreateSessionOptions {
            parent_id: None,
            session_type: SessionType::Main,
            agent_id: None,
            title: Some("Interactive chat".to_string()),
        };
        let s = services
            .session_manager
            .create_session(options)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        output::print_info(&format!("New session: {}", s.id.0));
        s
    };

    // Parse session ID as UUID for diagnostics tracing.
    let session_uuid =
        uuid::Uuid::parse_str(&session.id.0).unwrap_or_else(|_| uuid::Uuid::new_v4());

    // Initialize PromptContext with current state.
    let tool_names: Vec<String> = services
        .tool_registry
        .tool_index()
        .await
        .into_iter()
        .map(|entry| entry.name.as_str().to_string())
        .collect();

    let initial_ctx = PromptContext {
        agent_mode: "general".into(),
        active_skills: vec![],
        available_tools: tool_names,
        config_flags: HashMap::new(),
        working_directory: None,
        custom_system_prompt: None,
        selected_prompt_sections: None,
        mcp_server_instructions: None,
    };
    *services.prompt_context.write().await = initial_ctx;

    println!("Type your message (Ctrl+D to exit, /help for commands):\n");

    let has_providers = !provider_statuses.is_empty();
    let mut history: Vec<Message> = Vec::new();
    let mut turn_number: u32 = 0;
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    print!("> ");
    stdout.flush()?;

    for line in stdin.lock().lines() {
        let line = line?;
        let input = line.trim();

        if input.is_empty() {
            print!("> ");
            stdout.flush()?;
            continue;
        }

        // Handle slash commands.
        if input.starts_with('/') {
            if handle_slash_command(input, services, &session, &mut history, &mut turn_number).await
            {
                break;
            }
            println!();
            print!("> ");
            stdout.flush()?;
            continue;
        }

        // Build user message.
        let user_msg = Message {
            message_id: y_core::types::generate_message_id(),
            role: Role::User,
            content: input.to_string(),
            tool_call_id: None,
            tool_calls: vec![],
            timestamp: y_core::types::now(),
            metadata: serde_json::Value::Null,
        };

        // Persist user message.
        let _ = services
            .session_manager
            .append_message(&session.id, &user_msg)
            .await;

        history.push(user_msg);

        if has_providers {
            // Delegate to the shared orchestrator.
            let turn_input = TurnInput {
                user_input: input,
                session_id: session.id.clone(),
                session_uuid,
                history: &history,
                turn_number,
                provider_id: None,
                knowledge_collections: vec![],
                thinking: None,
                plan_mode: None,
                agent_name: "chat-turn".to_string(),
                toolcall_enabled: true,
                preferred_models: vec![],
                provider_tags: vec![],
                temperature: None,
                max_completion_tokens: None,
                max_iterations: None,
                max_tool_calls: None,
                trust_tier: None,
                agent_allowed_tools: vec![],
                prune_tool_history: false,
                mcp_mode: None,
                mcp_servers: vec![],
            };

            match orchestrator::execute_turn(services, &turn_input).await {
                Ok(result) => {
                    // Print tool call summaries.
                    for tc in &result.tool_calls_executed {
                        let status = if tc.success { "[OK]" } else { "[FAIL]" };
                        println!("\n  [tool: {}] {status}", tc.name);
                    }

                    println!("\nAssistant: {}\n", result.content);

                    // Append the new messages to local history.
                    history.extend(result.new_messages);
                    turn_number += 1;
                }
                Err(e) => {
                    output::print_error(&format!("LLM request failed: {e}"));
                }
            }
        } else {
            // No providers — echo mode.
            println!("\nAssistant: [echo] {input}");
            println!("           (No LLM providers configured — running in echo mode)\n");
        }

        print!("> ");
        stdout.flush()?;
    }

    Ok(())
}

/// Handle a slash command. Returns `true` if the chat should exit.
async fn handle_slash_command(
    input: &str,
    services: &AppServices,
    session: &y_core::session::SessionNode,
    history: &mut Vec<Message>,
    turn_number: &mut u32,
) -> bool {
    match input {
        "/help" => {
            println!("Commands:");
            println!("  /help         -- Show this help");
            println!("  /status       -- Show session status");
            println!("  /clear        -- Clear conversation history");
            println!("  /undo         -- Undo last turn");
            println!("  /checkpoints  -- List available checkpoints");
            println!("  /quit         -- Exit chat");
        }
        "/quit" | "/exit" => {
            output::print_info("Goodbye!");
            return true;
        }
        "/clear" => {
            history.clear();
            *turn_number = 0;
            output::print_success("Conversation history cleared");
        }
        "/status" => {
            println!("Session: {} ({:?})", session.id.0, session.state);
            println!("History: {} messages", history.len());
            println!("Turn: {}", *turn_number);
            println!("Tools: {}", services.tool_registry.len().await);
            let statuses = services.provider_pool().await.provider_statuses().await;
            println!("Providers: {}", statuses.len());
        }
        "/undo" => {
            match services
                .chat_checkpoint_manager
                .rollback_last(&session.id)
                .await
            {
                Ok(result) => {
                    *history = services
                        .session_manager
                        .read_transcript(&session.id)
                        .await
                        .unwrap_or_default();
                    *turn_number = result.rolled_back_to_turn;
                    output::print_success(&format!(
                        "Rolled back {} messages, {} checkpoint(s) invalidated. Now at turn {}.",
                        result.messages_removed,
                        result.checkpoints_invalidated,
                        result.rolled_back_to_turn,
                    ));
                    if !result.scopes_rolled_back.is_empty() {
                        output::print_info(&format!(
                            "File journal scopes to rollback: {:?}",
                            result.scopes_rolled_back
                        ));
                    }
                }
                Err(e) => {
                    output::print_error(&format!("Undo failed: {e}"));
                }
            }
        }
        "/checkpoints" => {
            match services
                .chat_checkpoint_manager
                .list_checkpoints(&session.id)
                .await
            {
                Ok(checkpoints) => {
                    if checkpoints.is_empty() {
                        output::print_info("No checkpoints available.");
                    } else {
                        println!("Available checkpoints:");
                        for cp in &checkpoints {
                            println!(
                                "  Turn {} | {} msgs before | ID: {}...",
                                cp.turn_number,
                                cp.message_count_before,
                                &cp.checkpoint_id[..8],
                            );
                        }
                    }
                }
                Err(e) => {
                    output::print_error(&format!("Failed to list checkpoints: {e}"));
                }
            }
        }
        other => {
            output::print_error(&format!("Unknown command: {other}"));
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use y_service::{AssembledContext, ChatService, ContextCategory, ContextItem};

    fn make_history() -> Vec<Message> {
        vec![
            Message {
                message_id: y_core::types::generate_message_id(),
                role: Role::User,
                content: "Hello".to_string(),
                tool_call_id: None,
                tool_calls: vec![],
                timestamp: y_core::types::now(),
                metadata: serde_json::Value::Null,
            },
            Message {
                message_id: y_core::types::generate_message_id(),
                role: Role::Assistant,
                content: "Hi there!".to_string(),
                tool_call_id: None,
                tool_calls: vec![],
                timestamp: y_core::types::now(),
                metadata: serde_json::Value::Null,
            },
        ]
    }

    // T-CLI-006-01: System prompt from assembled context appears first.
    #[test]
    fn test_build_chat_messages_prepends_system() {
        let mut assembled = AssembledContext::default();
        assembled.add(ContextItem {
            category: ContextCategory::SystemPrompt,
            content: "You are y-agent, a helpful AI assistant.".to_string(),
            token_estimate: 10,
            priority: 100,
        });

        let history = make_history();
        let messages = ChatService::build_chat_messages(&assembled, &history);

        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].role, Role::System);
        assert!(messages[0].content.contains("y-agent"));
        assert_eq!(messages[1].role, Role::User);
        assert_eq!(messages[2].role, Role::Assistant);
    }

    // T-CLI-006-02: Empty assembled context → no system message, just history.
    #[test]
    fn test_build_chat_messages_no_system_when_empty() {
        let assembled = AssembledContext::default();
        let history = make_history();
        let messages = ChatService::build_chat_messages(&assembled, &history);

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, Role::User);
        assert_eq!(messages[1].role, Role::Assistant);
    }

    // T-CLI-006-03: History messages follow system message in order.
    #[test]
    fn test_build_chat_messages_preserves_history_order() {
        let mut assembled = AssembledContext::default();
        assembled.add(ContextItem {
            category: ContextCategory::SystemPrompt,
            content: "System prompt".to_string(),
            token_estimate: 5,
            priority: 100,
        });

        let history = vec![
            Message {
                message_id: y_core::types::generate_message_id(),
                role: Role::User,
                content: "First".to_string(),
                tool_call_id: None,
                tool_calls: vec![],
                timestamp: y_core::types::now(),
                metadata: serde_json::Value::Null,
            },
            Message {
                message_id: y_core::types::generate_message_id(),
                role: Role::Assistant,
                content: "Second".to_string(),
                tool_call_id: None,
                tool_calls: vec![],
                timestamp: y_core::types::now(),
                metadata: serde_json::Value::Null,
            },
            Message {
                message_id: y_core::types::generate_message_id(),
                role: Role::User,
                content: "Third".to_string(),
                tool_call_id: None,
                tool_calls: vec![],
                timestamp: y_core::types::now(),
                metadata: serde_json::Value::Null,
            },
        ];

        let messages = ChatService::build_chat_messages(&assembled, &history);
        assert_eq!(messages.len(), 4);
        assert_eq!(messages[0].role, Role::System);
        assert_eq!(messages[1].content, "First");
        assert_eq!(messages[2].content, "Second");
        assert_eq!(messages[3].content, "Third");
    }

    // T-CLI-006-04: Multiple system prompt items are joined.
    #[test]
    fn test_build_chat_messages_joins_multiple_system_items() {
        let mut assembled = AssembledContext::default();
        assembled.add(ContextItem {
            category: ContextCategory::SystemPrompt,
            content: "Part one".to_string(),
            token_estimate: 5,
            priority: 100,
        });
        assembled.add(ContextItem {
            category: ContextCategory::Status,
            content: "status info".to_string(),
            token_estimate: 5,
            priority: 500,
        });
        assembled.add(ContextItem {
            category: ContextCategory::SystemPrompt,
            content: "Part two".to_string(),
            token_estimate: 5,
            priority: 200,
        });

        let history = make_history();
        let messages = ChatService::build_chat_messages(&assembled, &history);

        // Only SystemPrompt items → 1 system message.
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].role, Role::System);
        assert!(messages[0].content.contains("Part one"));
        assert!(messages[0].content.contains("Part two"));
        // Status item should NOT appear in the system message.
        assert!(!messages[0].content.contains("status info"));
    }

    // T-CLI-006-05: Empty history with system prompt → just system message.
    #[test]
    fn test_build_chat_messages_system_only_no_history() {
        let mut assembled = AssembledContext::default();
        assembled.add(ContextItem {
            category: ContextCategory::SystemPrompt,
            content: "You are y-agent.".to_string(),
            token_estimate: 5,
            priority: 100,
        });

        let messages = ChatService::build_chat_messages(&assembled, &[]);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, Role::System);
    }
}
