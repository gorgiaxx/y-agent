//! Permission mode manager: manages agent-level permission mode transitions.
//!
//! The mode manager owns the current `PermissionMode` and the shared
//! `PermissionRuleStore`, coordinating between them to produce
//! `PermissionContext` snapshots for the permission pipeline.
//!
//! Mode transitions emit tracing events for observability.

use std::sync::{Arc, RwLock};

use tracing::info;

use y_core::permission_types::{PermissionContext, PermissionMode};

use crate::error::GuardrailError;
use crate::rule_store::PermissionRuleStore;

/// Manages agent-level permission mode transitions and builds
/// `PermissionContext` snapshots for pipeline evaluation.
#[derive(Debug)]
pub struct PermissionModeManager {
    /// Current permission mode.
    current_mode: RwLock<PermissionMode>,
    /// Shared rule store.
    rule_store: Arc<RwLock<PermissionRuleStore>>,
}

impl PermissionModeManager {
    /// Create a new mode manager with the given rule store.
    ///
    /// The initial mode is taken from the rule store's configured default.
    pub fn new(rule_store: Arc<RwLock<PermissionRuleStore>>) -> Self {
        let default_mode = rule_store
            .read()
            .map(|s| s.default_mode())
            .unwrap_or(PermissionMode::Default);

        Self {
            current_mode: RwLock::new(default_mode),
            rule_store,
        }
    }

    /// Create a mode manager with an explicit initial mode.
    pub fn with_mode(rule_store: Arc<RwLock<PermissionRuleStore>>, mode: PermissionMode) -> Self {
        Self {
            current_mode: RwLock::new(mode),
            rule_store,
        }
    }

    pub fn current_mode(&self) -> PermissionMode {
        *self
            .current_mode
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Transition to a new permission mode.
    ///
    /// Validates the transition and emits a tracing event.
    pub fn transition(&self, to: PermissionMode) -> Result<(), GuardrailError> {
        let from = self.current_mode();

        // Validate transition.
        Self::validate_transition(from, to);

        let mut guard = self
            .current_mode
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *guard = to;

        info!(
            from = ?from,
            to = ?to,
            "permission mode transition"
        );

        Ok(())
    }

    /// Build the full `PermissionContext` for a tool permission check.
    ///
    /// Merges the current mode with all rules from the rule store.
    pub fn build_context(&self) -> PermissionContext {
        let mode = self.current_mode();
        let store = self
            .rule_store
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        store.build_context(Some(mode))
    }

    /// Get a reference to the shared rule store.
    pub fn rule_store(&self) -> &Arc<RwLock<PermissionRuleStore>> {
        &self.rule_store
    }

    /// Validate that a mode transition is allowed.
    fn validate_transition(_from: PermissionMode, _to: PermissionMode) {
        // All transitions are currently allowed. Future: add restrictions
        // (e.g., cannot transition from DontAsk to BypassPermissions without
        // explicit confirmation).
    }
}

#[cfg(test)]
mod tests {
    use y_core::permission_types::{
        PermissionBehavior, PermissionRule, PermissionRuleSource, PermissionRuleTarget,
    };

    use super::*;

    fn make_store() -> Arc<RwLock<PermissionRuleStore>> {
        let mut store = PermissionRuleStore::new();
        store.add_cli_allow("FileRead");
        Arc::new(RwLock::new(store))
    }

    #[test]
    fn test_initial_mode_from_store() {
        let store = Arc::new(RwLock::new(PermissionRuleStore::new()));
        let mgr = PermissionModeManager::new(store);
        assert_eq!(mgr.current_mode(), PermissionMode::Default);
    }

    #[test]
    fn test_initial_mode_explicit() {
        let store = Arc::new(RwLock::new(PermissionRuleStore::new()));
        let mgr = PermissionModeManager::with_mode(store, PermissionMode::Plan);
        assert_eq!(mgr.current_mode(), PermissionMode::Plan);
    }

    #[test]
    fn test_transition() {
        let store = make_store();
        let mgr = PermissionModeManager::new(store);

        assert_eq!(mgr.current_mode(), PermissionMode::Default);
        mgr.transition(PermissionMode::Plan).unwrap();
        assert_eq!(mgr.current_mode(), PermissionMode::Plan);
    }

    #[test]
    fn test_build_context() {
        let store = make_store();
        let mgr = PermissionModeManager::new(store);

        let ctx = mgr.build_context();
        assert_eq!(ctx.mode, PermissionMode::Default);
        assert_eq!(ctx.rules.len(), 1);
    }

    #[test]
    fn test_build_context_after_transition() {
        let store = make_store();
        let mgr = PermissionModeManager::new(store);

        mgr.transition(PermissionMode::BypassPermissions).unwrap();
        let ctx = mgr.build_context();
        assert_eq!(ctx.mode, PermissionMode::BypassPermissions);
    }

    #[test]
    fn test_rule_store_mutation_reflected() {
        let store = make_store();
        let mgr = PermissionModeManager::new(store);

        // Add a session rule through the rule store.
        {
            let mut s = mgr.rule_store().write().unwrap();
            s.add_session_rule(PermissionRule::new(
                PermissionRuleSource::Session,
                PermissionBehavior::Deny,
                PermissionRuleTarget::tool("ShellExec"),
            ));
        }

        let ctx = mgr.build_context();
        assert_eq!(ctx.rules.len(), 2); // CLI allow + session deny
    }
}
