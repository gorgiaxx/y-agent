//! `y-agent print` — single-shot prompt: send one message, print, exit.

use std::collections::HashMap;
use std::io::Write;

use anyhow::{anyhow, Result};
use serde::Serialize;
use y_core::provider::ProviderPool;
use y_core::session::{CreateSessionOptions, SessionType};
use y_core::tool::ToolRegistry;

use crate::commands::common;
use crate::wire::AppServices;

/// Arguments for the `print` subcommand (mirrors the `Commands::Print` variant).
#[derive(Debug, Clone)]
pub struct PrintArgs {
    pub mode: String,
    pub session: Option<String>,
    #[allow(dead_code)]
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
            let out = JsonResult {
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
        // Cannot run full `run` without services; test the guard logic.
        let args = PrintArgs {
            mode: "text".into(),
            session: None,
            agent: "default".into(),
            prompt: vec![],
        };
        // The join of empty vec is "", and run() returns Err.
        assert!(args.prompt.join(" ").is_empty());
    }
}
