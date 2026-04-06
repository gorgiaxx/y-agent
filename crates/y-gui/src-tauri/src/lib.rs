//! y-gui — Tauri v2 desktop application for y-agent.
//!
//! This crate embeds `y-service::ServiceContainer` in-process, exposing
//! Tauri commands as the bridge to the React frontend. LLM responses
//! are streamed via Tauri's native event system.

mod commands;
mod state;

use std::path::PathBuf;
use std::sync::Arc;

use tauri::{Emitter, Manager};

use y_service::{ServiceConfig, ServiceContainer};

use crate::state::AppState;

/// Resolve the user config directory (`~/.config/y-agent/`).
fn config_dir() -> PathBuf {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .expect("HOME not set");
    home.join(".config").join("y-agent")
}

/// Get the XDG state base directory for y-agent (`~/.local/state/y-agent/`).
fn state_dir() -> Option<PathBuf> {
    let state_home = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .map(|h| PathBuf::from(h).join(".local").join("state"))
        });
    state_home.map(|s| s.join("y-agent"))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
/// Launch the Tauri desktop application.
///
/// # Panics
///
/// Panics if the Tokio runtime, `ServiceContainer`, or Tauri application
/// fails to initialise.
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // Logging (debug builds only).
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }

            // Initialize the service container.
            // Tauri's setup runs on the main thread without a Tokio runtime,
            // so we create a temporary one for async initialization.
            let config_path = config_dir();
            let data_dir = state_dir();
            let rt = tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");

            // First-run auto-init: seed configs, prompts, skills, agents
            // if they don't already exist. This makes the GUI work
            // out-of-the-box without requiring `y-agent init`.
            if let Some(ref dd) = data_dir {
                // Determine bundled skills path from Tauri resources.
                let skills_source = app
                    .path()
                    .resource_dir()
                    .ok()
                    .map(|p| p.join("skills"))
                    .filter(|p| p.is_dir());

                if let Err(e) =
                    y_service::init::ensure_initialized(&config_path, dd, skills_source.as_deref())
                {
                    tracing::warn!(error = %e, "Auto-init failed; continuing with defaults");
                }
            }

            let container = rt.block_on(async {
                let config = ServiceConfig::load_from_directory(&config_path, data_dir.as_deref());
                let container = ServiceContainer::from_config(&config)
                    .await
                    .expect("Failed to initialize ServiceContainer");

                // NOTE: KnowledgeSearchTool and KnowledgeContextProvider are
                // both registered by ServiceContainer::from_config (with
                // embedding support if configured).

                container
            });

            // Create KnowledgeState wrapping the container's shared knowledge
            // service. This ensures the GUI knowledge panel, context pipeline,
            // and `KnowledgeSearch` tool all operate on the same KnowledgeService
            // instance (with embedding provider if configured).
            let knowledge_state = commands::knowledge::KnowledgeState::from_shared(Arc::clone(
                &container.knowledge_service,
            ));

            // Keep the runtime alive for async Tauri commands.
            // Leak it so it stays active for the app's entire lifetime.
            let rt = Box::leak(Box::new(rt));
            let _guard = rt.enter();

            let container = Arc::new(container);

            // Upgrade sub-agent runner from SingleTurnRunner to
            // ServiceAgentRunner so delegated agents (skill-ingestion, etc.)
            // get the full execution loop with multi-turn tool calling.
            rt.block_on(container.start_background_services());

            // Spawn a background task that bridges the diagnostics broadcast
            // channel to Tauri events. This enables real-time diagnostics
            // for ALL agent executions (knowledge import, skill import, etc.)
            // without per-caller manual wiring.
            {
                let mut rx = container.diagnostics_broadcast.subscribe();
                let app_handle = app.handle().clone();
                rt.spawn(async move {
                    loop {
                        match rx.recv().await {
                            Ok(event) => {
                                let _ = app_handle.emit("diagnostics:event", &event);
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                                tracing::warn!(
                                    skipped = n,
                                    "diagnostics broadcast bridge lagged -- events dropped"
                                );
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                                tracing::debug!("diagnostics broadcast channel closed");
                                break;
                            }
                        }
                    }
                });
            }

            let app_state = AppState::new(Arc::clone(&container), config_path.clone());
            app.manage(app_state);
            app.manage(knowledge_state);

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // Chat
            commands::chat::chat_send,
            commands::chat::chat_cancel,
            commands::chat::session_last_turn_meta,
            commands::chat::chat_undo,
            commands::chat::chat_checkpoint_list,
            commands::chat::chat_get_messages_with_status,
            commands::chat::chat_restore_branch,
            commands::chat::chat_resend,
            commands::chat::chat_find_checkpoint_for_resend,
            commands::chat::context_compact,
            commands::chat::chat_answer_question,
            commands::chat::chat_answer_permission,
            // Sessions
            commands::session::session_list,
            commands::session::session_create,
            commands::session::session_get_messages,
            commands::session::session_delete,
            commands::session::session_truncate_messages,
            commands::session::session_get_context_reset,
            commands::session::session_set_context_reset,
            commands::session::session_fork,
            // Diagnostics
            commands::diagnostics::diagnostics_get_by_session,
            commands::diagnostics::diagnostics_get_subagent_history,
            // Observability
            commands::observability::observability_snapshot,
            commands::observability::observability_history,
            // Config
            commands::config::config_get,
            commands::config::config_set_section,
            commands::config::config_get_gui,
            commands::config::config_set_gui,
            commands::config::config_get_section,
            commands::config::config_save_section,
            commands::config::config_reload,
            commands::config::provider_test,
            commands::config::provider_list_models,
            commands::config::prompt_list,
            commands::config::prompt_get,
            commands::config::prompt_get_default,
            commands::config::prompt_save,
            // MCP
            commands::config::mcp_config_get,
            commands::config::mcp_config_save,
            // System
            commands::system::system_status,
            commands::system::health_check,
            commands::system::provider_list,
            commands::system::show_window,
            commands::system::toggle_devtools,
            commands::system::app_paths,
            // Workspaces
            commands::workspace::workspace_list,
            commands::workspace::workspace_create,
            commands::workspace::workspace_update,
            commands::workspace::workspace_delete,
            commands::workspace::workspace_session_map,
            commands::workspace::workspace_assign_session,
            commands::workspace::workspace_unassign_session,
            // Skills
            commands::skills::skill_list,
            commands::skills::skill_get,
            commands::skills::skill_uninstall,
            commands::skills::skill_set_enabled,
            commands::skills::skill_open_folder,
            commands::skills::skill_import,
            commands::skills::skill_get_files,
            commands::skills::skill_read_file,
            commands::skills::skill_save_file,
            // Knowledge
            commands::knowledge::kb_collection_list,
            commands::knowledge::kb_collection_create,
            commands::knowledge::kb_collection_delete,
            commands::knowledge::kb_collection_rename,
            commands::knowledge::kb_entry_list,
            commands::knowledge::kb_entry_detail,
            commands::knowledge::kb_search,
            commands::knowledge::kb_ingest,
            commands::knowledge::kb_entry_delete,
            commands::knowledge::kb_stats,
            commands::knowledge::kb_expand_folder,
            commands::knowledge::kb_ingest_batch,
            commands::knowledge::kb_entry_update_metadata,
            // Agents
            commands::agents::agent_list,
            commands::agents::agent_get,
            commands::agents::agent_save,
            commands::agents::agent_reset,
            commands::agents::agent_reload,
            // Automation: Workflows
            commands::automation::workflow_list,
            commands::automation::workflow_get,
            commands::automation::workflow_create,
            commands::automation::workflow_update,
            commands::automation::workflow_delete,
            commands::automation::workflow_validate,
            commands::automation::workflow_dag,
            // Automation: Schedules
            commands::automation::schedule_list,
            commands::automation::schedule_get,
            commands::automation::schedule_create,
            commands::automation::schedule_update,
            commands::automation::schedule_delete,
            commands::automation::schedule_pause,
            commands::automation::schedule_resume,
            // Automation: Execution History
            commands::automation::schedule_execution_history,
            commands::automation::schedule_execution_get,
            commands::automation::schedule_trigger_now,
            commands::automation::workflow_execute,
            // Attachments
            commands::attachments::attachment_read_files,
            // Rewind (File History)
            commands::rewind::rewind_list_points,
            commands::rewind::rewind_execute,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
