//! Permission rule store: load, merge, and persist permission rules.
//!
//! Rules are loaded from layered TOML config files:
//! - Global: `~/.y-agent/y-agent.toml` `[permissions]` section
//! - Project: `.y-agent/y-agent.toml` `[permissions]` section
//! - CLI args: `--allow-tool`, `--deny-tool` flags
//! - Session: in-memory rules (not persisted)
//!
//! Merge priority: session > cli > project > global.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use y_core::permission_types::{
    PermissionBehavior, PermissionContext, PermissionMode, PermissionRule, PermissionRuleSource,
    PermissionRuleTarget, PermissionUpdate,
};

use crate::error::GuardrailError;

// ---------------------------------------------------------------------------
// TOML schema
// ---------------------------------------------------------------------------

/// The `[permissions]` section of `y-agent.toml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PermissionSettings {
    /// Default permission mode for the agent.
    pub default_mode: Option<String>,

    /// Rules that allow specific tools or tool+content patterns.
    ///
    /// Format: `"ToolName"` or `"ToolName(content_pattern)"`.
    #[serde(default)]
    pub allow: Vec<String>,

    /// Rules that deny specific tools or tool+content patterns.
    #[serde(default)]
    pub deny: Vec<String>,

    /// Rules that require asking for specific tools or tool+content patterns.
    #[serde(default)]
    pub ask: Vec<String>,

    /// Additional directories the agent is allowed to access.
    #[serde(default)]
    pub additional_directories: Vec<String>,
}

/// Wrapper for the top-level TOML file containing an optional `[permissions]` section.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct TomlConfig {
    #[serde(default)]
    permissions: PermissionSettings,
}

// ---------------------------------------------------------------------------
// Rule parsing
// ---------------------------------------------------------------------------

/// Parse a rule string like `"ShellExec"` or `"ShellExec(npm install:*)"`.
fn parse_rule_string(s: &str) -> PermissionRuleTarget {
    if let Some(open_paren) = s.find('(') {
        if let Some(close_paren) = s.rfind(')') {
            let tool_name = s[..open_paren].trim().to_string();
            let pattern = s[open_paren + 1..close_paren].trim().to_string();
            return PermissionRuleTarget::with_pattern(tool_name, pattern);
        }
    }
    PermissionRuleTarget::tool(s.trim())
}

/// Convert a list of rule strings + a behavior + a source into `PermissionRule`s.
fn parse_rules(
    strings: &[String],
    behavior: PermissionBehavior,
    source: &PermissionRuleSource,
) -> Vec<PermissionRule> {
    strings
        .iter()
        .map(|s| PermissionRule::new(source.clone(), behavior, parse_rule_string(s)))
        .collect()
}

/// Convert `PermissionSettings` into rules for a given source.
fn settings_to_rules(
    settings: &PermissionSettings,
    source: &PermissionRuleSource,
) -> Vec<PermissionRule> {
    let mut rules = Vec::new();
    rules.extend(parse_rules(
        &settings.allow,
        PermissionBehavior::Allow,
        source,
    ));
    rules.extend(parse_rules(
        &settings.deny,
        PermissionBehavior::Deny,
        source,
    ));
    rules.extend(parse_rules(&settings.ask, PermissionBehavior::Ask, source));
    rules
}

/// Format a permission rule back into its TOML rule string form.
fn rule_to_string(target: &PermissionRuleTarget) -> String {
    target.to_rule_string()
}

// ---------------------------------------------------------------------------
// PermissionRuleStore
// ---------------------------------------------------------------------------

/// Manages loading, merging, and persisting permission rules from multiple sources.
///
/// The store loads rules from global and project TOML files, CLI arguments,
/// and in-memory session rules. It merges them by precedence and can persist
/// changes back to the appropriate TOML file.
#[derive(Debug, Clone)]
pub struct PermissionRuleStore {
    /// Rules from `~/.y-agent/y-agent.toml`.
    global_rules: Vec<PermissionRule>,
    /// Rules from `.y-agent/y-agent.toml`.
    project_rules: Vec<PermissionRule>,
    /// Rules from CLI arguments.
    cli_rules: Vec<PermissionRule>,
    /// Session-scoped rules (in-memory only).
    session_rules: Vec<PermissionRule>,

    /// Global settings (for persistence).
    global_settings: PermissionSettings,
    /// Project settings (for persistence).
    project_settings: PermissionSettings,

    /// Path to the global settings file.
    global_path: Option<PathBuf>,
    /// Path to the project settings file.
    project_path: Option<PathBuf>,

    /// Additional directories from all sources.
    additional_directories: Vec<String>,

    /// Default permission mode (from most specific source).
    default_mode: PermissionMode,
}

impl Default for PermissionRuleStore {
    fn default() -> Self {
        Self {
            global_rules: Vec::new(),
            project_rules: Vec::new(),
            cli_rules: Vec::new(),
            session_rules: Vec::new(),
            global_settings: PermissionSettings::default(),
            project_settings: PermissionSettings::default(),
            global_path: None,
            project_path: None,
            additional_directories: Vec::new(),
            default_mode: PermissionMode::Default,
        }
    }
}

impl PermissionRuleStore {
    /// Create an empty rule store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Load rules from global and optional project config files.
    ///
    /// Files that don't exist or fail to parse are ignored with a warning.
    pub fn load(global_path: &Path, project_path: Option<&Path>) -> Self {
        let mut store = Self::new();

        // Load global settings.
        store.global_path = Some(global_path.to_path_buf());
        if global_path.exists() {
            match load_settings_from_file(global_path) {
                Ok(settings) => {
                    store.global_rules =
                        settings_to_rules(&settings, &PermissionRuleSource::GlobalSettings);
                    store
                        .additional_directories
                        .extend(settings.additional_directories.clone());

                    if let Some(ref mode_str) = settings.default_mode {
                        if let Some(mode) = parse_mode(mode_str) {
                            store.default_mode = mode;
                        }
                    }
                    store.global_settings = settings;
                    debug!(
                        path = %global_path.display(),
                        rules = store.global_rules.len(),
                        "loaded global permission rules"
                    );
                }
                Err(e) => {
                    warn!(
                        path = %global_path.display(),
                        error = %e,
                        "failed to load global permission settings"
                    );
                }
            }
        }

        // Load project settings.
        if let Some(pp) = project_path {
            store.project_path = Some(pp.to_path_buf());
            if pp.exists() {
                match load_settings_from_file(pp) {
                    Ok(settings) => {
                        store.project_rules =
                            settings_to_rules(&settings, &PermissionRuleSource::ProjectSettings);
                        store
                            .additional_directories
                            .extend(settings.additional_directories.clone());

                        // Project mode overrides global mode.
                        if let Some(ref mode_str) = settings.default_mode {
                            if let Some(mode) = parse_mode(mode_str) {
                                store.default_mode = mode;
                            }
                        }
                        store.project_settings = settings;
                        debug!(
                            path = %pp.display(),
                            rules = store.project_rules.len(),
                            "loaded project permission rules"
                        );
                    }
                    Err(e) => {
                        warn!(
                            path = %pp.display(),
                            error = %e,
                            "failed to load project permission settings"
                        );
                    }
                }
            }
        }

        store
    }

    /// Add CLI argument rules.
    pub fn add_cli_rules(&mut self, rules: Vec<PermissionRule>) {
        self.cli_rules.extend(rules);
    }

    /// Add a CLI allow rule for a tool.
    pub fn add_cli_allow(&mut self, tool_spec: &str) {
        let target = parse_rule_string(tool_spec);
        self.cli_rules.push(PermissionRule::new(
            PermissionRuleSource::CliArg,
            PermissionBehavior::Allow,
            target,
        ));
    }

    /// Add a CLI deny rule for a tool.
    pub fn add_cli_deny(&mut self, tool_spec: &str) {
        let target = parse_rule_string(tool_spec);
        self.cli_rules.push(PermissionRule::new(
            PermissionRuleSource::CliArg,
            PermissionBehavior::Deny,
            target,
        ));
    }

    /// Add a session rule (in-memory only, lost on restart).
    pub fn add_session_rule(&mut self, rule: PermissionRule) {
        self.session_rules.push(rule);
    }

    /// Process a permission update (e.g., from HITL "Always Allow" / "Always Deny").
    pub fn apply_update(&mut self, update: PermissionUpdate) -> Result<(), GuardrailError> {
        let rule = PermissionRule::new(update.destination.clone(), update.behavior, update.target);

        match update.destination {
            PermissionRuleSource::GlobalSettings => {
                self.add_rule_to_settings(&rule, &mut self.global_settings.clone(), true)?;
            }
            PermissionRuleSource::ProjectSettings => {
                self.add_rule_to_settings(&rule, &mut self.project_settings.clone(), false)?;
            }
            PermissionRuleSource::Session => {
                self.session_rules.push(rule);
            }
            _ => {
                return Err(GuardrailError::Other {
                    message: format!("cannot persist rules to {:?} source", update.destination),
                });
            }
        }

        Ok(())
    }

    /// Add a rule and persist it to the appropriate TOML file.
    pub fn add_rule(&mut self, rule: PermissionRule) -> Result<(), GuardrailError> {
        match rule.source {
            PermissionRuleSource::GlobalSettings => {
                self.global_rules.push(rule.clone());
                add_to_settings(&mut self.global_settings, &rule);
                if let Some(ref path) = self.global_path {
                    save_settings_to_file(path, &self.global_settings)?;
                }
            }
            PermissionRuleSource::ProjectSettings => {
                self.project_rules.push(rule.clone());
                add_to_settings(&mut self.project_settings, &rule);
                if let Some(ref path) = self.project_path {
                    save_settings_to_file(path, &self.project_settings)?;
                }
            }
            PermissionRuleSource::CliArg => {
                self.cli_rules.push(rule);
            }
            PermissionRuleSource::Session => {
                self.session_rules.push(rule);
            }
            PermissionRuleSource::AgentConfig => {
                return Err(GuardrailError::Other {
                    message: "cannot add rules to AgentConfig source".into(),
                });
            }
        }
        Ok(())
    }

    /// Remove a rule from its source and persist if applicable.
    pub fn remove_rule(&mut self, rule: &PermissionRule) -> Result<(), GuardrailError> {
        match rule.source {
            PermissionRuleSource::GlobalSettings => {
                self.global_rules
                    .retain(|r| r.target != rule.target || r.behavior != rule.behavior);
                remove_from_settings(&mut self.global_settings, rule);
                if let Some(ref path) = self.global_path {
                    save_settings_to_file(path, &self.global_settings)?;
                }
            }
            PermissionRuleSource::ProjectSettings => {
                self.project_rules
                    .retain(|r| r.target != rule.target || r.behavior != rule.behavior);
                remove_from_settings(&mut self.project_settings, rule);
                if let Some(ref path) = self.project_path {
                    save_settings_to_file(path, &self.project_settings)?;
                }
            }
            PermissionRuleSource::CliArg => {
                self.cli_rules
                    .retain(|r| r.target != rule.target || r.behavior != rule.behavior);
            }
            PermissionRuleSource::Session => {
                self.session_rules
                    .retain(|r| r.target != rule.target || r.behavior != rule.behavior);
            }
            PermissionRuleSource::AgentConfig => {}
        }
        Ok(())
    }

    /// Merge all rules, sorted by source precedence (highest priority first).
    pub fn merged_rules(&self) -> Vec<PermissionRule> {
        let mut all = Vec::new();
        all.extend(self.session_rules.clone());
        all.extend(self.cli_rules.clone());
        all.extend(self.project_rules.clone());
        all.extend(self.global_rules.clone());
        // Already in precedence order since we extend session first.
        all
    }

    /// Build a `PermissionContext` from the current state.
    pub fn build_context(&self, mode_override: Option<PermissionMode>) -> PermissionContext {
        PermissionContext {
            mode: mode_override.unwrap_or(self.default_mode),
            rules: self.merged_rules(),
            additional_directories: self.additional_directories.clone(),
        }
    }

    /// Get the configured default mode.
    pub fn default_mode(&self) -> PermissionMode {
        self.default_mode
    }

    /// Set the default mode.
    pub fn set_default_mode(&mut self, mode: PermissionMode) {
        self.default_mode = mode;
    }

    /// Get additional directories.
    pub fn additional_directories(&self) -> &[String] {
        &self.additional_directories
    }

    /// Get all rules for a specific tool name (as owned copies).
    pub fn rules_for_tool(&self, tool_name: &str) -> Vec<PermissionRule> {
        self.merged_rules()
            .into_iter()
            .filter(|r| r.target.tool_name == tool_name)
            .collect()
    }

    /// Internal helper for `apply_update` with settings mutation.
    fn add_rule_to_settings(
        &mut self,
        rule: &PermissionRule,
        settings: &mut PermissionSettings,
        is_global: bool,
    ) -> Result<(), GuardrailError> {
        add_to_settings(settings, rule);
        if is_global {
            self.global_rules.push(rule.clone());
            self.global_settings = settings.clone();
            if let Some(ref path) = self.global_path {
                save_settings_to_file(path, settings)?;
            }
        } else {
            self.project_rules.push(rule.clone());
            self.project_settings = settings.clone();
            if let Some(ref path) = self.project_path {
                save_settings_to_file(path, settings)?;
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Settings mutation helpers
// ---------------------------------------------------------------------------

fn add_to_settings(settings: &mut PermissionSettings, rule: &PermissionRule) {
    let s = rule_to_string(&rule.target);
    match rule.behavior {
        PermissionBehavior::Allow => {
            if !settings.allow.contains(&s) {
                settings.allow.push(s);
            }
        }
        PermissionBehavior::Deny => {
            if !settings.deny.contains(&s) {
                settings.deny.push(s);
            }
        }
        PermissionBehavior::Ask => {
            if !settings.ask.contains(&s) {
                settings.ask.push(s);
            }
        }
        PermissionBehavior::Passthrough => {
            // Passthrough is not persisted.
        }
    }
}

fn remove_from_settings(settings: &mut PermissionSettings, rule: &PermissionRule) {
    let s = rule_to_string(&rule.target);
    match rule.behavior {
        PermissionBehavior::Allow => settings.allow.retain(|x| x != &s),
        PermissionBehavior::Deny => settings.deny.retain(|x| x != &s),
        PermissionBehavior::Ask => settings.ask.retain(|x| x != &s),
        PermissionBehavior::Passthrough => {}
    }
}

// ---------------------------------------------------------------------------
// TOML I/O
// ---------------------------------------------------------------------------

/// Load `PermissionSettings` from a TOML file.
fn load_settings_from_file(path: &Path) -> Result<PermissionSettings, GuardrailError> {
    let content = std::fs::read_to_string(path).map_err(|e| GuardrailError::Other {
        message: format!("failed to read '{}': {}", path.display(), e),
    })?;

    let config: TomlConfig = toml::from_str(&content).map_err(|e| GuardrailError::Other {
        message: format!("failed to parse '{}': {}", path.display(), e),
    })?;

    Ok(config.permissions)
}

/// Save `PermissionSettings` to a TOML file.
///
/// This reads the existing TOML file, updates only the `[permissions]` section,
/// and writes it back. Other sections are preserved.
fn save_settings_to_file(path: &Path, settings: &PermissionSettings) -> Result<(), GuardrailError> {
    // Read existing TOML or start with empty.
    let existing = if path.exists() {
        std::fs::read_to_string(path).unwrap_or_default()
    } else {
        String::new()
    };

    let mut config: toml::Value =
        toml::from_str(&existing).unwrap_or(toml::Value::Table(toml::map::Map::new()));

    // Serialize the permissions section.
    let perm_value = toml::Value::try_from(settings).map_err(|e| GuardrailError::Other {
        message: format!("failed to serialize permissions: {e}"),
    })?;

    if let toml::Value::Table(ref mut table) = config {
        table.insert("permissions".to_string(), perm_value);
    }

    let output = toml::to_string_pretty(&config).map_err(|e| GuardrailError::Other {
        message: format!("failed to format TOML: {e}"),
    })?;

    // Ensure parent dir exists.
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| GuardrailError::Other {
            message: format!("failed to create directory '{}': {}", parent.display(), e),
        })?;
    }

    std::fs::write(path, output).map_err(|e| GuardrailError::Other {
        message: format!("failed to write '{}': {}", path.display(), e),
    })?;

    debug!(path = %path.display(), "persisted permission settings");
    Ok(())
}

/// Parse a mode string from TOML config.
fn parse_mode(s: &str) -> Option<PermissionMode> {
    match s {
        "default" => Some(PermissionMode::Default),
        "plan" => Some(PermissionMode::Plan),
        "accept_edits" => Some(PermissionMode::AcceptEdits),
        "bypass_permissions" => Some(PermissionMode::BypassPermissions),
        "dont_ask" => Some(PermissionMode::DontAsk),
        _ => {
            warn!(mode = s, "unknown permission mode in config");
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // -- parse_rule_string --

    #[test]
    fn test_parse_tool_only() {
        let target = parse_rule_string("ShellExec");
        assert_eq!(target.tool_name, "ShellExec");
        assert!(target.content_pattern.is_none());
    }

    #[test]
    fn test_parse_tool_with_pattern() {
        let target = parse_rule_string("ShellExec(npm install:*)");
        assert_eq!(target.tool_name, "ShellExec");
        assert_eq!(target.content_pattern.unwrap(), "npm install:*");
    }

    #[test]
    fn test_parse_tool_with_exact_content() {
        let target = parse_rule_string("ShellExec(git status)");
        assert_eq!(target.tool_name, "ShellExec");
        assert_eq!(target.content_pattern.unwrap(), "git status");
    }

    // -- TOML round-trip --

    #[test]
    fn test_toml_roundtrip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("y-agent.toml");

        let mut settings = PermissionSettings::default();
        settings.allow.push("FileRead".to_string());
        settings.allow.push("Glob".to_string());
        settings.deny.push("ShellExec(rm -rf:*)".to_string());
        settings.ask.push("FileWrite".to_string());
        settings.default_mode = Some("default".to_string());

        save_settings_to_file(&path, &settings).unwrap();
        let loaded = load_settings_from_file(&path).unwrap();

        assert_eq!(loaded.allow, settings.allow);
        assert_eq!(loaded.deny, settings.deny);
        assert_eq!(loaded.ask, settings.ask);
        assert_eq!(loaded.default_mode, settings.default_mode);
    }

    // -- PermissionRuleStore --

    #[test]
    fn test_store_load_and_merge() {
        let dir = TempDir::new().unwrap();
        let global_path = dir.path().join("global.toml");
        let project_path = dir.path().join("project.toml");

        // Write global settings.
        let global_toml = r#"
[permissions]
allow = ["FileRead", "Glob"]
deny = ["ShellExec(rm -rf:*)"]
"#;
        std::fs::write(&global_path, global_toml).unwrap();

        // Write project settings.
        let project_toml = r#"
[permissions]
allow = ["ShellExec(npm install:*)"]
ask = ["FileWrite"]
"#;
        std::fs::write(&project_path, project_toml).unwrap();

        let store = PermissionRuleStore::load(&global_path, Some(&project_path));

        let merged = store.merged_rules();
        // project allow (1) + project ask (1) + global allow (2) + global deny (1) = 5
        assert_eq!(merged.len(), 5);

        // Check precedence: project rules come before global rules.
        let sources: Vec<_> = merged.iter().map(|r| &r.source).collect();
        assert_eq!(sources[0], &PermissionRuleSource::ProjectSettings);
        assert_eq!(sources[1], &PermissionRuleSource::ProjectSettings);
        assert_eq!(sources[2], &PermissionRuleSource::GlobalSettings);
    }

    #[test]
    fn test_store_add_session_rule() {
        let mut store = PermissionRuleStore::new();
        store.add_session_rule(PermissionRule::new(
            PermissionRuleSource::Session,
            PermissionBehavior::Allow,
            PermissionRuleTarget::tool("ShellExec"),
        ));

        let merged = store.merged_rules();
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].source, PermissionRuleSource::Session);
    }

    #[test]
    fn test_store_cli_rules() {
        let mut store = PermissionRuleStore::new();
        store.add_cli_allow("FileRead");
        store.add_cli_deny("ShellExec");

        let merged = store.merged_rules();
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].behavior, PermissionBehavior::Allow);
        assert_eq!(merged[1].behavior, PermissionBehavior::Deny);
    }

    #[test]
    fn test_store_persist_rule() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("y-agent.toml");
        std::fs::write(&path, "[permissions]\n").unwrap();

        let mut store = PermissionRuleStore::load(&path, None);
        store
            .add_rule(PermissionRule::new(
                PermissionRuleSource::GlobalSettings,
                PermissionBehavior::Allow,
                PermissionRuleTarget::tool("MyTool"),
            ))
            .unwrap();

        // Verify it was persisted.
        let reloaded = PermissionRuleStore::load(&path, None);
        let rules = reloaded.merged_rules();
        assert!(rules.iter().any(|r| r.target.tool_name == "MyTool"));
    }

    // T-PERM-008: Persisted rule survives restart
    #[test]
    fn test_persisted_rule_survives_restart() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("y-agent.toml");

        // Session 1: add a rule.
        {
            std::fs::write(&path, "").unwrap();
            let mut store = PermissionRuleStore::load(&path, None);
            store
                .add_rule(PermissionRule::new(
                    PermissionRuleSource::GlobalSettings,
                    PermissionBehavior::Deny,
                    PermissionRuleTarget::with_pattern("ShellExec", "rm -rf:*"),
                ))
                .unwrap();
        }

        // Session 2: rule should be there.
        {
            let store = PermissionRuleStore::load(&path, None);
            let rules = store.merged_rules();
            let found = rules
                .iter()
                .any(|r| r.target.matches("ShellExec", Some("rm -rf /")));
            assert!(found, "persisted rule should survive restart");
        }
    }

    #[test]
    fn test_remove_rule() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("y-agent.toml");
        std::fs::write(&path, "[permissions]\nallow = [\"FileRead\"]\n").unwrap();

        let mut store = PermissionRuleStore::load(&path, None);
        assert_eq!(store.merged_rules().len(), 1);

        store
            .remove_rule(&PermissionRule::new(
                PermissionRuleSource::GlobalSettings,
                PermissionBehavior::Allow,
                PermissionRuleTarget::tool("FileRead"),
            ))
            .unwrap();

        assert_eq!(store.merged_rules().len(), 0);
    }

    #[test]
    fn test_build_context() {
        let mut store = PermissionRuleStore::new();
        store.add_cli_allow("FileRead");
        store.set_default_mode(PermissionMode::Plan);

        let ctx = store.build_context(None);
        assert_eq!(ctx.mode, PermissionMode::Plan);
        assert_eq!(ctx.rules.len(), 1);

        // Override mode.
        let ctx2 = store.build_context(Some(PermissionMode::BypassPermissions));
        assert_eq!(ctx2.mode, PermissionMode::BypassPermissions);
    }

    #[test]
    fn test_parse_mode_strings() {
        assert_eq!(parse_mode("default"), Some(PermissionMode::Default));
        assert_eq!(parse_mode("plan"), Some(PermissionMode::Plan));
        assert_eq!(
            parse_mode("accept_edits"),
            Some(PermissionMode::AcceptEdits)
        );
        assert_eq!(
            parse_mode("bypass_permissions"),
            Some(PermissionMode::BypassPermissions)
        );
        assert_eq!(parse_mode("dont_ask"), Some(PermissionMode::DontAsk));
        assert_eq!(parse_mode("invalid"), None);
    }

    #[test]
    fn test_nonexistent_file_no_error() {
        let store = PermissionRuleStore::load(Path::new("/nonexistent/path.toml"), None);
        assert!(store.merged_rules().is_empty());
    }
}
