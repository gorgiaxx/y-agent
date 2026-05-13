//! Hook system configuration.
//!
//! Defines configuration types for:
//! - Middleware chain tuning (timeout, capacity)
//! - External hook handler definitions (command, HTTP, prompt, agent)
//! - Handler validation and security settings

use std::collections::HashMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::HookError;

// ---------------------------------------------------------------------------
// Hook handler configuration types
// ---------------------------------------------------------------------------

/// Configuration for a handler group bound to a hook point.
/// Each group has an optional matcher and one or more handlers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookHandlerGroupConfig {
    /// Regex pattern to filter when handlers fire.
    /// Omit or `"*"` for all events at this hook point.
    /// For tool-related hooks: matches against `tool_name`.
    /// For session hooks: matches against session event subtype.
    #[serde(default = "default_matcher")]
    pub matcher: String,

    /// Per-handler timeout in milliseconds.
    /// Defaults: 5000 (command/HTTP), 30000 (prompt), 120000 (agent).
    #[serde(default)]
    pub timeout_ms: Option<u64>,

    /// List of handlers to execute when matched.
    pub handlers: Vec<HandlerConfig>,
}

fn default_matcher() -> String {
    "*".to_string()
}

/// Individual handler definition, tagged by type.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HandlerConfig {
    /// Execute a shell command with JSON stdin/stdout.
    /// Exit codes: 0=allow, 1=error(continue), 2=block.
    Command {
        /// Absolute path to script or shell command.
        command: String,
        /// If true, fire-and-forget (result not awaited).
        #[serde(default)]
        r#async: bool,
    },
    /// POST JSON to an HTTP endpoint.
    /// Response uses same JSON output format as command hooks.
    Http {
        /// URL to POST to.
        url: String,
        /// HTTP headers. Values support $`ENV_VAR` substitution.
        #[serde(default)]
        headers: HashMap<String, String>,
        /// If true, fire-and-forget (result not awaited).
        #[serde(default)]
        r#async: bool,
    },
    /// Send event context + prompt to an LLM for single-turn evaluation.
    /// Only on decision-capable hook points.
    Prompt {
        /// Prompt template. $ARGUMENTS replaced with hook input JSON.
        prompt: String,
        /// Model override. Defaults to fastest available.
        #[serde(default)]
        model: Option<String>,
    },
    /// Spawn a subagent with read-only tools for multi-turn verification.
    /// Only on decision-capable hook points. Max 50 turns.
    Agent {
        /// Task prompt. $ARGUMENTS replaced with hook input JSON.
        prompt: String,
        /// Model override. Defaults to fastest available.
        #[serde(default)]
        model: Option<String>,
    },
}

impl HandlerConfig {
    /// Get the handler type as a string for logging/metrics.
    pub fn handler_type(&self) -> &'static str {
        match self {
            Self::Command { .. } => "command",
            Self::Http { .. } => "http",
            Self::Prompt { .. } => "prompt",
            Self::Agent { .. } => "agent",
        }
    }

    /// Get the default timeout for this handler type.
    pub fn default_timeout_ms(&self) -> u64 {
        match self {
            Self::Command { .. } | Self::Http { .. } => 5000,
            Self::Prompt { .. } => 30_000,
            Self::Agent { .. } => 120_000,
        }
    }

    /// Whether this handler runs asynchronously (fire-and-forget).
    pub fn is_async(&self) -> bool {
        match self {
            Self::Command { r#async, .. } | Self::Http { r#async, .. } => *r#async,
            // Prompt and agent handlers do not support async mode.
            Self::Prompt { .. } | Self::Agent { .. } => false,
        }
    }
}

/// Controls what data is serialized to hook handlers.
/// Per design §Security: "Hook handlers receive summaries by default, not raw content."
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookContextVerbosity {
    /// Keys and types only; no content.
    Minimal,
    /// Summaries and metadata (default).
    #[default]
    Standard,
    /// Full raw content.
    Full,
}

// ---------------------------------------------------------------------------
// Main HookConfig
// ---------------------------------------------------------------------------

/// Configuration for the hook system.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookConfig {
    /// Per-middleware timeout in milliseconds.
    #[serde(default = "default_middleware_timeout_ms")]
    pub middleware_timeout_ms: u64,

    /// Event bus channel capacity per subscriber.
    #[serde(default = "default_event_channel_capacity")]
    pub event_channel_capacity: usize,

    /// Maximum number of subscribers allowed.
    #[serde(default = "default_max_subscribers")]
    pub max_subscribers: usize,

    /// External hook handler groups, keyed by hook point name (`snake_case`).
    /// Example key: "`pre_tool_execute`"
    #[serde(default)]
    pub hook_handlers: HashMap<String, Vec<HookHandlerGroupConfig>>,

    /// Global enable/disable for external hook handlers.
    #[serde(default = "default_true")]
    pub handlers_enabled: bool,

    /// Directories from which command hook scripts can be loaded.
    /// Empty = any directory allowed.
    /// Per design §Security: "Scripts must be absolute paths."
    #[serde(default)]
    pub allowed_hook_dirs: Vec<String>,

    /// Controls data verbosity in hook handler payloads.
    #[serde(default)]
    pub verbosity: HookContextVerbosity,
}

fn default_middleware_timeout_ms() -> u64 {
    5000
}

fn default_event_channel_capacity() -> usize {
    1000
}

fn default_max_subscribers() -> usize {
    100
}

fn default_true() -> bool {
    true
}

impl Default for HookConfig {
    fn default() -> Self {
        Self {
            middleware_timeout_ms: default_middleware_timeout_ms(),
            event_channel_capacity: default_event_channel_capacity(),
            max_subscribers: default_max_subscribers(),
            hook_handlers: HashMap::new(),
            handlers_enabled: true,
            allowed_hook_dirs: Vec::new(),
            verbosity: HookContextVerbosity::default(),
        }
    }
}

impl HookConfig {
    /// Get middleware timeout as a `Duration`.
    pub fn middleware_timeout(&self) -> Duration {
        Duration::from_millis(self.middleware_timeout_ms)
    }
}

// ---------------------------------------------------------------------------
// Config validation
// ---------------------------------------------------------------------------

/// Hook points that support prompt and agent handlers.
const DECISION_CAPABLE_HOOK_POINTS: &[&str] = &[
    "pre_tool_execute",
    "post_tool_execute",
    "pre_llm_call",
    "agent_loop_start",
    "agent_loop_end",
    "pre_compaction",
];

/// Validate handler config at load time.
///
/// - Prompt/agent handlers only on `DECISION_CAPABLE_HOOK_POINTS`.
/// - Command scripts must be absolute paths.
/// - If `allowed_hook_dirs` is non-empty, command scripts must be within allowed dirs.
pub fn validate_hook_handler_config(config: &HookConfig) -> Result<(), HookError> {
    for (hook_point, groups) in &config.hook_handlers {
        for group in groups {
            for handler in &group.handlers {
                match handler {
                    HandlerConfig::Prompt { .. } | HandlerConfig::Agent { .. } => {
                        if !DECISION_CAPABLE_HOOK_POINTS.contains(&hook_point.as_str()) {
                            return Err(HookError::HookHandlerValidation {
                                message: format!(
                                    "{} handler not allowed on hook point '{}'; \
                                     only supported on: {}",
                                    handler.handler_type(),
                                    hook_point,
                                    DECISION_CAPABLE_HOOK_POINTS.join(", ")
                                ),
                            });
                        }
                    }
                    HandlerConfig::Command { command, .. } => {
                        // Command scripts must be absolute paths.
                        if !command.starts_with('/') {
                            return Err(HookError::HookHandlerValidation {
                                message: format!(
                                    "command hook script must be an absolute path, got: '{command}'"
                                ),
                            });
                        }

                        // If allowed_hook_dirs is set, script must be within.
                        if !config.allowed_hook_dirs.is_empty() {
                            let in_allowed = config
                                .allowed_hook_dirs
                                .iter()
                                .any(|dir| command.starts_with(dir));
                            if !in_allowed {
                                return Err(HookError::HookHandlerValidation {
                                    message: format!(
                                        "command hook script '{}' is not in allowed directories: {:?}",
                                        command, config.allowed_hook_dirs
                                    ),
                                });
                            }
                        }
                    }
                    HandlerConfig::Http { .. } => {}
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_defaults() {
        let config = HookConfig::default();
        assert_eq!(config.middleware_timeout_ms, 5000);
        assert_eq!(config.event_channel_capacity, 1000);
        assert_eq!(config.max_subscribers, 100);
        assert!(config.handlers_enabled);
        assert!(config.hook_handlers.is_empty());
        assert!(config.allowed_hook_dirs.is_empty());
    }

    #[test]
    fn test_middleware_timeout_duration() {
        let config = HookConfig::default();
        assert_eq!(config.middleware_timeout(), Duration::from_millis(5000));
    }

    #[test]
    fn test_handler_config_command() {
        let toml = r#"
            [[handlers]]
            type = "command"
            command = "/usr/local/bin/hook.sh"
        "#;

        #[derive(Deserialize)]
        struct Wrapper {
            handlers: Vec<HandlerConfig>,
        }
        let w: Wrapper = toml::from_str(toml).unwrap();
        assert_eq!(w.handlers.len(), 1);
        assert!(
            matches!(&w.handlers[0], HandlerConfig::Command { command, .. } if command == "/usr/local/bin/hook.sh")
        );
        assert!(!w.handlers[0].is_async());
    }

    #[test]
    fn test_handler_config_http() {
        let toml = r#"
            [[handlers]]
            type = "http"
            url = "http://localhost:8080/hook"
            headers = { Authorization = "Bearer $MY_TOKEN" }
        "#;

        #[derive(Deserialize)]
        struct Wrapper {
            handlers: Vec<HandlerConfig>,
        }
        let w: Wrapper = toml::from_str(toml).unwrap();
        assert!(
            matches!(&w.handlers[0], HandlerConfig::Http { url, headers, .. }
            if url == "http://localhost:8080/hook" && headers.contains_key("Authorization"))
        );
    }

    #[test]
    fn test_handler_config_prompt() {
        let toml = r#"
            [[handlers]]
            type = "prompt"
            prompt = "Evaluate: $ARGUMENTS"
            model = "haiku"
        "#;

        #[derive(Deserialize)]
        struct Wrapper {
            handlers: Vec<HandlerConfig>,
        }
        let w: Wrapper = toml::from_str(toml).unwrap();
        assert!(
            matches!(&w.handlers[0], HandlerConfig::Prompt { prompt, model }
            if prompt == "Evaluate: $ARGUMENTS" && model.as_deref() == Some("haiku"))
        );
    }

    #[test]
    fn test_handler_config_agent() {
        let toml = r#"
            [[handlers]]
            type = "agent"
            prompt = "Verify safety: $ARGUMENTS"
        "#;

        #[derive(Deserialize)]
        struct Wrapper {
            handlers: Vec<HandlerConfig>,
        }
        let w: Wrapper = toml::from_str(toml).unwrap();
        assert!(
            matches!(&w.handlers[0], HandlerConfig::Agent { prompt, model }
            if prompt == "Verify safety: $ARGUMENTS" && model.is_none())
        );
    }

    #[test]
    fn test_handler_config_with_matcher() {
        let toml = r#"
            matcher = "Bash|ShellExec"
            timeout_ms = 10000

            [[handlers]]
            type = "command"
            command = "/usr/bin/check.sh"
        "#;

        let group: HookHandlerGroupConfig = toml::from_str(toml).unwrap();
        assert_eq!(group.matcher, "Bash|ShellExec");
        assert_eq!(group.timeout_ms, Some(10000));
        assert_eq!(group.handlers.len(), 1);
    }

    #[test]
    fn test_handler_config_empty() {
        let config = HookConfig::default();
        assert!(config.hook_handlers.is_empty());
        assert!(validate_hook_handler_config(&config).is_ok());
    }

    #[test]
    fn test_handler_config_default_timeouts() {
        let cmd = HandlerConfig::Command {
            command: "/bin/test".into(),
            r#async: false,
        };
        assert_eq!(cmd.default_timeout_ms(), 5000);

        let http = HandlerConfig::Http {
            url: "http://localhost".into(),
            headers: HashMap::new(),
            r#async: false,
        };
        assert_eq!(http.default_timeout_ms(), 5000);

        let prompt = HandlerConfig::Prompt {
            prompt: "test".into(),
            model: None,
        };
        assert_eq!(prompt.default_timeout_ms(), 30_000);

        let agent = HandlerConfig::Agent {
            prompt: "test".into(),
            model: None,
        };
        assert_eq!(agent.default_timeout_ms(), 120_000);
    }

    #[test]
    fn test_validate_prompt_on_unsupported_point() {
        let mut config = HookConfig::default();
        config.hook_handlers.insert(
            "session_created".into(),
            vec![HookHandlerGroupConfig {
                matcher: "*".into(),
                timeout_ms: None,
                handlers: vec![HandlerConfig::Prompt {
                    prompt: "test".into(),
                    model: None,
                }],
            }],
        );
        let err = validate_hook_handler_config(&config).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("prompt handler not allowed"), "{msg}");
        assert!(msg.contains("session_created"), "{msg}");
    }

    #[test]
    fn test_validate_agent_on_unsupported_point() {
        let mut config = HookConfig::default();
        config.hook_handlers.insert(
            "memory_stored".into(),
            vec![HookHandlerGroupConfig {
                matcher: "*".into(),
                timeout_ms: None,
                handlers: vec![HandlerConfig::Agent {
                    prompt: "test".into(),
                    model: None,
                }],
            }],
        );
        let err = validate_hook_handler_config(&config).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("agent handler not allowed"), "{msg}");
    }

    #[test]
    fn test_validate_command_script_not_absolute() {
        let mut config = HookConfig::default();
        config.hook_handlers.insert(
            "pre_tool_execute".into(),
            vec![HookHandlerGroupConfig {
                matcher: "*".into(),
                timeout_ms: None,
                handlers: vec![HandlerConfig::Command {
                    command: "./script.sh".into(),
                    r#async: false,
                }],
            }],
        );
        let err = validate_hook_handler_config(&config).unwrap_err();
        assert!(err.to_string().contains("absolute path"));
    }

    #[test]
    fn test_validate_allowed_hook_dirs() {
        let mut config = HookConfig::default();
        config.allowed_hook_dirs = vec!["/home/user/.y-agent/hooks".into()];
        config.hook_handlers.insert(
            "pre_tool_execute".into(),
            vec![HookHandlerGroupConfig {
                matcher: "*".into(),
                timeout_ms: None,
                handlers: vec![HandlerConfig::Command {
                    command: "/usr/local/bin/other.sh".into(),
                    r#async: false,
                }],
            }],
        );
        let err = validate_hook_handler_config(&config).unwrap_err();
        assert!(err.to_string().contains("not in allowed directories"));
    }
}
