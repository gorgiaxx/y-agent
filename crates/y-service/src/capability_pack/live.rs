use std::collections::HashMap;
use std::sync::Arc;

#[cfg(feature = "lsp")]
use super::owner::parse_lsp_declaration;
use super::owner::{parse_hook_declaration, parse_mcp_declaration};
use crate::container::ServiceContainer;
use crate::mcp_service::McpService;

pub(crate) struct CapabilityPackLiveOwners;

impl CapabilityPackLiveOwners {
    pub(crate) async fn activate(
        container: &Arc<ServiceContainer>,
        resource_keys: &[String],
    ) -> Result<Vec<String>, String> {
        let mut keys = resource_keys.to_vec();
        keys.sort();
        #[cfg(not(feature = "lsp"))]
        if let Some(unsupported) = keys.iter().find(|key| key.starts_with("lsp:")) {
            return Err(format!(
                "live activation owner is not implemented for {unsupported}"
            ));
        }

        let mut activated = Vec::new();
        let mut newly_activated: Vec<String> = Vec::new();
        for key in keys {
            let was_active = if let Some(id) = key.strip_prefix("mcp:") {
                container
                    .active_capability_pack_mcp
                    .read()
                    .await
                    .contains_key(id)
            } else if let Some(id) = key.strip_prefix("hook:") {
                container
                    .active_capability_pack_hooks
                    .read()
                    .map_err(|_| "active capability-pack hook lock is poisoned".to_string())?
                    .contains_key(id)
            } else if let Some(id) = key.strip_prefix("lsp:") {
                #[cfg(feature = "lsp")]
                {
                    container
                        .lsp_manager
                        .as_ref()
                        .is_some_and(|manager| manager.has_dynamic_server(id))
                }
                #[cfg(not(feature = "lsp"))]
                {
                    let _ = id;
                    false
                }
            } else {
                continue;
            };
            if let Err(error) = activate_one(container, &key).await {
                for activated_key in newly_activated.into_iter().rev() {
                    let _ = deactivate_one(container, &activated_key).await;
                }
                return Err(error);
            }
            if !was_active {
                newly_activated.push(key.clone());
            }
            activated.push(key);
        }
        Ok(activated)
    }

    pub(crate) async fn deactivate(
        container: &ServiceContainer,
        resource_keys: &[String],
    ) -> Result<Vec<String>, String> {
        let mut keys = resource_keys.to_vec();
        keys.sort();
        let mut deactivated = Vec::new();
        for key in keys {
            if deactivate_one(container, &key).await? {
                deactivated.push(key);
            }
        }
        Ok(deactivated)
    }
}

async fn activate_one(container: &Arc<ServiceContainer>, key: &str) -> Result<(), String> {
    if let Some(id) = key.strip_prefix("mcp:") {
        let declaration = declaration_path(container, "mcp", id);
        let config = parse_mcp_declaration(&declaration, id)?;
        return McpService::activate_capability_pack_server(container, config).await;
    }
    if let Some(id) = key.strip_prefix("hook:") {
        return activate_hook(container, id);
    }
    #[cfg(feature = "lsp")]
    if let Some(id) = key.strip_prefix("lsp:") {
        let manager = container
            .lsp_manager
            .as_ref()
            .ok_or_else(|| "LSP support is disabled by user configuration".to_string())?;
        let config = parse_lsp_declaration(&declaration_path(container, "lsp", id), id)?;
        manager.register_dynamic_server(config)?;
        return Ok(());
    }
    Ok(())
}

async fn deactivate_one(container: &ServiceContainer, key: &str) -> Result<bool, String> {
    if let Some(id) = key.strip_prefix("mcp:") {
        return McpService::deactivate_capability_pack_server(container, id).await;
    }
    if let Some(id) = key.strip_prefix("hook:") {
        return deactivate_hook(container, id);
    }
    #[cfg(feature = "lsp")]
    if let Some(id) = key.strip_prefix("lsp:") {
        let Some(manager) = &container.lsp_manager else {
            return Ok(false);
        };
        return manager.unregister_dynamic_server(id).await;
    }
    Ok(false)
}

fn activate_hook(container: &ServiceContainer, id: &str) -> Result<(), String> {
    let declaration = parse_hook_declaration(&declaration_path(container, "hook", id))?;
    #[cfg(not(all(feature = "hook_handlers", feature = "llm_hooks")))]
    if declaration.handlers.iter().any(|handler| {
        matches!(
            handler,
            y_hooks::config::HandlerConfig::Prompt { .. }
                | y_hooks::config::HandlerConfig::Agent { .. }
        )
    }) {
        return Err(
            "prompt and agent hook activation requires hook_handlers and llm_hooks".to_string(),
        );
    }
    let base = container
        .capability_pack_hook_base
        .read()
        .map_err(|_| "capability-pack hook base lock is poisoned".to_string())?
        .clone();
    if !base.handlers_enabled {
        return Err("hook handlers are disabled by user configuration".to_string());
    }
    let mut overlay_handlers = HashMap::new();
    overlay_handlers.insert(
        declaration.hook_point,
        vec![y_hooks::config::HookHandlerGroupConfig {
            matcher: declaration.matcher,
            timeout_ms: declaration.timeout_ms,
            handlers: declaration.handlers,
        }],
    );
    let overlay = y_hooks::HookConfig {
        hook_handlers: overlay_handlers,
        ..y_hooks::HookConfig::default()
    };
    let mut candidate = container
        .active_capability_pack_hooks
        .read()
        .map_err(|_| "active capability-pack hook lock is poisoned".to_string())?
        .clone();
    candidate.insert(id.to_string(), overlay);
    let effective = compose_hook_config(&base, &candidate);
    validate_effective_hook_config(&effective)?;
    *container
        .active_capability_pack_hooks
        .write()
        .map_err(|_| "active capability-pack hook lock is poisoned".to_string())? = candidate;
    container
        .hook_system
        .write()
        .map_err(|_| "hook system lock is poisoned".to_string())?
        .reload_config(&effective);
    Ok(())
}

fn deactivate_hook(container: &ServiceContainer, id: &str) -> Result<bool, String> {
    let base = container
        .capability_pack_hook_base
        .read()
        .map_err(|_| "capability-pack hook base lock is poisoned".to_string())?
        .clone();
    let mut candidate = container
        .active_capability_pack_hooks
        .read()
        .map_err(|_| "active capability-pack hook lock is poisoned".to_string())?
        .clone();
    if candidate.remove(id).is_none() {
        return Ok(false);
    }
    let effective = compose_hook_config(&base, &candidate);
    validate_effective_hook_config(&effective)?;
    *container
        .active_capability_pack_hooks
        .write()
        .map_err(|_| "active capability-pack hook lock is poisoned".to_string())? = candidate;
    container
        .hook_system
        .write()
        .map_err(|_| "hook system lock is poisoned".to_string())?
        .reload_config(&effective);
    Ok(true)
}

pub(crate) fn compose_hook_config(
    base: &y_hooks::HookConfig,
    overlays: &HashMap<String, y_hooks::HookConfig>,
) -> y_hooks::HookConfig {
    let mut effective = base.clone();
    let mut ids = overlays.keys().collect::<Vec<_>>();
    ids.sort();
    for id in ids {
        if let Some(overlay) = overlays.get(id) {
            let mut hook_points = overlay.hook_handlers.keys().collect::<Vec<_>>();
            hook_points.sort();
            for hook_point in hook_points {
                if let Some(groups) = overlay.hook_handlers.get(hook_point) {
                    effective
                        .hook_handlers
                        .entry(hook_point.clone())
                        .or_default()
                        .extend(groups.clone());
                }
            }
        }
    }
    effective
}

fn validate_effective_hook_config(config: &y_hooks::HookConfig) -> Result<(), String> {
    y_hooks::config::validate_hook_handler_config(config)
        .map_err(|error| format!("invalid effective hook configuration: {error}"))?;
    if config.handlers_enabled && !config.hook_handlers.is_empty() {
        y_hooks::HookHandlerExecutor::from_config(config)
            .map(|_| ())
            .map_err(|error| format!("invalid effective hook configuration: {error}"))
    } else {
        Ok(())
    }
}

fn declaration_path(container: &ServiceContainer, kind: &str, id: &str) -> std::path::PathBuf {
    container
        .data_dir
        .join("capability-packs/declarations")
        .join(kind)
        .join(format!("{id}.toml"))
}
