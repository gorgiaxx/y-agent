//! y-gui — Tauri v2 desktop application for y-agent.
//!
//! This crate embeds `y-service::ServiceContainer` in-process, exposing
//! Tauri commands as the bridge to the React frontend. LLM responses
//! are streamed via Tauri's native event system.

mod commands;
mod state;

use std::path::PathBuf;
use std::sync::Arc;

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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            // Logging (debug builds only).
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }

            // Initialize the service container on the Tokio runtime.
            let config_path = config_dir();
            let rt = tokio::runtime::Handle::current();

            let container = rt.block_on(async {
                let config = ServiceConfig::default();
                ServiceContainer::from_config(&config)
                    .await
                    .expect("Failed to initialize ServiceContainer")
            });

            let app_state = AppState::new(Arc::new(container), config_path);
            app.manage(app_state);

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // Chat
            commands::chat::chat_send,
            // Sessions
            commands::session::session_list,
            commands::session::session_create,
            commands::session::session_get_messages,
            commands::session::session_delete,
            // Config
            commands::config::config_get,
            commands::config::config_set_section,
            commands::config::config_get_gui,
            commands::config::config_set_gui,
            // System
            commands::system::system_status,
            commands::system::health_check,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
