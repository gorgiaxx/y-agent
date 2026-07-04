// The `ProvidesStaticType` derive from starlark generates code with explicit
// `'pst` lifetimes that clippy's `elidable_lifetime_names` flags. This is a
// macro-internal issue, not our code — suppress for the whole module.
#![allow(clippy::elidable_lifetime_names)]
//! Starlark-based command approval policy engine.
//!
//! This module implements a prefix-based command approval policy using a
//! Starlark DSL (the Python dialect used by Bazel). It is inspired by
//! Codex's `execpolicy` crate and adapted for y-agent's permission vocabulary.
//!
//! # Overview
//!
//! - [`ExecDecision`] — `Allow`, `Ask`, or `Deny` for a matched command
//! - [`PrefixRule`] — matches a command by its leading tokens
//! - [`Policy`] — indexed collection of rules with strictest-wins matching
//! - [`PolicyParser`] — parses Starlark policy files
//! - [`ExecPolicyManager`] — load, hot-reload, and amend policy files
//! - [`amend`] — append rules to a policy file (auto-derived amendments)
//!
//! # Policy file format
//!
//! ```python
//! prefix_rule(
//!     pattern = ["git", ["push", "commit"]],
//!     decision = "ask",
//!     match = [["git", "push"]],
//!     not_match = [["git", "status"]],
//!     justification = "review before push or commit",
//! )
//! ```
//!
//! # Auto-derived amendments
//!
//! When the user approves a command via HITL with "Always Allow", the system
//! derives a prefix rule that would auto-approve it next time:
//!
//! 1. [`ExecPolicyManager::propose_amendment`] returns the command tokens
//! 2. [`ExecPolicyManager::persist_amendment`] appends an `allow` rule to
//!    the policy file and hot-reloads
//!
//! # Integration
//!
//! The exec policy is consulted by `permission_pipeline` for `ShellExec`
//! tool calls. If a rule matches, its decision takes precedence over the
//! generic permission model.

pub mod amend;
pub mod decision;
pub mod error;
pub mod manager;
pub mod parser;
pub mod policy;
pub mod rule;

pub use decision::ExecDecision;
pub use error::{ExecPolicyError, ExecPolicyResult};
pub use manager::ExecPolicyManager;
pub use parser::PolicyParser;
pub use policy::{Evaluation, Policy};
pub use rule::{PatternToken, PrefixPattern, PrefixRule, Rule, RuleMatch, RuleRef};
