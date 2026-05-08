//! y-gui — Tauri v2 desktop application for y-agent.
//!
//! This crate embeds `y-service::ServiceContainer` in-process, exposing
//! Tauri commands as the bridge to the React frontend. LLM responses
//! are streamed via Tauri's native event system.

mod commands;
mod state;

use std::path::PathBuf;
use std::sync::atomic::Ordering;
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

fn setup_app(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
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
    let state_path = data_dir.clone().unwrap_or_else(|| PathBuf::from("."));
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
    let knowledge_state =
        commands::knowledge::KnowledgeState::from_shared(Arc::clone(&container.knowledge_service));

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

    let app_state = AppState::new(Arc::clone(&container), config_path.clone(), state_path);

    // Periodic sweep of stale pending_runs entries.
    // If an LLM worker panics before cleanup, its CancellationToken
    // remains in the map. This sweep removes entries older than 10 min.
    {
        let pending = Arc::clone(&app_state.pending_runs);
        rt.spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
            interval.tick().await; // skip first immediate tick
            loop {
                interval.tick().await;
                if let Ok(mut map) = pending.lock() {
                    let before = map.len();
                    map.retain(|_, token| !token.is_cancelled());
                    let removed = before - map.len();
                    if removed > 0 {
                        tracing::info!(
                            removed,
                            remaining = map.len(),
                            "swept stale pending_runs entries"
                        );
                    }
                }
            }
        });
    }

    // Apply the persisted window-decoration preference to the main
    // window before it is shown.
    // - macOS: switch title bar style between Overlay (custom) and
    //   Visible (native). Overlay keeps traffic lights on a layered
    //   chrome; Visible restores the standard macOS title bar.
    // - Linux/Windows: toggle native decorations so the frontend can
    //   draw its own chrome when the user opts in.
    if let Some(main_window) = app.get_webview_window("main") {
        let use_custom = rt
            .block_on(app_state.gui_config.read())
            .use_custom_decorations;

        #[cfg(target_os = "macos")]
        {
            use tauri::utils::config::WindowEffectsConfig;
            use tauri::utils::{WindowEffect, WindowEffectState};

            let style = if use_custom {
                tauri::TitleBarStyle::Overlay
            } else {
                tauri::TitleBarStyle::Visible
            };
            if let Err(e) = main_window.set_title_bar_style(style) {
                tracing::warn!(error = %e, "Failed to apply title bar style");
            }

            let effects = WindowEffectsConfig {
                effects: vec![WindowEffect::Sidebar],
                state: Some(WindowEffectState::FollowsWindowActiveState),
                radius: None,
                color: None,
            };
            if let Err(e) = main_window.set_effects(Some(effects)) {
                tracing::warn!(error = %e, "Failed to apply vibrancy effects");
            }
        }

        #[cfg(not(target_os = "macos"))]
        if let Err(e) = main_window.set_decorations(!use_custom) {
            tracing::warn!(error = %e, "Failed to apply window decoration preference");
        }
    }

    // Sync the translation target language from persisted GUI config
    // into the agent registry so the translator agent prompt is correct
    // on first use after launch.
    {
        let gui_cfg = rt.block_on(app_state.gui_config.read());
        let mut registry = rt.block_on(container.agent_registry.lock());
        registry.add_template_var(
            "{{TRANSLATE_TARGET_LANGUAGE}}".to_string(),
            gui_cfg.translate_target_language.clone(),
        );
    }

    // Webview health monitor: detect WKWebView content process
    // termination on macOS. The frontend sends a heartbeat_pong
    // every 15s. If no pong arrives for 120s after at least one
    // was received AND the window is visible+focused, assume
    // macOS killed the content process and reload the webview.
    // When the window is minimized or unfocused, macOS throttles
    // JS timers aggressively, so we skip the check to avoid
    // false-positive reloads.
    {
        let app_handle = app.handle().clone();
        let heartbeat = Arc::clone(&app_state.last_heartbeat_pong);
        rt.spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
            interval.tick().await;
            loop {
                interval.tick().await;
                let last_pong = heartbeat.load(Ordering::Relaxed);
                if last_pong == 0 {
                    continue;
                }
                let Some(window) = app_handle.get_webview_window("main") else {
                    continue;
                };
                let visible = window.is_visible().unwrap_or(false);
                let focused = window.is_focused().unwrap_or(false);
                if !visible || !focused {
                    continue;
                }
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let elapsed = now.saturating_sub(last_pong);
                if elapsed > 120 {
                    tracing::warn!(
                        elapsed_secs = elapsed,
                        "webview heartbeat timeout -- reloading webview"
                    );
                    if let Err(e) = window.eval("window.location.reload()") {
                        tracing::error!(error = %e, "failed to reload webview via eval");
                    }
                    heartbeat.store(0, Ordering::Relaxed);
                }
            }
        });
    }

    app.manage(app_state);
    app.manage(knowledge_state);

    Ok(())
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
        .setup(setup_app)
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
            commands::session::session_get_custom_prompt,
            commands::session::session_set_custom_prompt,
            commands::session::session_get_prompt_config,
            commands::session::session_set_prompt_config,
            commands::session::session_fork,
            commands::session::session_rename,
            // Diagnostics
            commands::diagnostics::diagnostics_get_by_session,
            commands::diagnostics::diagnostics_get_subagent_history,
            commands::diagnostics::diagnostics_clear_by_session,
            commands::diagnostics::diagnostics_clear_all,
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
            commands::config::prompt_template_list,
            commands::config::prompt_template_save,
            commands::config::prompt_template_delete,
            // MCP
            commands::config::mcp_config_get,
            commands::config::mcp_config_save,
            // System
            commands::system::system_status,
            commands::system::health_check,
            commands::system::provider_list,
            commands::system::show_window,
            commands::system::heartbeat_pong,
            commands::system::toggle_devtools,
            commands::system::window_set_decorations,
            commands::system::window_minimize,
            commands::system::window_toggle_maximize,
            commands::system::window_close,
            commands::system::window_set_theme,
            commands::system::app_paths,
            commands::system::memory_stats,
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
            commands::agents::agent_source_get,
            commands::agents::agent_toml_parse,
            commands::agents::agent_save,
            commands::agents::agent_reset,
            commands::agents::agent_reload,
            commands::agents::agent_tool_list,
            commands::agents::agent_prompt_section_list,
            commands::agents::translate_text,
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
            // Background tasks
            commands::background_tasks::background_task_list,
            commands::background_tasks::background_task_poll,
            commands::background_tasks::background_task_write,
            commands::background_tasks::background_task_kill,
            // Attachments
            commands::attachments::attachment_read_files,
            // Rewind (File History)
            commands::rewind::rewind_list_points,
            commands::rewind::rewind_execute,
            commands::rewind::rewind_restore_files,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
