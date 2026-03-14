//! y-gui — Tauri v2 desktop application for y-agent.
//!
//! This crate embeds `y-service::ServiceContainer` in-process, exposing
//! Tauri commands as the bridge to the React frontend. LLM responses
//! are streamed via Tauri's native event system.

mod commands;
mod state;

use std::path::PathBuf;
use std::sync::Arc;

use tauri::Manager;

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
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
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
            let rt = tokio::runtime::Runtime::new()
                .expect("Failed to create Tokio runtime");

            let container = rt.block_on(async {
                let config = ServiceConfig::load_from_directory(
                    &config_path,
                    state_dir().as_deref(),
                );
                ServiceContainer::from_config(&config)
                    .await
                    .expect("Failed to initialize ServiceContainer")
            });

            // Keep the runtime alive for async Tauri commands.
            // Leak it so it stays active for the app's entire lifetime.
            let rt = Box::leak(Box::new(rt));
            let _guard = rt.enter();

            let app_state = AppState::new(Arc::new(container), config_path);
            app.manage(app_state);

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
            // Sessions
            commands::session::session_list,
            commands::session::session_create,
            commands::session::session_get_messages,
            commands::session::session_delete,
            commands::session::session_truncate_messages,
            // Diagnostics
            commands::diagnostics::diagnostics_get_by_session,
            // Observability
            commands::observability::observability_snapshot,
            // Config
            commands::config::config_get,
            commands::config::config_set_section,
            commands::config::config_get_gui,
            commands::config::config_set_gui,
            commands::config::config_get_section,
            commands::config::config_save_section,
            commands::config::config_reload,
            commands::config::provider_test,
            commands::config::prompt_list,
            commands::config::prompt_get,
            commands::config::prompt_get_default,
            commands::config::prompt_save,
            // System
            commands::system::system_status,
            commands::system::health_check,
            commands::system::provider_list,
            commands::system::toggle_devtools,
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
