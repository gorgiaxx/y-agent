//! `y-agent print` — single-shot prompt: send one message, print, exit.

#[cfg(not(feature = "automation_a2a"))]
use std::collections::HashMap;
use std::io::Write;
#[cfg(feature = "automation_a2a")]
use std::path::PathBuf;

use anyhow::{anyhow, Result};
use serde::Serialize;
#[cfg(not(feature = "automation_a2a"))]
use y_core::provider::ProviderPool;
#[cfg(not(feature = "automation_a2a"))]
use y_core::session::{CreateSessionOptions, SessionType};
#[cfg(not(feature = "automation_a2a"))]
use y_core::tool::ToolRegistry;
#[cfg(feature = "automation_a2a")]
use y_service::{AutomationRunRequest, AutomationRunService, ChatService};

#[cfg(not(feature = "automation_a2a"))]
use crate::commands::common;
use crate::wire::AppServices;

/// Arguments for the `print` subcommand (mirrors the `Commands::Print` variant).
#[derive(Debug, Clone)]
pub struct PrintArgs {
    pub mode: String,
    pub session: Option<String>,
    #[cfg_attr(not(feature = "automation_a2a"), allow(dead_code))]
    pub agent: String,
    pub prompt: Vec<String>,
}

/// Output mode for the print command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrintMode {
    /// Final response text only.
    Text,
    /// Structured JSON result.
    Json,
}

impl PrintMode {
    fn parse(s: &str) -> Result<Self> {
        match s {
            "text" => Ok(Self::Text),
            "json" => Ok(Self::Json),
            other => Err(anyhow!(
                "invalid print mode `{other}` (expected `text` or `json`)"
            )),
        }
    }
}

/// JSON-serializable result for `--mode json`.
#[derive(Serialize)]
struct JsonResult<'a> {
    session_reference: &'a str,
    /// Legacy raw identifier retained for existing print-mode consumers.
    session_id: &'a str,
    turn: u32,
    content: &'a str,
    model: &'a str,
    tool_calls: Vec<JsonToolCall>,
}

#[derive(Serialize)]
struct JsonToolCall {
    name: String,
    success: bool,
}

/// Run the print command.
#[cfg(feature = "automation_a2a")]
pub async fn run(services: &AppServices, args: PrintArgs) -> Result<()> {
    let mode = PrintMode::parse(&args.mode)?;
    let request = build_automation_request(args, std::env::current_dir()?)?;
    let prepared = AutomationRunService::prepare(services, request).await?;
    let result = ChatService::execute_turn(services, &prepared.turn.as_turn_input())
        .await
        .map_err(|error| anyhow!("turn failed: {error}"))?;

    match mode {
        PrintMode::Text => {
            for tool_call in &result.tool_calls_executed {
                let status = if tool_call.success { "[OK]" } else { "[FAIL]" };
                eprintln!("[tool: {}] {status}", tool_call.name);
            }
            println!("{}", result.content);
        }
        PrintMode::Json => {
            let tool_calls = result
                .tool_calls_executed
                .iter()
                .map(|tool_call| JsonToolCall {
                    name: tool_call.name.clone(),
                    success: tool_call.success,
                })
                .collect();
            let output = JsonResult {
                session_reference: &prepared.session_reference,
                session_id: prepared.turn.session_id.as_str(),
                turn: prepared.turn.turn_number.saturating_add(1),
                content: &result.content,
                model: &result.model,
                tool_calls,
            };
            println!("{}", serde_json::to_string(&output)?);
        }
    }
    std::io::stdout().flush()?;
    Ok(())
}

#[cfg(feature = "automation_a2a")]
fn build_automation_request(args: PrintArgs, workspace: PathBuf) -> Result<AutomationRunRequest> {
    let PrintArgs {
        mode: _,
        session,
        agent,
        prompt,
    } = args;
    let user_input = prompt.join(" ");
    if user_input.trim().is_empty() {
        return Err(anyhow!(
            "no prompt provided (use `y-agent print -- \"your prompt\"`)"
        ));
    }
    let session_name = session.is_none().then(|| "print".to_string());
    let agent_id = match agent.as_str() {
        "default" | "chat" => None,
        _ => Some(agent),
    };

    Ok(AutomationRunRequest {
        session_target: session,
        continue_last: false,
        session_name,
        agent_id,
        user_input,
        workspace,
        provider_id: None,
        model: None,
        skills: None,
        knowledge_collections: None,
        thinking: None,
        plan_mode: Some("fast".to_string()),
        operation_mode: None,
    })
}

/// Compatibility implementation when the automation subsystem is disabled.
#[cfg(not(feature = "automation_a2a"))]
pub async fn run(services: &AppServices, args: PrintArgs) -> Result<()> {
    let mode = PrintMode::parse(&args.mode)?;
    let prompt = args.prompt.join(" ");
    if prompt.is_empty() {
        return Err(anyhow!(
            "no prompt provided (use `y-agent print -- \"your prompt\"`)"
        ));
    }

    // Check providers.
    let provider_statuses = services.provider_pool().await.provider_statuses().await;
    if provider_statuses.is_empty() {
        return Err(anyhow!(
            "no LLM providers configured; run `y-agent init` to set up a provider"
        ));
    }

    // Create or resume session.
    let session = if let Some(id) = &args.session {
        let sid = y_core::types::SessionId(id.clone());
        services
            .session_manager
            .get_session(&sid)
            .await
            .map_err(|e| anyhow!("session not found: {e}"))?
    } else {
        let options = CreateSessionOptions {
            parent_id: None,
            session_type: SessionType::Main,
            agent_id: None,
            title: Some("print".to_string()),
        };
        services
            .session_manager
            .create_session(options)
            .await
            .map_err(|e| anyhow!("{e}"))?
    };

    let session_uuid =
        uuid::Uuid::parse_str(&session.id.0).unwrap_or_else(|_| uuid::Uuid::new_v4());
    let working_directory = std::env::current_dir()
        .ok()
        .map(|p| p.to_string_lossy().to_string());

    // Initialize PromptContext.
    let tool_names: Vec<String> = services
        .tool_registry
        .tool_index()
        .await
        .into_iter()
        .map(|e| e.name.as_str().to_string())
        .collect();
    let initial_ctx = y_service::PromptContext {
        agent_mode: "general".into(),
        active_skills: vec![],
        available_tools: tool_names,
        config_flags: HashMap::new(),
        working_directory: working_directory.clone(),
        custom_system_prompt: None,
        selected_prompt_sections: None,
        mcp_server_instructions: None,
    };
    *services.prompt_context.write().await = initial_ctx;

    let mut history = common::load_history(services, &session.id).await;
    let mut turn_number: u32 = 0;

    let result = common::run_single_turn(
        services,
        &session,
        &mut history,
        &mut turn_number,
        &prompt,
        working_directory,
        session_uuid,
    )
    .await
    .map_err(|e| anyhow!("turn failed: {e}"))?;

    match mode {
        PrintMode::Text => {
            // Tool call summaries to stderr (so stdout has only the response).
            for tc in &result.tool_calls_executed {
                let status = if tc.success { "[OK]" } else { "[FAIL]" };
                eprintln!("[tool: {}] {status}", tc.name);
            }
            println!("{}", result.content);
            let _ = std::io::stdout().flush();
        }
        PrintMode::Json => {
            let tool_calls: Vec<JsonToolCall> = result
                .tool_calls_executed
                .iter()
                .map(|tc| JsonToolCall {
                    name: tc.name.clone(),
                    success: tc.success,
                })
                .collect();
            let session_reference =
                y_service::SessionService::public_session_reference(&session.id);
            let out = JsonResult {
                session_reference: &session_reference,
                session_id: &session.id.0,
                turn: turn_number,
                content: &result.content,
                model: &result.model,
                tool_calls,
            };
            println!("{}", serde_json::to_string(&out)?);
            let _ = std::io::stdout().flush();
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // T-CLI-PRINT-01: mode parsing.
    #[test]
    fn test_mode_parse() {
        assert!(matches!(PrintMode::parse("text").unwrap(), PrintMode::Text));
        assert!(matches!(PrintMode::parse("json").unwrap(), PrintMode::Json));
        assert!(PrintMode::parse("xml").is_err());
    }

    // T-CLI-PRINT-02: empty prompt is rejected.
    #[test]
    fn test_empty_prompt_rejected() {
        let args = PrintArgs {
            mode: "text".into(),
            session: None,
            agent: "default".into(),
            prompt: vec![],
        };

        #[cfg(feature = "automation_a2a")]
        assert!(build_automation_request(args, std::path::PathBuf::from("/workspace")).is_err());

        #[cfg(not(feature = "automation_a2a"))]
        assert!(args.prompt.join(" ").is_empty());
    }

    #[cfg(feature = "automation_a2a")]
    #[test]
    fn test_automation_request_forwards_named_agent_and_session() {
        let request = build_automation_request(
            PrintArgs {
                mode: "json".into(),
                session: Some("ses_existing".into()),
                agent: "general-purpose".into(),
                prompt: vec!["analyze".into(), "CVE-2026-0001".into()],
            },
            std::path::PathBuf::from("/workspace"),
        )
        .expect("print request should adapt to automation");

        assert_eq!(request.session_target.as_deref(), Some("ses_existing"));
        assert_eq!(request.agent_id.as_deref(), Some("general-purpose"));
        assert_eq!(request.user_input, "analyze CVE-2026-0001");
        assert_eq!(request.workspace, std::path::PathBuf::from("/workspace"));
    }

    #[cfg(feature = "automation_a2a")]
    #[test]
    fn test_automation_request_maps_default_agent_to_chat() {
        let request = build_automation_request(
            PrintArgs {
                mode: "text".into(),
                session: None,
                agent: "default".into(),
                prompt: vec!["hello".into()],
            },
            std::path::PathBuf::from("/workspace"),
        )
        .expect("default print request should adapt to chat");

        assert!(request.agent_id.is_none());
    }

    #[test]
    fn test_json_result_exposes_public_and_raw_session_references() {
        let result = JsonResult {
            session_reference: "ses_raw-id",
            session_id: "raw-id",
            turn: 1,
            content: "done",
            model: "model",
            tool_calls: Vec::new(),
        };

        let value = serde_json::to_value(result).expect("print result should serialize");
        assert_eq!(value["session_reference"], "ses_raw-id");
        assert_eq!(value["session_id"], "raw-id");
    }
}
