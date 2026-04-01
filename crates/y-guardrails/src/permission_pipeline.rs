//! Full permission evaluation pipeline.
//!
//! Implements the multi-stage pipeline that mirrors Claude Code's architecture:
//!
//! ```text
//! 1. Deny rules (tool-level and content-specific)
//! 2. Ask rules (tool-level and content-specific)
//! 3. Tool.check_permissions() (content-specific tool logic)
//! 4. Mode-based overrides (bypass, plan, accept_edits, etc.)
//! 5. Allow rules (tool-level and content-specific)
//! 6. Passthrough-to-Ask conversion
//! 7. Mode transforms (dont_ask: ask->deny)
//! ```
//!
//! Design reference: `docs/design/guardrails-hitl-design.md`

use y_core::permission_types::{
    PermissionBehavior, PermissionContext, PermissionMode, PermissionReason, PermissionResult,
    PermissionRule,
};

// ---------------------------------------------------------------------------
// Pipeline
// ---------------------------------------------------------------------------

/// Evaluate the full permission pipeline for a tool invocation.
///
/// Arguments:
/// - `tool_name`: name of the tool being called
/// - `input_content`: optional content string for content-specific rule matching
///   (e.g., the shell command for `ShellExec`, the file path for `FileWrite`)
/// - `is_dangerous`: whether the tool's `ToolDefinition.is_dangerous` is set
/// - `tool_result`: result from `Tool::check_permissions()` (the tool's own opinion)
/// - `perm_ctx`: merged permission context (mode + rules + directories)
///
/// Returns the final `PermissionResult` after all pipeline stages.
pub fn evaluate_pipeline(
    tool_name: &str,
    input_content: Option<&str>,
    is_dangerous: bool,
    tool_result: &PermissionResult,
    perm_ctx: &PermissionContext,
) -> PermissionResult {
    // Stage 1: Deny rules -- highest priority, cannot be overridden.
    if let Some(rule) = find_matching_rule(
        &perm_ctx.rules,
        tool_name,
        input_content,
        PermissionBehavior::Deny,
    ) {
        return PermissionResult {
            behavior: PermissionBehavior::Deny,
            reason: PermissionReason::Rule {
                rule_display: rule.to_string(),
            },
            message: Some(format!("denied by rule: {rule}")),
            updated_input: None,
        };
    }

    // Stage 2: Ask rules.
    let ask_rule = find_matching_rule(
        &perm_ctx.rules,
        tool_name,
        input_content,
        PermissionBehavior::Ask,
    );

    // Stage 3: Tool's own check_permissions() result.
    // If the tool said Deny, respect it (bypass-immune).
    if tool_result.behavior == PermissionBehavior::Deny {
        return tool_result.clone();
    }

    // If the tool said Ask, merge with ask rules.
    let tool_wants_ask = tool_result.behavior == PermissionBehavior::Ask;

    // Stage 4: Mode-based overrides.
    match perm_ctx.mode {
        PermissionMode::BypassPermissions => {
            // Bypass overrides everything except Deny (already handled above).
            // If tool explicitly asked, we still bypass.
            if tool_result.behavior == PermissionBehavior::Allow {
                return tool_result.clone();
            }
            return PermissionResult {
                behavior: PermissionBehavior::Allow,
                reason: PermissionReason::Mode {
                    mode: "bypass_permissions".into(),
                },
                message: None,
                updated_input: tool_result.updated_input.clone(),
            };
        }
        PermissionMode::Plan => {
            // Plan mode: only read-only tools are allowed without asking.
            // The tool_result already reflects is_read_only() via check_permissions default.
            if tool_result.behavior == PermissionBehavior::Allow {
                return tool_result.clone();
            }
            // Everything else must ask.
            return PermissionResult {
                behavior: PermissionBehavior::Ask,
                reason: PermissionReason::Mode {
                    mode: "plan".into(),
                },
                message: Some(format!("{tool_name} requires approval in plan mode")),
                updated_input: tool_result.updated_input.clone(),
            };
        }
        PermissionMode::AcceptEdits => {
            // Accept edits: file tools auto-allowed, shell still asks.
            if tool_result.behavior == PermissionBehavior::Allow {
                return tool_result.clone();
            }
            // Check if this is a file edit tool (by name convention).
            let is_file_tool = matches!(tool_name, "FileWrite" | "FileEdit");
            if is_file_tool {
                return PermissionResult {
                    behavior: PermissionBehavior::Allow,
                    reason: PermissionReason::Mode {
                        mode: "accept_edits".into(),
                    },
                    message: None,
                    updated_input: tool_result.updated_input.clone(),
                };
            }
            // Non-file tools with side effects still ask.
        }
        PermissionMode::Default | PermissionMode::DontAsk => {
            // Handled in later stages.
        }
    }

    // Stage 5: Allow rules -- check if an explicit allow rule matches.
    if let Some(rule) = find_matching_rule(
        &perm_ctx.rules,
        tool_name,
        input_content,
        PermissionBehavior::Allow,
    ) {
        // Allow rule found.
        // If an ask rule also exists, the more specific one wins.
        if let Some(ask) = &ask_rule {
            if rule_is_more_specific(rule, ask) {
                return PermissionResult {
                    behavior: PermissionBehavior::Allow,
                    reason: PermissionReason::Rule {
                        rule_display: rule.to_string(),
                    },
                    message: None,
                    updated_input: tool_result.updated_input.clone(),
                };
            }
            // Ask rule is more specific or equally specific -- ask wins.
        } else {
            return PermissionResult {
                behavior: PermissionBehavior::Allow,
                reason: PermissionReason::Rule {
                    rule_display: rule.to_string(),
                },
                message: None,
                updated_input: tool_result.updated_input.clone(),
            };
        }
    }

    // If the tool already said Allow (read-only tools), respect it
    // unless an Ask rule overrides.
    if tool_result.behavior == PermissionBehavior::Allow && ask_rule.is_none() {
        return tool_result.clone();
    }

    // Stage 6: Passthrough-to-Ask conversion.
    // If we get here, the tool said Passthrough or Ask (or Allow overridden by ask rule).
    let ask_reason = if tool_wants_ask {
        tool_result
            .message
            .clone()
            .unwrap_or_else(|| format!("{tool_name} requires approval"))
    } else if let Some(ask) = &ask_rule {
        format!("ask rule: {ask}")
    } else if is_dangerous {
        format!("{tool_name} is dangerous -- requires approval")
    } else {
        format!("{tool_name} requires approval")
    };

    // Stage 7: Mode transforms.
    if perm_ctx.mode == PermissionMode::DontAsk {
        // Auto-deny all asks in headless/background mode.
        PermissionResult {
            behavior: PermissionBehavior::Deny,
            reason: PermissionReason::Mode {
                mode: "dont_ask".into(),
            },
            message: Some(format!("{tool_name} denied -- dont_ask mode active")),
            updated_input: None,
        }
    } else {
        // Default/Plan: escalate to Ask.
        let reason = if let Some(ask) = ask_rule {
            PermissionReason::Rule {
                rule_display: ask.to_string(),
            }
        } else if is_dangerous {
            PermissionReason::DangerousAutoAsk {
                tool_name: tool_name.to_string(),
            }
        } else {
            PermissionReason::GlobalDefault
        };

        PermissionResult {
            behavior: PermissionBehavior::Ask,
            reason,
            message: Some(ask_reason),
            updated_input: tool_result.updated_input.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// Rule matching helpers
// ---------------------------------------------------------------------------

/// Find the first matching rule with the given behavior.
///
/// Rules are expected to be in precedence order (session > cli > project > global).
/// Content-specific rules are checked before tool-level rules.
fn find_matching_rule<'a>(
    rules: &'a [PermissionRule],
    tool_name: &str,
    content: Option<&str>,
    behavior: PermissionBehavior,
) -> Option<&'a PermissionRule> {
    // First pass: content-specific matches (more specific).
    if content.is_some() {
        for rule in rules {
            if rule.behavior == behavior
                && rule.target.content_pattern.is_some()
                && rule.target.matches(tool_name, content)
            {
                return Some(rule);
            }
        }
    }

    // Second pass: tool-level matches.
    rules.iter().find(|rule| {
        rule.behavior == behavior
            && rule.target.content_pattern.is_none()
            && rule.target.matches(tool_name, content)
    })
}

/// Determine if rule `a` is more specific than rule `b`.
///
/// A content-specific rule (`ToolName(pattern)`) is more specific than a
/// tool-level rule (`ToolName`). Among rules at the same specificity level,
/// higher precedence source wins.
fn rule_is_more_specific(a: &PermissionRule, b: &PermissionRule) -> bool {
    let a_specific = a.target.content_pattern.is_some();
    let b_specific = b.target.content_pattern.is_some();

    if a_specific && !b_specific {
        return true;
    }
    if !a_specific && b_specific {
        return false;
    }

    // Same specificity: higher precedence wins (lower number = higher priority).
    a.source.precedence() < b.source.precedence()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use y_core::permission_types::{PermissionRuleSource, PermissionRuleTarget};

    use super::*;

    fn ctx(mode: PermissionMode, rules: Vec<PermissionRule>) -> PermissionContext {
        PermissionContext {
            mode,
            rules,
            additional_directories: vec![],
        }
    }

    fn allow_rule(tool: &str) -> PermissionRule {
        PermissionRule::new(
            PermissionRuleSource::GlobalSettings,
            PermissionBehavior::Allow,
            PermissionRuleTarget::tool(tool),
        )
    }

    fn deny_rule(tool: &str) -> PermissionRule {
        PermissionRule::new(
            PermissionRuleSource::GlobalSettings,
            PermissionBehavior::Deny,
            PermissionRuleTarget::tool(tool),
        )
    }

    fn ask_rule(tool: &str) -> PermissionRule {
        PermissionRule::new(
            PermissionRuleSource::GlobalSettings,
            PermissionBehavior::Ask,
            PermissionRuleTarget::tool(tool),
        )
    }

    fn content_allow_rule(tool: &str, pattern: &str) -> PermissionRule {
        PermissionRule::new(
            PermissionRuleSource::ProjectSettings,
            PermissionBehavior::Allow,
            PermissionRuleTarget::with_pattern(tool, pattern),
        )
    }

    // T-PERM-001: Read-only tools default to Allow
    #[test]
    fn test_read_only_tool_defaults_to_allow() {
        let tool_result = PermissionResult::allow("read-only tool");
        let context = ctx(PermissionMode::Default, vec![]);
        let result = evaluate_pipeline("FileRead", None, false, &tool_result, &context);
        assert_eq!(result.behavior, PermissionBehavior::Allow);
    }

    // T-PERM-002: Write tools default to Passthrough (becomes Ask)
    #[test]
    fn test_write_tool_passthrough_becomes_ask() {
        let tool_result = PermissionResult::passthrough();
        let context = ctx(PermissionMode::Default, vec![]);
        let result = evaluate_pipeline("ShellExec", None, true, &tool_result, &context);
        assert_eq!(result.behavior, PermissionBehavior::Ask);
    }

    // T-PERM-003: Deny rule blocks regardless
    #[test]
    fn test_deny_rule_blocks() {
        let tool_result = PermissionResult::allow("read-only tool");
        let context = ctx(PermissionMode::Default, vec![deny_rule("FileRead")]);
        let result = evaluate_pipeline("FileRead", None, false, &tool_result, &context);
        assert_eq!(result.behavior, PermissionBehavior::Deny);
    }

    // T-PERM-004: Content-specific allow overrides tool-level ask
    #[test]
    fn test_content_allow_overrides_tool_ask() {
        let tool_result = PermissionResult::passthrough();
        let context = ctx(
            PermissionMode::Default,
            vec![
                ask_rule("ShellExec"),
                content_allow_rule("ShellExec", "npm install:*"),
            ],
        );
        let result = evaluate_pipeline(
            "ShellExec",
            Some("npm install --save-dev foo"),
            true,
            &tool_result,
            &context,
        );
        assert_eq!(result.behavior, PermissionBehavior::Allow);
    }

    // T-PERM-005: BypassPermissions overrides Ask (but not Deny)
    #[test]
    fn test_bypass_overrides_ask_not_deny() {
        let tool_result = PermissionResult::passthrough();

        // Bypass + no deny -> Allow.
        let context = ctx(PermissionMode::BypassPermissions, vec![]);
        let result = evaluate_pipeline("ShellExec", None, true, &tool_result, &context);
        assert_eq!(result.behavior, PermissionBehavior::Allow);

        // Bypass + deny -> Deny.
        let context2 = ctx(
            PermissionMode::BypassPermissions,
            vec![deny_rule("ShellExec")],
        );
        let result2 = evaluate_pipeline("ShellExec", None, true, &tool_result, &context2);
        assert_eq!(result2.behavior, PermissionBehavior::Deny);
    }

    // T-PERM-006: Passthrough becomes Ask when no allow rule
    #[test]
    fn test_passthrough_becomes_ask_no_allow() {
        let tool_result = PermissionResult::passthrough();
        let context = ctx(PermissionMode::Default, vec![]);
        let result = evaluate_pipeline("FileWrite", None, true, &tool_result, &context);
        assert_eq!(result.behavior, PermissionBehavior::Ask);
    }

    // T-PERM-007: Rule source priority (session > cli > project > global)
    #[test]
    fn test_rule_source_priority() {
        let tool_result = PermissionResult::passthrough();

        // Global allows, session denies -> deny wins.
        let context = ctx(
            PermissionMode::Default,
            vec![
                PermissionRule::new(
                    PermissionRuleSource::Session,
                    PermissionBehavior::Deny,
                    PermissionRuleTarget::tool("ShellExec"),
                ),
                PermissionRule::new(
                    PermissionRuleSource::GlobalSettings,
                    PermissionBehavior::Allow,
                    PermissionRuleTarget::tool("ShellExec"),
                ),
            ],
        );
        let result = evaluate_pipeline("ShellExec", None, false, &tool_result, &context);
        assert_eq!(result.behavior, PermissionBehavior::Deny);
    }

    // T-PERM-010: DontAsk mode converts Ask to Deny
    #[test]
    fn test_dont_ask_mode_converts_ask_to_deny() {
        let tool_result = PermissionResult::passthrough();
        let context = ctx(PermissionMode::DontAsk, vec![]);
        let result = evaluate_pipeline("FileWrite", None, true, &tool_result, &context);
        assert_eq!(result.behavior, PermissionBehavior::Deny);
    }

    // Plan mode: read-only tools allowed, write tools ask
    #[test]
    fn test_plan_mode() {
        let context = ctx(PermissionMode::Plan, vec![]);

        // Read-only tool -> Allow.
        let read_result = PermissionResult::allow("read-only tool");
        let result = evaluate_pipeline("FileRead", None, false, &read_result, &context);
        assert_eq!(result.behavior, PermissionBehavior::Allow);

        // Write tool -> Ask.
        let write_result = PermissionResult::passthrough();
        let result = evaluate_pipeline("FileWrite", None, true, &write_result, &context);
        assert_eq!(result.behavior, PermissionBehavior::Ask);
    }

    // AcceptEdits mode: file tools auto-allowed, shell asks
    #[test]
    fn test_accept_edits_mode() {
        let context = ctx(PermissionMode::AcceptEdits, vec![]);

        // FileWrite -> Allow.
        let write_result = PermissionResult::passthrough();
        let result = evaluate_pipeline("FileWrite", None, true, &write_result, &context);
        assert_eq!(result.behavior, PermissionBehavior::Allow);

        // ShellExec -> Ask.
        let shell_result = PermissionResult::passthrough();
        let result = evaluate_pipeline("ShellExec", None, true, &shell_result, &context);
        assert_eq!(result.behavior, PermissionBehavior::Ask);
    }

    // Tool deny is bypass-immune
    #[test]
    fn test_tool_deny_bypass_immune() {
        let tool_result = PermissionResult::deny("unsafe path", "path outside workspace");
        let context = ctx(PermissionMode::BypassPermissions, vec![]);
        let result = evaluate_pipeline(
            "FileWrite",
            Some("/etc/passwd"),
            true,
            &tool_result,
            &context,
        );
        assert_eq!(result.behavior, PermissionBehavior::Deny);
    }

    // Allow rule for tool not matched by content-specific deny
    #[test]
    fn test_allow_rule_unaffected_by_unmatched_deny() {
        let tool_result = PermissionResult::passthrough();
        let context = ctx(
            PermissionMode::Default,
            vec![
                allow_rule("ShellExec"),
                PermissionRule::new(
                    PermissionRuleSource::GlobalSettings,
                    PermissionBehavior::Deny,
                    PermissionRuleTarget::with_pattern("ShellExec", "rm -rf:*"),
                ),
            ],
        );
        // Running "git status" -- deny rule for "rm -rf:*" shouldn't match.
        let result = evaluate_pipeline(
            "ShellExec",
            Some("git status"),
            true,
            &tool_result,
            &context,
        );
        assert_eq!(result.behavior, PermissionBehavior::Allow);
    }
}
