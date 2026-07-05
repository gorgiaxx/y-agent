//! `y-agent rpc` — headless JSONL stdio protocol.
//!
//! Reads JSON commands from stdin (one per line), writes JSON events/responses
//! to stdout. Designed for embedding y-agent in other processes (editors,
//! scripts, IDEs) without an HTTP server.

use std::collections::HashMap;
use std::io::Write;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncBufReadExt;
use y_core::provider::ProviderPool;
use y_core::session::{CreateSessionOptions, SessionType};
use y_core::tool::ToolRegistry;
use y_core::types::SessionId;
use y_service::PromptContext;

use crate::commands::common;
use crate::wire::AppServices;

/// A request read from stdin.
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum RpcRequest {
    /// Send a prompt and stream the response.
    #[serde(rename = "prompt")]
    Prompt {
        id: String,
        text: String,
        #[serde(default)]
        session_id: Option<String>,
        #[serde(default = "default_agent")]
        #[allow(dead_code)]
        agent: String,
    },
    /// Create a new session.
    #[serde(rename = "new_session")]
    NewSession {
        id: String,
        #[serde(default)]
        title: Option<String>,
    },
    /// List recent sessions.
    #[serde(rename = "list_sessions")]
    ListSessions { id: String },
    /// Interrupt the current turn (best-effort).
    #[serde(rename = "interrupt")]
    Interrupt { id: String },
}

fn default_agent() -> String {
    "default".to_string()
}

/// A response written to stdout.
#[derive(Debug, Serialize)]
#[serde(untagged)]
enum RpcOutput {
    Response {
        #[serde(rename = "type")]
        kind: &'static str,
        id: String,
        ok: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        data: Option<serde_json::Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    Event {
        #[serde(rename = "type")]
        kind: &'static str,
        event: &'static str,
        #[serde(skip_serializing_if = "Option::is_none")]
        data: Option<serde_json::Value>,
    },
}

/// Run the RPC loop until stdin EOF.
pub async fn run(services: &AppServices) -> Result<()> {
    let provider_statuses = services.provider_pool().await.provider_statuses().await;
    if provider_statuses.is_empty() {
        write_line(&RpcOutput::Response {
            kind: "response",
            id: "init".to_string(),
            ok: false,
            data: None,
            error: Some("no LLM providers configured".to_string()),
        });
        return Ok(());
    }

    // Initialize PromptContext once.
    let tool_names: Vec<String> = services
        .tool_registry
        .tool_index()
        .await
        .into_iter()
        .map(|e| e.name.as_str().to_string())
        .collect();
    let working_directory = std::env::current_dir()
        .ok()
        .map(|p| p.to_string_lossy().to_string());
    let initial_ctx = PromptContext {
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

    let stdin = tokio::io::stdin();
    let mut reader = tokio::io::BufReader::new(stdin);
    let mut line = String::new();

    loop {
        line.clear();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            break; // EOF
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let req: RpcRequest = match serde_json::from_str(trimmed) {
            Ok(r) => r,
            Err(e) => {
                write_line(&RpcOutput::Response {
                    kind: "response",
                    id: "?".to_string(),
                    ok: false,
                    data: None,
                    error: Some(format!("invalid JSON: {e}")),
                });
                continue;
            }
        };

        match req {
            RpcRequest::Prompt {
                id,
                text,
                session_id,
                agent: _,
            } => {
                handle_prompt(
                    services,
                    &id,
                    &text,
                    session_id.as_deref(),
                    working_directory.as_deref(),
                )
                .await;
            }
            RpcRequest::NewSession { id, title } => {
                handle_new_session(services, &id, title).await;
            }
            RpcRequest::ListSessions { id } => {
                handle_list_sessions(services, &id).await;
            }
            RpcRequest::Interrupt { id } => {
                // Best-effort: no live turn to cancel in this sync loop.
                write_line(&RpcOutput::Response {
                    kind: "response",
                    id,
                    ok: true,
                    data: Some(serde_json::json!({"interrupted": false})),
                    error: None,
                });
            }
        }
    }

    Ok(())
}

async fn handle_prompt(
    services: &AppServices,
    id: &str,
    text: &str,
    session_id: Option<&str>,
    working_directory: Option<&str>,
) {
    // Resolve or create session.
    let session = match resolve_or_create_session(services, session_id).await {
        Ok(s) => s,
        Err(e) => {
            write_line(&RpcOutput::Response {
                kind: "response",
                id: id.to_string(),
                ok: false,
                data: None,
                error: Some(e),
            });
            return;
        }
    };

    let session_uuid =
        uuid::Uuid::parse_str(&session.id.0).unwrap_or_else(|_| uuid::Uuid::new_v4());
    let mut history = common::load_history(services, &session.id).await;
    let mut turn_number: u32 = 0;

    // Emit turn_start event.
    write_line(&RpcOutput::Event {
        kind: "event",
        event: "turn_start",
        data: Some(serde_json::json!({
            "session_id": session.id.0,
            "turn": turn_number,
        })),
    });

    let result = common::run_single_turn(
        services,
        &session,
        &mut history,
        &mut turn_number,
        text,
        working_directory.map(str::to_string),
        session_uuid,
    )
    .await;

    match result {
        Ok(r) => {
            // Emit stream_delta with final content (non-streaming v1).
            write_line(&RpcOutput::Event {
                kind: "event",
                event: "stream_delta",
                data: Some(serde_json::json!({ "text": r.content })),
            });
            write_line(&RpcOutput::Event {
                kind: "event",
                event: "turn_end",
                data: Some(serde_json::json!({
                    "session_id": session.id.0,
                    "turn": turn_number,
                    "model": r.model,
                    "input_tokens": r.input_tokens,
                    "output_tokens": r.output_tokens,
                    "tool_calls": r.tool_calls_executed.iter().map(|tc| {
                        serde_json::json!({
                            "name": tc.name,
                            "success": tc.success,
                        })
                    }).collect::<Vec<_>>(),
                })),
            });
            write_line(&RpcOutput::Response {
                kind: "response",
                id: id.to_string(),
                ok: true,
                data: Some(serde_json::json!({
                    "session_id": session.id.0,
                    "content": r.content,
                    "model": r.model,
                })),
                error: None,
            });
        }
        Err(e) => {
            write_line(&RpcOutput::Response {
                kind: "response",
                id: id.to_string(),
                ok: false,
                data: None,
                error: Some(format!("{e}")),
            });
        }
    }
}

async fn handle_new_session(services: &AppServices, id: &str, title: Option<String>) {
    let options = CreateSessionOptions {
        parent_id: None,
        session_type: SessionType::Main,
        agent_id: None,
        title: title.or(Some("rpc".to_string())),
    };
    match services.session_manager.create_session(options).await {
        Ok(s) => {
            write_line(&RpcOutput::Response {
                kind: "response",
                id: id.to_string(),
                ok: true,
                data: Some(serde_json::json!({ "session_id": s.id.0 })),
                error: None,
            });
        }
        Err(e) => {
            write_line(&RpcOutput::Response {
                kind: "response",
                id: id.to_string(),
                ok: false,
                data: None,
                error: Some(format!("{e}")),
            });
        }
    }
}

async fn handle_list_sessions(services: &AppServices, id: &str) {
    match services
        .session_manager
        .list_sessions(&y_core::session::SessionFilter::default())
        .await
    {
        Ok(sessions) => {
            let data: Vec<serde_json::Value> = sessions
                .iter()
                .map(|s| {
                    serde_json::json!({
                        "id": s.id.0,
                        "title": s.title,
                        "message_count": s.message_count,
                    })
                })
                .collect();
            write_line(&RpcOutput::Response {
                kind: "response",
                id: id.to_string(),
                ok: true,
                data: Some(serde_json::json!({ "sessions": data })),
                error: None,
            });
        }
        Err(e) => {
            write_line(&RpcOutput::Response {
                kind: "response",
                id: id.to_string(),
                ok: false,
                data: None,
                error: Some(format!("{e}")),
            });
        }
    }
}

async fn resolve_or_create_session(
    services: &AppServices,
    session_id: Option<&str>,
) -> std::result::Result<y_core::session::SessionNode, String> {
    if let Some(id) = session_id {
        let sid = SessionId(id.to_string());
        services
            .session_manager
            .get_session(&sid)
            .await
            .map_err(|e| format!("session not found: {e}"))
    } else {
        let options = CreateSessionOptions {
            parent_id: None,
            session_type: SessionType::Main,
            agent_id: None,
            title: Some("rpc".to_string()),
        };
        services
            .session_manager
            .create_session(options)
            .await
            .map_err(|e| format!("{e}"))
    }
}

fn write_line(output: &RpcOutput) {
    if let Ok(s) = serde_json::to_string(output) {
        println!("{s}");
        let _ = std::io::stdout().flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // T-CLI-RPC-01: request deserialization — prompt command.
    #[test]
    fn test_parse_prompt_request() {
        let json = r#"{"type":"prompt","id":"r1","text":"hello","agent":"default"}"#;
        let req: RpcRequest = serde_json::from_str(json).unwrap();
        match req {
            RpcRequest::Prompt {
                id, text, agent, ..
            } => {
                assert_eq!(id, "r1");
                assert_eq!(text, "hello");
                assert_eq!(agent, "default");
            }
            _ => panic!("expected Prompt"),
        }
    }

    // T-CLI-RPC-02: request deserialization — new_session command.
    #[test]
    fn test_parse_new_session_request() {
        let json = r#"{"type":"new_session","id":"r2"}"#;
        let req: RpcRequest = serde_json::from_str(json).unwrap();
        match req {
            RpcRequest::NewSession { id, title } => {
                assert_eq!(id, "r2");
                assert!(title.is_none());
            }
            _ => panic!("expected NewSession"),
        }
    }

    // T-CLI-RPC-03: request deserialization — list_sessions command.
    #[test]
    fn test_parse_list_sessions_request() {
        let json = r#"{"type":"list_sessions","id":"r3"}"#;
        let req: RpcRequest = serde_json::from_str(json).unwrap();
        match req {
            RpcRequest::ListSessions { id } => assert_eq!(id, "r3"),
            _ => panic!("expected ListSessions"),
        }
    }

    // T-CLI-RPC-04: response serialization — ok response.
    #[test]
    fn test_serialize_ok_response() {
        let out = RpcOutput::Response {
            kind: "response",
            id: "r1".to_string(),
            ok: true,
            data: Some(serde_json::json!({"content": "hi"})),
            error: None,
        };
        let s = serde_json::to_string(&out).unwrap();
        assert!(s.contains("\"ok\":true"));
        assert!(s.contains("\"content\":\"hi\""));
        assert!(!s.contains("\"error\""));
    }

    // T-CLI-RPC-05: response serialization — error response.
    #[test]
    fn test_serialize_error_response() {
        let out = RpcOutput::Response {
            kind: "response",
            id: "r1".to_string(),
            ok: false,
            data: None,
            error: Some("bad".to_string()),
        };
        let s = serde_json::to_string(&out).unwrap();
        assert!(s.contains("\"ok\":false"));
        assert!(s.contains("\"error\":\"bad\""));
        assert!(!s.contains("\"data\""));
    }

    // T-CLI-RPC-06: event serialization.
    #[test]
    fn test_serialize_event() {
        let out = RpcOutput::Event {
            kind: "event",
            event: "turn_end",
            data: Some(serde_json::json!({"turn": 1})),
        };
        let s = serde_json::to_string(&out).unwrap();
        assert!(s.contains("\"type\":\"event\""));
        assert!(s.contains("\"event\":\"turn_end\""));
    }

    // T-CLI-RPC-07: invalid JSON type produces no panic (serialization safety).
    #[test]
    fn test_unknown_type_rejected() {
        let json = r#"{"type":"frobnicate","id":"r1"}"#;
        let result: std::result::Result<RpcRequest, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }
}
