//! [`ExecPolicyManager`]: load, hot-reload, and amend Starlark policy files.
//!
//! The manager holds the current [`Policy`] behind an `RwLock` for
//! lock-free concurrent reads and serialized updates. It supports:
//!
//! - Loading policy files from disk
//! - Hot-reloading after amendments
//! - Proposing auto-derived amendments from HITL decisions
//! - Checking commands against the current policy

use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::RwLock;
use tokio::sync::Semaphore;

use crate::exec_policy::amend::append_allow_prefix_rule;
use crate::exec_policy::decision::ExecDecision;
use crate::exec_policy::error::{ExecPolicyError, ExecPolicyResult};
use crate::exec_policy::parser::PolicyParser;
use crate::exec_policy::policy::{Evaluation, Policy};

/// Manages an exec policy: load, hot-reload, and amend.
///
/// Holds the current [`Policy`] behind an `RwLock` for concurrent reads.
/// Amendments are serialized via a `Semaphore` to prevent concurrent writes.
pub struct ExecPolicyManager {
    /// Current policy (hot-reloadable).
    policy: RwLock<Arc<Policy>>,
    /// Path to the policy file on disk.
    policy_path: Option<PathBuf>,
    /// Serializes amendment writes.
    update_lock: Semaphore,
}

impl ExecPolicyManager {
    /// Create a manager with an empty policy (no rules).
    pub fn empty() -> Self {
        Self {
            policy: RwLock::new(Arc::new(Policy::default())),
            policy_path: None,
            update_lock: Semaphore::new(1),
        }
    }

    /// Create a manager and load a policy file from disk.
    pub fn from_file(path: impl AsRef<Path>) -> ExecPolicyResult<Self> {
        let path = path.as_ref();
        let policy = load_policy_file(path)?;
        Ok(Self {
            policy: RwLock::new(Arc::new(policy)),
            policy_path: Some(path.to_path_buf()),
            update_lock: Semaphore::new(1),
        })
    }

    /// Create a manager from an in-memory policy string.
    pub fn from_str(identifier: &str, contents: &str) -> ExecPolicyResult<Self> {
        let mut parser = PolicyParser::new();
        parser.parse(identifier, contents)?;
        let policy = parser.build();
        Ok(Self {
            policy: RwLock::new(Arc::new(policy)),
            policy_path: None,
            update_lock: Semaphore::new(1),
        })
    }

    /// Get a snapshot of the current policy.
    pub fn policy(&self) -> Arc<Policy> {
        Arc::clone(&self.policy.read())
    }

    /// Evaluate `cmd` against the current policy.
    ///
    /// Returns `None` if no rule matches (caller should apply fallback).
    pub fn evaluate(&self, cmd: &[String]) -> Option<Evaluation> {
        self.policy().evaluate(cmd)
    }

    /// Reload the policy from disk.
    ///
    /// Acquires the update lock, re-reads the file, and swaps in the new
    /// policy atomically.
    pub async fn reload(&self) -> ExecPolicyResult<()> {
        let path = self
            .policy_path
            .as_ref()
            .ok_or_else(|| ExecPolicyError::InvalidRule("no policy path set".to_string()))?;
        let _permit = self
            .update_lock
            .acquire()
            .await
            .map_err(|e| ExecPolicyError::InvalidRule(format!("update lock: {e}")))?;

        let path = path.clone();
        let policy = tokio::task::spawn_blocking(move || load_policy_file(&path))
            .await
            .map_err(|e| ExecPolicyError::InvalidRule(format!("join error: {e}")))??;

        *self.policy.write() = Arc::new(policy);
        Ok(())
    }

    /// Propose an auto-derived amendment for a command that needs approval.
    ///
    /// Returns the proposed prefix (the command tokens) that would be appended
    /// as an `allow` rule if the user chooses "Always Allow".
    pub fn propose_amendment(cmd: &[String]) -> Vec<String> {
        // Use the full command as the prefix — this is conservative.
        // A more sophisticated approach would use just the first N tokens.
        cmd.to_vec()
    }

    /// Persist an amendment: append an `allow` prefix rule to the policy file
    /// and hot-reload.
    ///
    /// This is the "Always Allow" path from HITL. The `prefix` is typically
    /// [`propose_amendment`](Self::propose_amendment) for the command that was
    /// just approved.
    pub async fn persist_amendment(&self, prefix: Vec<String>) -> ExecPolicyResult<()> {
        let path = self
            .policy_path
            .as_ref()
            .ok_or_else(|| ExecPolicyError::InvalidRule("no policy path set".to_string()))?;

        let _permit = self
            .update_lock
            .acquire()
            .await
            .map_err(|e| ExecPolicyError::InvalidRule(format!("update lock: {e}")))?;

        let path = path.clone();
        tokio::task::spawn_blocking(move || append_allow_prefix_rule(&path, &prefix))
            .await
            .map_err(|e| ExecPolicyError::InvalidRule(format!("join error: {e}")))??;

        // Reload to pick up the new rule.
        // We already hold the permit, so we read the file directly rather than
        // calling self.reload() which would try to re-acquire.
        let path = self
            .policy_path
            .as_ref()
            .ok_or_else(|| ExecPolicyError::InvalidRule("no policy path set".to_string()))?
            .clone();
        let policy = tokio::task::spawn_blocking(move || load_policy_file(&path))
            .await
            .map_err(|e| ExecPolicyError::InvalidRule(format!("join error: {e}")))??;

        *self.policy.write() = Arc::new(policy);
        Ok(())
    }

    /// Check if a command is already allowed by an explicit rule (not heuristics).
    ///
    /// Returns `true` only if a rule with `Allow` decision matches.
    pub fn is_explicitly_allowed(&self, cmd: &[String]) -> bool {
        self.evaluate(cmd)
            .is_some_and(|e| e.decision == ExecDecision::Allow)
    }

    /// Check if a command is explicitly denied.
    pub fn is_explicitly_denied(&self, cmd: &[String]) -> bool {
        self.evaluate(cmd)
            .is_some_and(|e| e.decision == ExecDecision::Deny)
    }
}

impl Default for ExecPolicyManager {
    fn default() -> Self {
        Self::empty()
    }
}

impl std::fmt::Debug for ExecPolicyManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecPolicyManager")
            .field("policy_path", &self.policy_path)
            .field("rule_count", &self.policy.read().rule_count())
            .finish_non_exhaustive()
    }
}

/// Load and parse a policy file from disk.
fn load_policy_file(path: &Path) -> ExecPolicyResult<Policy> {
    let contents = std::fs::read_to_string(path).map_err(|source| ExecPolicyError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut parser = PolicyParser::new();
    parser.parse(path.to_str().unwrap_or("policy"), &contents)?;
    Ok(parser.build())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_manager_no_match() {
        let mgr = ExecPolicyManager::empty();
        assert!(mgr.evaluate(&["git".into()]).is_none());
    }

    #[test]
    fn from_str_loads_policy() {
        let mgr = ExecPolicyManager::from_str(
            "test",
            r#"prefix_rule(pattern = ["ls"], decision = "allow")"#,
        )
        .unwrap();
        let eval = mgr.evaluate(&["ls".into()]).unwrap();
        assert_eq!(eval.decision, ExecDecision::Allow);
    }

    #[test]
    fn from_file_loads_policy() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("test.policy");
        std::fs::write(
            &path,
            r#"prefix_rule(pattern = ["cat"], decision = "allow")"#,
        )
        .unwrap();
        let mgr = ExecPolicyManager::from_file(&path).unwrap();
        assert!(mgr.is_explicitly_allowed(&["cat".into()]));
    }

    #[tokio::test]
    async fn persist_amendment_appends_and_reloads() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("test.policy");
        std::fs::write(
            &path,
            r#"prefix_rule(pattern = ["ls"], decision = "allow")"#,
        )
        .unwrap();

        let mgr = ExecPolicyManager::from_file(&path).unwrap();
        // Initially, cargo test is not allowed.
        assert!(!mgr.is_explicitly_allowed(&["cargo".into(), "test".into()]));

        // Persist amendment.
        mgr.persist_amendment(vec!["cargo".into(), "test".into()])
            .await
            .unwrap();

        // Now it should be allowed.
        assert!(mgr.is_explicitly_allowed(&["cargo".into(), "test".into()]));
        // And the original rule should still be there.
        assert!(mgr.is_explicitly_allowed(&["ls".into()]));
    }

    #[tokio::test]
    async fn reload_picks_up_external_changes() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("test.policy");
        std::fs::write(
            &path,
            r#"prefix_rule(pattern = ["ls"], decision = "allow")"#,
        )
        .unwrap();

        let mgr = ExecPolicyManager::from_file(&path).unwrap();
        assert!(!mgr.is_explicitly_allowed(&["cat".into()]));

        // External edit.
        std::fs::write(
            &path,
            r#"prefix_rule(pattern = ["ls"], decision = "allow")
prefix_rule(pattern = ["cat"], decision = "allow")
"#,
        )
        .unwrap();

        mgr.reload().await.unwrap();
        assert!(mgr.is_explicitly_allowed(&["cat".into()]));
    }

    #[test]
    fn propose_amendment_returns_full_command() {
        let cmd = vec![
            "npm".to_string(),
            "install".to_string(),
            "react".to_string(),
        ];
        let proposed = ExecPolicyManager::propose_amendment(&cmd);
        assert_eq!(proposed, cmd);
    }

    #[test]
    fn is_explicitly_denied() {
        let mgr = ExecPolicyManager::from_str(
            "test",
            r#"prefix_rule(pattern = ["rm"], decision = "deny")"#,
        )
        .unwrap();
        assert!(mgr.is_explicitly_denied(&["rm".into()]));
        assert!(!mgr.is_explicitly_denied(&["ls".into()]));
    }
}
