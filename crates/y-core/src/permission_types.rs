//! Permission types shared across y-agent crates.
//!
//! This module defines the core permission vocabulary: behaviors, rules,
//! rule targets, modes, and decision contexts. These types are used by
//! `y-guardrails` for evaluation, `y-tools` for tool-level defaults,
//! and `y-service` for persistence and wiring.
//!
//! Design reference: `docs/design/guardrails-hitl-design.md`

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Permission behaviors
// ---------------------------------------------------------------------------

/// The behavior a permission check can return.
///
/// Ordered from most permissive to most restrictive:
/// `Allow` > `Passthrough` > `Ask` > `Deny`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionBehavior {
    /// Execute immediately, no restrictions.
    Allow,
    /// Tool has no opinion -- defer to the general permission system.
    /// Converted to `Ask` at the end of the pipeline if no rule allows it.
    Passthrough,
    /// Pause execution and ask the user via HITL.
    Ask,
    /// Block execution entirely.
    Deny,
}

// ---------------------------------------------------------------------------
// Permission rule target
// ---------------------------------------------------------------------------

/// Target specifier for a permission rule.
///
/// Rules can target an entire tool (`tool_name` only) or a specific
/// content pattern within a tool (e.g., `ShellExec(npm install:*)`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PermissionRuleTarget {
    /// Tool name (e.g., "`ShellExec`").
    pub tool_name: String,
    /// Optional content pattern for content-specific rules.
    ///
    /// When `None`, the rule applies to the entire tool.
    /// When `Some`, matches against tool-specific content:
    /// - `ShellExec`: command prefix (e.g., "npm install:*")
    /// - `FileWrite`: path pattern (e.g., "/tmp/*")
    /// - `Browser`: URL pattern (e.g., "<https://docs.rs>/*")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_pattern: Option<String>,
}

impl PermissionRuleTarget {
    /// Create a target for the entire tool.
    pub fn tool(name: impl Into<String>) -> Self {
        Self {
            tool_name: name.into(),
            content_pattern: None,
        }
    }

    /// Create a target with a content pattern.
    pub fn with_pattern(name: impl Into<String>, pattern: impl Into<String>) -> Self {
        Self {
            tool_name: name.into(),
            content_pattern: Some(pattern.into()),
        }
    }

    /// Check if this target matches a given tool name without content.
    pub fn matches_tool(&self, tool_name: &str) -> bool {
        self.tool_name == tool_name && self.content_pattern.is_none()
    }

    /// Check if this target matches a given tool name and content.
    ///
    /// A tool-level target (no pattern) matches all content for that tool.
    /// A content-specific target matches if the content matches the pattern:
    /// - Exact match: `"npm install"` matches `"npm install"`
    /// - Prefix match with `:*`: `"npm install:*"` matches `"npm install --save foo"`
    /// - Wildcard `*`: matches everything
    pub fn matches(&self, tool_name: &str, content: Option<&str>) -> bool {
        if self.tool_name != tool_name {
            return false;
        }

        match (&self.content_pattern, content) {
            // Tool-level rule (no pattern) matches everything for that tool.
            (None, _) => true,
            // Content-specific rule but no content provided -- no match.
            (Some(_), None) => false,
            // Content-specific matching.
            (Some(pattern), Some(content)) => match_content_pattern(pattern, content),
        }
    }

    /// Format this target as a rule string (e.g., "`ShellExec`" or "ShellExec(npm:*)").
    pub fn to_rule_string(&self) -> String {
        match &self.content_pattern {
            None => self.tool_name.clone(),
            Some(pattern) => format!("{}({})", self.tool_name, pattern),
        }
    }
}

impl std::fmt::Display for PermissionRuleTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_rule_string())
    }
}

/// Match content against a pattern string.
///
/// Supports:
/// - `*` -- matches everything
/// - `prefix:*` -- matches content starting with `prefix`
/// - exact match
fn match_content_pattern(pattern: &str, content: &str) -> bool {
    if pattern == "*" {
        return true;
    }

    if let Some(prefix) = pattern.strip_suffix(":*") {
        return content.starts_with(prefix);
    }

    pattern == content
}

// ---------------------------------------------------------------------------
// Permission rule source
// ---------------------------------------------------------------------------

/// Where a permission rule originated.
///
/// Ordered by precedence (highest first):
/// `Session` > `CliArg` > `ProjectSettings` > `GlobalSettings` > `AgentConfig`
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionRuleSource {
    /// Global settings file (`~/.y-agent/y-agent.toml`).
    GlobalSettings,
    /// Project settings file (`.y-agent/y-agent.toml` in project root).
    ProjectSettings,
    /// CLI argument (`--allow-tool`, `--deny-tool`).
    CliArg,
    /// In-memory session rule (not persisted to disk).
    Session,
    /// Agent-level configuration (from the agent definition).
    AgentConfig,
}

impl PermissionRuleSource {
    /// Whether rules from this source can be persisted to disk.
    pub fn is_persistable(&self) -> bool {
        matches!(
            self,
            PermissionRuleSource::GlobalSettings | PermissionRuleSource::ProjectSettings
        )
    }

    /// Precedence rank (lower = higher priority).
    pub fn precedence(&self) -> u8 {
        match self {
            PermissionRuleSource::Session => 0,
            PermissionRuleSource::CliArg => 1,
            PermissionRuleSource::ProjectSettings => 2,
            PermissionRuleSource::GlobalSettings => 3,
            PermissionRuleSource::AgentConfig => 4,
        }
    }
}

// ---------------------------------------------------------------------------
// Permission rule
// ---------------------------------------------------------------------------

/// A persistent permission rule.
///
/// Rules are the primary mechanism for controlling tool behavior.
/// They are loaded from config files, CLI args, or the interactive session,
/// and evaluated in precedence order during the permission pipeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionRule {
    /// Source of this rule.
    pub source: PermissionRuleSource,
    /// The behavior this rule enforces.
    pub behavior: PermissionBehavior,
    /// Which tool (and optionally which content) this rule targets.
    pub target: PermissionRuleTarget,
}

impl PermissionRule {
    /// Create a new permission rule.
    pub fn new(
        source: PermissionRuleSource,
        behavior: PermissionBehavior,
        target: PermissionRuleTarget,
    ) -> Self {
        Self {
            source,
            behavior,
            target,
        }
    }
}

impl std::fmt::Display for PermissionRule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:?}({:?}) -> {}",
            self.behavior, self.source, self.target
        )
    }
}

// ---------------------------------------------------------------------------
// Permission mode
// ---------------------------------------------------------------------------

/// Agent-level permission mode controlling the overall behavior.
///
/// Modes affect how the permission pipeline resolves `Ask` and `Passthrough`
/// behaviors. They do NOT override explicit `Deny` rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    /// Normal mode -- each tool evaluated per its rules.
    /// `Passthrough` becomes `Ask` unless an allow rule matches.
    #[default]
    Default,
    /// Plan mode -- read-only tools allowed, write tools ask.
    Plan,
    /// Accept edits mode -- file edits auto-allowed, shell still asks.
    AcceptEdits,
    /// Bypass all permissions (dangerous).
    /// `Passthrough` and `Ask` become `Allow`. `Deny` rules still enforced.
    BypassPermissions,
    /// Auto-deny instead of asking (headless/background agents).
    /// `Ask` and `Passthrough` become `Deny`.
    DontAsk,
}

impl std::fmt::Display for PermissionMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            PermissionMode::Default => "default",
            PermissionMode::Plan => "plan",
            PermissionMode::AcceptEdits => "accept_edits",
            PermissionMode::BypassPermissions => "bypass_permissions",
            PermissionMode::DontAsk => "dont_ask",
        };
        write!(f, "{s}")
    }
}

// ---------------------------------------------------------------------------
// Permission decision reason
// ---------------------------------------------------------------------------

/// Why a permission decision was made -- for audit and UI display.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PermissionReason {
    /// Matched a persistent rule.
    Rule {
        /// The rule that was matched.
        rule_display: String,
    },
    /// The tool's own `check_permissions()` returned this behavior.
    ToolCheck {
        /// Explanation from the tool.
        detail: String,
    },
    /// The agent-level permission mode decided.
    Mode {
        /// Which mode produced this decision.
        mode: String,
    },
    /// Dangerous tool auto-escalated to Ask.
    DangerousAutoAsk {
        /// The tool that triggered auto-escalation.
        tool_name: String,
    },
    /// Safety check (bypass-immune).
    SafetyCheck {
        /// Why this is a safety concern.
        reason: String,
    },
    /// Global default policy applied.
    GlobalDefault,
}

// ---------------------------------------------------------------------------
// Permission result (returned by tool-level checks and the full pipeline)
// ---------------------------------------------------------------------------

/// Result of a permission evaluation.
#[derive(Debug, Clone)]
pub struct PermissionResult {
    /// The resolved behavior.
    pub behavior: PermissionBehavior,
    /// Why this behavior was chosen.
    pub reason: PermissionReason,
    /// Human-readable message for UI display (e.g., "`ShellExec` wants to run...").
    pub message: Option<String>,
    /// Optional updated input (tool may rewrite arguments during permission check).
    pub updated_input: Option<serde_json::Value>,
}

impl PermissionResult {
    /// Create a `Passthrough` result (tool has no opinion).
    pub fn passthrough() -> Self {
        Self {
            behavior: PermissionBehavior::Passthrough,
            reason: PermissionReason::ToolCheck {
                detail: "tool defers to general permission system".into(),
            },
            message: None,
            updated_input: None,
        }
    }

    /// Create an `Allow` result from a tool check.
    pub fn allow(detail: impl Into<String>) -> Self {
        Self {
            behavior: PermissionBehavior::Allow,
            reason: PermissionReason::ToolCheck {
                detail: detail.into(),
            },
            message: None,
            updated_input: None,
        }
    }

    /// Create a `Deny` result from a tool check.
    pub fn deny(detail: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            behavior: PermissionBehavior::Deny,
            reason: PermissionReason::ToolCheck {
                detail: detail.into(),
            },
            message: Some(message.into()),
            updated_input: None,
        }
    }

    /// Create an `Ask` result from a tool check.
    pub fn ask(detail: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            behavior: PermissionBehavior::Ask,
            reason: PermissionReason::ToolCheck {
                detail: detail.into(),
            },
            message: Some(message.into()),
            updated_input: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Permission context
// ---------------------------------------------------------------------------

/// Immutable context passed to permission checks.
///
/// Assembled by the permission pipeline from all rule sources and the
/// current mode. Individual tools receive this to make content-aware
/// permission decisions.
#[derive(Debug, Clone, Default)]
pub struct PermissionContext {
    /// Current permission mode.
    pub mode: PermissionMode,
    /// Active rules (merged from all sources, sorted by precedence).
    pub rules: Vec<PermissionRule>,
    /// Additional allowed working directories.
    pub additional_directories: Vec<String>,
}

// ---------------------------------------------------------------------------
// Permission update (for HITL ApproveAlways / DenyAlways)
// ---------------------------------------------------------------------------

/// An update operation for permission rules.
///
/// Used when the user responds to an HITL prompt with "Always Allow"
/// or "Always Deny" to persist a rule for future calls.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionUpdate {
    /// Where to persist (global or project).
    pub destination: PermissionRuleSource,
    /// The rule target.
    pub target: PermissionRuleTarget,
    /// The behavior to set.
    pub behavior: PermissionBehavior,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- PermissionRuleTarget --

    #[test]
    fn test_target_tool_matches_tool_name() {
        let target = PermissionRuleTarget::tool("ShellExec");
        assert!(target.matches_tool("ShellExec"));
        assert!(!target.matches_tool("FileRead"));
    }

    #[test]
    fn test_target_with_pattern_does_not_match_tool_only() {
        let target = PermissionRuleTarget::with_pattern("ShellExec", "npm:*");
        assert!(!target.matches_tool("ShellExec"));
    }

    #[test]
    fn test_target_tool_level_matches_any_content() {
        let target = PermissionRuleTarget::tool("ShellExec");
        assert!(target.matches("ShellExec", None));
        assert!(target.matches("ShellExec", Some("npm install")));
        assert!(target.matches("ShellExec", Some("rm -rf /")));
    }

    #[test]
    fn test_target_content_pattern_exact() {
        let target = PermissionRuleTarget::with_pattern("ShellExec", "npm install");
        assert!(target.matches("ShellExec", Some("npm install")));
        assert!(!target.matches("ShellExec", Some("npm install --save foo")));
        assert!(!target.matches("ShellExec", None));
    }

    #[test]
    fn test_target_content_pattern_prefix_wildcard() {
        let target = PermissionRuleTarget::with_pattern("ShellExec", "npm install:*");
        assert!(target.matches("ShellExec", Some("npm install")));
        assert!(target.matches("ShellExec", Some("npm install --save foo")));
        assert!(!target.matches("ShellExec", Some("npm publish")));
    }

    #[test]
    fn test_target_content_pattern_star() {
        let target = PermissionRuleTarget::with_pattern("ShellExec", "*");
        assert!(target.matches("ShellExec", Some("anything")));
    }

    #[test]
    fn test_target_wrong_tool_never_matches() {
        let target = PermissionRuleTarget::tool("FileRead");
        assert!(!target.matches("ShellExec", None));
    }

    #[test]
    fn test_target_to_rule_string() {
        let t1 = PermissionRuleTarget::tool("ShellExec");
        assert_eq!(t1.to_rule_string(), "ShellExec");

        let t2 = PermissionRuleTarget::with_pattern("ShellExec", "npm install:*");
        assert_eq!(t2.to_rule_string(), "ShellExec(npm install:*)");
    }

    // -- PermissionRuleSource --

    #[test]
    fn test_source_precedence_order() {
        assert!(
            PermissionRuleSource::Session.precedence() < PermissionRuleSource::CliArg.precedence()
        );
        assert!(
            PermissionRuleSource::CliArg.precedence()
                < PermissionRuleSource::ProjectSettings.precedence()
        );
        assert!(
            PermissionRuleSource::ProjectSettings.precedence()
                < PermissionRuleSource::GlobalSettings.precedence()
        );
        assert!(
            PermissionRuleSource::GlobalSettings.precedence()
                < PermissionRuleSource::AgentConfig.precedence()
        );
    }

    #[test]
    fn test_source_persistable() {
        assert!(PermissionRuleSource::GlobalSettings.is_persistable());
        assert!(PermissionRuleSource::ProjectSettings.is_persistable());
        assert!(!PermissionRuleSource::CliArg.is_persistable());
        assert!(!PermissionRuleSource::Session.is_persistable());
        assert!(!PermissionRuleSource::AgentConfig.is_persistable());
    }

    // -- PermissionResult constructors --

    #[test]
    fn test_result_passthrough() {
        let r = PermissionResult::passthrough();
        assert_eq!(r.behavior, PermissionBehavior::Passthrough);
    }

    #[test]
    fn test_result_allow() {
        let r = PermissionResult::allow("read-only tool");
        assert_eq!(r.behavior, PermissionBehavior::Allow);
    }

    #[test]
    fn test_result_deny() {
        let r = PermissionResult::deny("blocked", "not allowed");
        assert_eq!(r.behavior, PermissionBehavior::Deny);
        assert_eq!(r.message.unwrap(), "not allowed");
    }

    #[test]
    fn test_result_ask() {
        let r = PermissionResult::ask("needs approval", "confirm?");
        assert_eq!(r.behavior, PermissionBehavior::Ask);
        assert_eq!(r.message.unwrap(), "confirm?");
    }

    // -- PermissionMode --

    #[test]
    fn test_mode_default() {
        let mode = PermissionMode::default();
        assert_eq!(mode, PermissionMode::Default);
    }

    #[test]
    fn test_mode_display() {
        assert_eq!(PermissionMode::Default.to_string(), "default");
        assert_eq!(PermissionMode::Plan.to_string(), "plan");
        assert_eq!(PermissionMode::AcceptEdits.to_string(), "accept_edits");
        assert_eq!(
            PermissionMode::BypassPermissions.to_string(),
            "bypass_permissions"
        );
        assert_eq!(PermissionMode::DontAsk.to_string(), "dont_ask");
    }

    // -- match_content_pattern --

    #[test]
    fn test_match_content_pattern_star() {
        assert!(match_content_pattern("*", "anything"));
    }

    #[test]
    fn test_match_content_pattern_prefix() {
        assert!(match_content_pattern("npm:*", "npm install foo"));
        assert!(match_content_pattern("npm:*", "npm"));
        assert!(!match_content_pattern("npm:*", "yarn"));
    }

    #[test]
    fn test_match_content_pattern_exact() {
        assert!(match_content_pattern("git status", "git status"));
        assert!(!match_content_pattern("git status", "git status --short"));
    }
}
