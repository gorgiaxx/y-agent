//! Hook Handler Executor: 4 handler types, matcher, JSON I/O, decision aggregation.
//!
//! Design reference: hooks-plugin-design.md §Hook Handler Types
//!
//! External extensibility via configuration-driven hook handlers:
//! - **Command hooks**: shell scripts with JSON stdin/stdout
//! - **HTTP hooks**: POST JSON to URL endpoints
//! - **Prompt hooks**: single-turn LLM evaluation (feature-gated `llm_hooks`)
//! - **Agent hooks**: multi-turn subagent verification (feature-gated `llm_hooks`)

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(feature = "llm_hooks")]
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tracing::{debug, error, info, warn};

use y_core::hook::HookPoint;
#[cfg(feature = "llm_hooks")]
use y_core::hook::{HookAgentRunner, HookLlmRunner};

use crate::config::{HandlerConfig, HookConfig, HookContextVerbosity};
use crate::error::HookError;

// ---------------------------------------------------------------------------
// Core result types
// ---------------------------------------------------------------------------

/// Aggregate result from executing all handlers for a hook point.
#[derive(Debug, Clone)]
pub struct HookHandlerResult {
    /// "allow" or "block".
    pub decision: HookDecision,
    /// Concatenated reasons from all blocking handlers.
    pub reasons: Vec<String>,
    /// Concatenated context messages to inject into agent.
    pub context_messages: Vec<String>,
    /// Number of handlers that executed.
    pub handler_count: usize,
    /// Number of handlers that returned block.
    pub block_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookDecision {
    Allow,
    Block,
}

impl Default for HookHandlerResult {
    fn default() -> Self {
        Self {
            decision: HookDecision::Allow,
            reasons: Vec::new(),
            context_messages: Vec::new(),
            handler_count: 0,
            block_count: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Decision types per handler kind
// ---------------------------------------------------------------------------

/// Decision returned by a command/HTTP hook handler (JSON stdout or response body).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandHttpDecision {
    #[serde(default = "default_allow")]
    pub decision: String,
    pub reason: Option<String>,
    pub context_message: Option<String>,
    #[serde(default)]
    pub suppress_output: bool,
}

impl Default for CommandHttpDecision {
    fn default() -> Self {
        Self {
            decision: "allow".to_string(),
            reason: None,
            context_message: None,
            suppress_output: false,
        }
    }
}

fn default_allow() -> String {
    "allow".to_string()
}

/// Decision returned by a prompt/agent hook handler.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptAgentDecision {
    /// true = allow, false = block
    pub ok: bool,
    pub reason: Option<String>,
}

// ---------------------------------------------------------------------------
// Hook input
// ---------------------------------------------------------------------------

/// Common hook input fields serialized as JSON.
#[derive(Debug, Serialize)]
pub struct HookInput {
    pub session_id: Option<String>,
    /// Hook point name (e.g., "`pre_tool_execute`").
    pub hook_event: String,
    /// ISO 8601 timestamp.
    pub timestamp: String,
    /// Hook-specific fields from `HookPoints` table.
    #[serde(flatten)]
    pub extra: serde_json::Value,
}

// ---------------------------------------------------------------------------
// Compiled handler group
// ---------------------------------------------------------------------------

/// A handler group with compiled matcher regex.
#[cfg(feature = "hook_handlers")]
struct CompiledHandlerGroup {
    /// None = match all ("*").
    matcher: Option<regex::Regex>,
    timeout: Duration,
    handlers: Vec<HandlerConfig>,
}

// ---------------------------------------------------------------------------
// Metrics
// ---------------------------------------------------------------------------

/// Metrics for hook handler execution.
#[derive(Debug, Default)]
pub struct HookHandlerMetrics {
    /// Total handler invocations.
    pub invocations: AtomicU64,
    /// Total duration in microseconds.
    pub duration_us: AtomicU64,
    /// Total errors (handler failures that defaulted to allow).
    pub errors: AtomicU64,
    /// Total timeouts.
    pub timeouts: AtomicU64,
    /// Total blocks (handlers that returned block/ok:false).
    pub blocks: AtomicU64,
}

/// A snapshot of metrics for inspection.
#[derive(Debug, Clone)]
pub struct HookHandlerMetricsSnapshot {
    pub invocations: u64,
    pub duration_us: u64,
    pub errors: u64,
    pub timeouts: u64,
    pub blocks: u64,
}

impl HookHandlerMetrics {
    pub fn snapshot(&self) -> HookHandlerMetricsSnapshot {
        HookHandlerMetricsSnapshot {
            invocations: self.invocations.load(Ordering::Relaxed),
            duration_us: self.duration_us.load(Ordering::Relaxed),
            errors: self.errors.load(Ordering::Relaxed),
            timeouts: self.timeouts.load(Ordering::Relaxed),
            blocks: self.blocks.load(Ordering::Relaxed),
        }
    }
}

// ---------------------------------------------------------------------------
// Environment variable substitution
// ---------------------------------------------------------------------------

/// Replace $`VAR_NAME` and ${`VAR_NAME`} patterns in a string with env var values.
/// Missing env vars are replaced with empty strings.
pub fn expand_env_vars(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '$' {
            if chars.peek() == Some(&'{') {
                // ${VAR_NAME} form
                chars.next(); // consume '{'
                let mut var_name = String::new();
                for c in chars.by_ref() {
                    if c == '}' {
                        break;
                    }
                    var_name.push(c);
                }
                result.push_str(&std::env::var(&var_name).unwrap_or_default());
            } else {
                // $VAR_NAME form
                let mut var_name = String::new();
                while let Some(&c) = chars.peek() {
                    if c.is_ascii_alphanumeric() || c == '_' {
                        var_name.push(c);
                        chars.next();
                    } else {
                        break;
                    }
                }
                if var_name.is_empty() {
                    result.push('$');
                } else {
                    result.push_str(&std::env::var(&var_name).unwrap_or_default());
                }
            }
        } else {
            result.push(ch);
        }
    }

    result
}

// ---------------------------------------------------------------------------
// Hook Handler Executor
// ---------------------------------------------------------------------------

/// Executor for configuration-driven hook handlers.
///
/// Manages matching, dispatching, and aggregating results from
/// command, HTTP, prompt, and agent hook handlers.
#[cfg(feature = "hook_handlers")]
pub struct HookHandlerExecutor {
    /// Compiled handlers per hook point.
    handlers: HashMap<HookPoint, Vec<CompiledHandlerGroup>>,
    /// Shared HTTP client with connection pooling.
    http_client: reqwest::Client,
    /// Allowed hook script directories.
    allowed_hook_dirs: Vec<String>,
    /// Context verbosity level.
    #[allow(dead_code)]
    verbosity: HookContextVerbosity,
    /// Metrics counters.
    metrics: HookHandlerMetrics,
    /// LLM runner for prompt hooks (injected post-construction).
    #[cfg(feature = "llm_hooks")]
    llm_runner: Option<Arc<dyn HookLlmRunner>>,
    /// Agent runner for agent hooks (injected post-construction).
    #[cfg(feature = "llm_hooks")]
    agent_runner: Option<Arc<dyn HookAgentRunner>>,
}

#[cfg(feature = "hook_handlers")]
impl HookHandlerExecutor {
    /// Parse config, compile regex matchers, validate prompt/agent hook points.
    /// Returns Err if config validation fails.
    pub fn from_config(config: &HookConfig) -> Result<Self, HookError> {
        use crate::config::validate_hook_handler_config;
        validate_hook_handler_config(config)?;

        let mut handlers: HashMap<HookPoint, Vec<CompiledHandlerGroup>> = HashMap::new();

        for (hook_point_str, groups) in &config.hook_handlers {
            let hook_point = parse_hook_point(hook_point_str)?;

            let compiled_groups: Vec<CompiledHandlerGroup> = groups
                .iter()
                .map(|group| {
                    let matcher = if group.matcher == "*" || group.matcher.is_empty() {
                        None
                    } else {
                        Some(regex::Regex::new(&group.matcher).map_err(|e| {
                            HookError::HookHandlerValidation {
                                message: format!("invalid matcher regex '{}': {e}", group.matcher),
                            }
                        })?)
                    };

                    // Determine timeout: group-level override or per-handler default.
                    // Use group-level if set, otherwise pick the max default across handlers.
                    let timeout_ms = group.timeout_ms.unwrap_or_else(|| {
                        group
                            .handlers
                            .iter()
                            .map(super::config::HandlerConfig::default_timeout_ms)
                            .max()
                            .unwrap_or(5000)
                    });

                    Ok(CompiledHandlerGroup {
                        matcher,
                        timeout: Duration::from_millis(timeout_ms),
                        handlers: group.handlers.clone(),
                    })
                })
                .collect::<Result<Vec<_>, HookError>>()?;

            handlers
                .entry(hook_point)
                .or_default()
                .extend(compiled_groups);
        }

        Ok(Self {
            handlers,
            http_client: reqwest::Client::new(),
            allowed_hook_dirs: config.allowed_hook_dirs.clone(),
            verbosity: config.verbosity.clone(),
            metrics: HookHandlerMetrics::default(),
            #[cfg(feature = "llm_hooks")]
            llm_runner: None,
            #[cfg(feature = "llm_hooks")]
            agent_runner: None,
        })
    }

    /// Inject an LLM runner for prompt hook execution.
    ///
    /// Called post-construction from `HookSystem::set_llm_runner()` after
    /// the provider pool is initialized during application startup.
    #[cfg(feature = "llm_hooks")]
    pub fn set_llm_runner(&mut self, runner: Arc<dyn HookLlmRunner>) {
        self.llm_runner = Some(runner);
    }

    /// Inject an agent runner for agent hook execution.
    ///
    /// Called post-construction from `HookSystem::set_agent_runner()` after
    /// y-agent is initialized during application startup.
    #[cfg(feature = "llm_hooks")]
    pub fn set_agent_runner(&mut self, runner: Arc<dyn HookAgentRunner>) {
        self.agent_runner = Some(runner);
    }

    /// Execute all matching handlers for a hook point.
    /// Returns aggregate decision.
    /// Per design §Failure Handling: if any sync handler returns "block", aggregate is "block".
    pub async fn execute(&self, hook_point: HookPoint, input: &HookInput) -> HookHandlerResult {
        let Some(groups) = self.handlers.get(&hook_point) else {
            return HookHandlerResult::default();
        };

        let input_json = match serde_json::to_vec(input) {
            Ok(json) => json,
            Err(e) => {
                error!(error = %e, "failed to serialize hook input");
                self.metrics.errors.fetch_add(1, Ordering::Relaxed);
                return HookHandlerResult::default();
            }
        };

        // Determine match subject from input extra fields.
        let match_subject = extract_match_subject(input);

        let mut cmd_http_decisions: Vec<CommandHttpDecision> = Vec::new();
        #[allow(unused_mut)]
        let mut prompt_agent_decisions: Vec<PromptAgentDecision> = Vec::new();
        let mut handler_count: usize = 0;

        let start = std::time::Instant::now();

        for group in groups {
            // Check matcher.
            if let Some(ref regex) = group.matcher {
                if !regex.is_match(&match_subject) {
                    continue;
                }
            }

            for handler in &group.handlers {
                handler_count += 1;
                self.metrics.invocations.fetch_add(1, Ordering::Relaxed);

                let timeout = group.timeout;

                match handler {
                    HandlerConfig::Command { command, r#async } => {
                        if *r#async {
                            let cmd = command.clone();
                            let json = input_json.clone();
                            let allowed = self.allowed_hook_dirs.clone();
                            tokio::spawn(async move {
                                let executor = CommandExecutor {
                                    allowed_hook_dirs: allowed,
                                };
                                if let Err(e) = executor.execute(&cmd, &json, timeout).await {
                                    warn!(error = %e, command = %cmd, "async command hook failed");
                                }
                            });
                        } else {
                            let executor = CommandExecutor {
                                allowed_hook_dirs: self.allowed_hook_dirs.clone(),
                            };
                            match executor.execute(command, &input_json, timeout).await {
                                Ok(decision) => cmd_http_decisions.push(decision),
                                Err(e) => {
                                    warn!(error = %e, command = %command, "command hook failed, defaulting to allow");
                                    self.metrics.errors.fetch_add(1, Ordering::Relaxed);
                                }
                            }
                        }
                    }
                    HandlerConfig::Http {
                        url,
                        headers,
                        r#async,
                    } => {
                        if *r#async {
                            let url = url.clone();
                            let headers = headers.clone();
                            let json = input_json.clone();
                            let client = self.http_client.clone();
                            tokio::spawn(async move {
                                if let Err(e) =
                                    execute_http(&client, &url, &headers, &json, timeout).await
                                {
                                    warn!(error = %e, url = %url, "async HTTP hook failed");
                                }
                            });
                        } else {
                            match execute_http(
                                &self.http_client,
                                url,
                                headers,
                                &input_json,
                                timeout,
                            )
                            .await
                            {
                                Ok(decision) => cmd_http_decisions.push(decision),
                                Err(e) => {
                                    warn!(error = %e, url = %url, "HTTP hook failed, defaulting to allow");
                                    self.metrics.errors.fetch_add(1, Ordering::Relaxed);
                                }
                            }
                        }
                    }
                    #[cfg(feature = "llm_hooks")]
                    HandlerConfig::Prompt { prompt, model } => {
                        match execute_prompt(
                            self.llm_runner.as_deref(),
                            prompt,
                            model.as_deref(),
                            input,
                            timeout,
                        )
                        .await
                        {
                            Ok(decision) => prompt_agent_decisions.push(decision),
                            Err(e) => {
                                warn!(error = %e, "prompt hook failed, defaulting to allow");
                                self.metrics.errors.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }
                    #[cfg(feature = "llm_hooks")]
                    HandlerConfig::Agent { prompt, model } => {
                        match execute_agent(
                            self.agent_runner.as_deref(),
                            prompt,
                            model.as_deref(),
                            input,
                            timeout,
                        )
                        .await
                        {
                            Ok(decision) => prompt_agent_decisions.push(decision),
                            Err(e) => {
                                warn!(error = %e, "agent hook failed, defaulting to allow");
                                self.metrics.errors.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }
                    // When llm_hooks feature is not enabled, prompt/agent handlers are
                    // compiled but not executable — log and skip.
                    #[cfg(not(feature = "llm_hooks"))]
                    HandlerConfig::Prompt { .. } | HandlerConfig::Agent { .. } => {
                        warn!(
                            handler_type = handler.handler_type(),
                            "prompt/agent hook handler requires 'llm_hooks' feature; skipping"
                        );
                    }
                }
            }
        }

        let elapsed = start.elapsed();
        self.metrics.duration_us.fetch_add(
            u64::try_from(elapsed.as_micros()).unwrap_or(0),
            Ordering::Relaxed,
        );

        let result = aggregate_decisions(&cmd_http_decisions, &prompt_agent_decisions);

        if result.block_count > 0 {
            self.metrics
                .blocks
                .fetch_add(result.block_count as u64, Ordering::Relaxed);
        }

        debug!(
            hook_point = ?hook_point,
            handler_count,
            decision = ?result.decision,
            block_count = result.block_count,
            elapsed_us = u64::try_from(elapsed.as_micros()).unwrap_or(0),
            "hook handlers executed"
        );

        HookHandlerResult {
            handler_count,
            ..result
        }
    }

    /// Get metrics for diagnostics.
    pub fn metrics(&self) -> &HookHandlerMetrics {
        &self.metrics
    }

    /// Check if any handlers are configured for a hook point.
    pub fn has_handlers(&self, hook_point: &HookPoint) -> bool {
        self.handlers.contains_key(hook_point)
    }
}

#[cfg(feature = "hook_handlers")]
impl std::fmt::Debug for HookHandlerExecutor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HookHandlerExecutor")
            .field("hook_points", &self.handlers.keys().collect::<Vec<_>>())
            .field(
                "total_groups",
                &self.handlers.values().map(Vec::len).sum::<usize>(),
            )
            .field("metrics", &self.metrics.snapshot())
            .finish_non_exhaustive()
    }
}

// ---------------------------------------------------------------------------
// Hook point parsing
// ---------------------------------------------------------------------------

/// Parse a `snake_case` hook point name into a `HookPoint` enum variant.
fn parse_hook_point(s: &str) -> Result<HookPoint, HookError> {
    match s {
        "pre_llm_call" => Ok(HookPoint::PreLlmCall),
        "post_llm_call" => Ok(HookPoint::PostLlmCall),
        "pre_tool_execute" => Ok(HookPoint::PreToolExecute),
        "post_tool_execute" => Ok(HookPoint::PostToolExecute),
        "memory_stored" => Ok(HookPoint::MemoryStored),
        "memory_recalled" => Ok(HookPoint::MemoryRecalled),
        "session_created" => Ok(HookPoint::SessionCreated),
        "session_closed" => Ok(HookPoint::SessionClosed),
        "pre_compaction" => Ok(HookPoint::PreCompaction),
        "post_compaction" => Ok(HookPoint::PostCompaction),
        "workflow_started" => Ok(HookPoint::WorkflowStarted),
        "workflow_completed" => Ok(HookPoint::WorkflowCompleted),
        "agent_loop_start" => Ok(HookPoint::AgentLoopStart),
        "agent_loop_end" => Ok(HookPoint::AgentLoopEnd),
        "pre_pipeline_step" => Ok(HookPoint::PrePipelineStep),
        "post_pipeline_step" => Ok(HookPoint::PostPipelineStep),
        "tool_gap_detected" => Ok(HookPoint::ToolGapDetected),
        "tool_gap_resolved" => Ok(HookPoint::ToolGapResolved),
        "agent_gap_detected" => Ok(HookPoint::AgentGapDetected),
        "agent_gap_resolved" => Ok(HookPoint::AgentGapResolved),
        "dynamic_agent_created" => Ok(HookPoint::DynamicAgentCreated),
        "dynamic_agent_deactivated" => Ok(HookPoint::DynamicAgentDeactivated),
        "context_overflow" => Ok(HookPoint::ContextOverflow),
        "post_skill_injection" => Ok(HookPoint::PostSkillInjection),
        _ => Err(HookError::HookHandlerValidation {
            message: format!("unknown hook point: '{s}'"),
        }),
    }
}

/// Extract a match subject from the hook input for regex matching.
/// For tool hooks, matches against `tool_name`. For others, uses `hook_event`.
fn extract_match_subject(input: &HookInput) -> String {
    if let Some(tool_name) = input.extra.get("tool_name").and_then(|v| v.as_str()) {
        tool_name.to_string()
    } else {
        input.hook_event.clone()
    }
}

// ---------------------------------------------------------------------------
// Decision aggregation
// ---------------------------------------------------------------------------

/// Aggregate decisions from all synchronous handlers.
///
/// Per design §Failure Handling → "Multiple handlers disagree":
/// - If any `CommandHttpDecision.decision == "block"` → aggregate = Block.
/// - If any `PromptAgentDecision.ok == false` → aggregate = Block.
/// - Collect all `reason` and `context_message` values.
/// - If all allow → aggregate = Allow.
pub fn aggregate_decisions(
    cmd_http_decisions: &[CommandHttpDecision],
    prompt_agent_decisions: &[PromptAgentDecision],
) -> HookHandlerResult {
    let mut result = HookHandlerResult::default();
    let mut block_count = 0_usize;

    result.handler_count = cmd_http_decisions.len() + prompt_agent_decisions.len();

    for d in cmd_http_decisions {
        if d.decision == "block" {
            block_count += 1;
            if let Some(ref reason) = d.reason {
                result.reasons.push(reason.clone());
            }
        }
        if let Some(ref msg) = d.context_message {
            result.context_messages.push(msg.clone());
        }
    }

    for d in prompt_agent_decisions {
        if !d.ok {
            block_count += 1;
            if let Some(ref reason) = d.reason {
                result.reasons.push(reason.clone());
            }
        }
    }

    result.block_count = block_count;
    if block_count > 0 {
        result.decision = HookDecision::Block;
    }

    result
}

// ---------------------------------------------------------------------------
// Command hook execution
// ---------------------------------------------------------------------------

/// Internal command executor for isolation.
struct CommandExecutor {
    allowed_hook_dirs: Vec<String>,
}

impl CommandExecutor {
    /// Execute a command hook.
    ///
    /// Per design §Command Hook Fields:
    /// - Write JSON to stdin, close stdin.
    /// - Exit code 0: allow. Parse optional JSON from stdout.
    /// - Exit code 1: non-blocking error. Log. Return allow.
    /// - Exit code 2: block. Parse reason from JSON or stderr.
    /// - Other: non-blocking error. Log. Return allow.
    async fn execute(
        &self,
        cmd: &str,
        input_json: &[u8],
        timeout: Duration,
    ) -> Result<CommandHttpDecision, HookError> {
        use tokio::io::AsyncReadExt;
        use tokio::io::AsyncWriteExt;

        // Validate script path.
        if !cmd.starts_with('/') {
            return Err(HookError::HookHandlerValidation {
                message: format!("command hook must be absolute path: '{cmd}'"),
            });
        }

        if !self.allowed_hook_dirs.is_empty() {
            let in_allowed = self
                .allowed_hook_dirs
                .iter()
                .any(|dir| cmd.starts_with(dir));
            if !in_allowed {
                return Err(HookError::HookHandlerValidation {
                    message: format!(
                        "command hook '{cmd}' not in allowed directories: {:?}",
                        self.allowed_hook_dirs
                    ),
                });
            }
        }

        let mut child = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| HookError::HookHandlerError {
                handler_type: "command".into(),
                message: format!("failed to spawn: {e}"),
            })?;

        // Write input to stdin.
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(input_json).await.ok();
            drop(stdin);
        }

        // Take stdout/stderr handles before waiting (wait_with_output takes ownership).
        let stdout_handle = child.stdout.take();
        let stderr_handle = child.stderr.take();

        // Wait with timeout.
        let status = match tokio::time::timeout(timeout, child.wait()).await {
            Ok(Ok(status)) => status,
            Ok(Err(e)) => {
                return Err(HookError::HookHandlerError {
                    handler_type: "command".into(),
                    message: format!("wait failed: {e}"),
                });
            }
            Err(_) => {
                // Timeout — kill process.
                child.kill().await.ok();
                return Err(HookError::HookHandlerTimeout {
                    handler_type: "command".into(),
                    timeout_ms: u64::try_from(timeout.as_millis()).unwrap_or(0),
                });
            }
        };

        // Read stdout and stderr.
        let mut stdout_buf = Vec::new();
        let mut stderr_buf = Vec::new();
        if let Some(mut h) = stdout_handle {
            h.read_to_end(&mut stdout_buf).await.ok();
        }
        if let Some(mut h) = stderr_handle {
            h.read_to_end(&mut stderr_buf).await.ok();
        }

        let exit_code = status.code().unwrap_or(-1);
        let stdout = String::from_utf8_lossy(&stdout_buf);
        let stderr = String::from_utf8_lossy(&stderr_buf);

        match exit_code {
            0 => {
                // Allow. Parse optional JSON from stdout.
                if stdout.trim().is_empty() {
                    Ok(CommandHttpDecision::default())
                } else {
                    serde_json::from_str(stdout.trim())
                        .unwrap_or_else(|_| CommandHttpDecision::default())
                        .pipe(Ok)
                }
            }
            1 => {
                // Non-blocking error.
                info!(
                    command = cmd,
                    stderr = %stderr,
                    "command hook exit 1 (non-blocking error)"
                );
                Ok(CommandHttpDecision::default())
            }
            2 => {
                // Block.
                let decision = if stdout.trim().is_empty() {
                    CommandHttpDecision {
                        decision: "block".into(),
                        reason: Some(stderr.trim().to_string()),
                        ..Default::default()
                    }
                } else {
                    serde_json::from_str::<CommandHttpDecision>(stdout.trim()).unwrap_or_else(
                        |_| CommandHttpDecision {
                            decision: "block".into(),
                            reason: Some(stderr.trim().to_string()),
                            ..Default::default()
                        },
                    )
                };
                Ok(decision)
            }
            other => {
                info!(
                    command = cmd,
                    exit_code = other,
                    stderr = %stderr,
                    "command hook unexpected exit code, defaulting to allow"
                );
                Ok(CommandHttpDecision::default())
            }
        }
    }
}

/// Pipe trait for inline chaining.
trait Pipe: Sized {
    fn pipe<F, R>(self, f: F) -> R
    where
        F: FnOnce(Self) -> R,
    {
        f(self)
    }
}

impl<T> Pipe for T {}

// ---------------------------------------------------------------------------
// HTTP hook execution
// ---------------------------------------------------------------------------

/// Execute an HTTP hook handler.
///
/// Per design §HTTP Hook Fields:
/// - POST JSON body to URL with Content-Type: application/json.
/// - Expand $`ENV_VAR` in headers.
/// - 2xx + JSON → parse as `CommandHttpDecision`.
/// - 2xx + empty → default allow.
/// - Non-2xx → log, return allow.
/// - Connection error / timeout → log, return allow.
#[cfg(feature = "hook_handlers")]
async fn execute_http(
    client: &reqwest::Client,
    url: &str,
    headers: &HashMap<String, String>,
    input_json: &[u8],
    timeout: Duration,
) -> Result<CommandHttpDecision, HookError> {
    let mut request = client
        .post(url)
        .header("Content-Type", "application/json")
        .timeout(timeout)
        .body(input_json.to_vec());

    // Expand env vars in headers.
    for (key, value) in headers {
        let expanded = expand_env_vars(value);
        request = request.header(key.as_str(), expanded);
    }

    let response = match request.send().await {
        Ok(resp) => resp,
        Err(e) => {
            if e.is_timeout() {
                return Err(HookError::HookHandlerTimeout {
                    handler_type: "http".into(),
                    timeout_ms: u64::try_from(timeout.as_millis()).unwrap_or(0),
                });
            }
            return Err(HookError::HookHandlerError {
                handler_type: "http".into(),
                message: format!("request failed: {e}"),
            });
        }
    };

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        info!(
            url,
            status = status.as_u16(),
            body = %body,
            "HTTP hook returned non-2xx, defaulting to allow"
        );
        return Ok(CommandHttpDecision::default());
    }

    let body = response.text().await.unwrap_or_default();
    if body.trim().is_empty() {
        return Ok(CommandHttpDecision::default());
    }

    Ok(serde_json::from_str(body.trim()).unwrap_or_else(|e| {
        warn!(url, error = %e, "HTTP hook response not valid JSON, defaulting to allow");
        CommandHttpDecision::default()
    }))
}

// ---------------------------------------------------------------------------
// Prompt hook execution (feature-gated)
// ---------------------------------------------------------------------------

/// Execute a prompt hook handler.
///
/// Per design §Prompt Hook Fields:
/// 1. Replace $ARGUMENTS in prompt template with hook input JSON.
/// 2. Send single-turn completion request via `HookLlmRunner`.
/// 3. Parse response as PromptAgentDecision.
/// 4. On error → default allow.
#[cfg(feature = "llm_hooks")]
async fn execute_prompt(
    llm_runner: Option<&dyn HookLlmRunner>,
    prompt_template: &str,
    model: Option<&str>,
    input: &HookInput,
    timeout: Duration,
) -> Result<PromptAgentDecision, HookError> {
    let input_json = serde_json::to_string(input).map_err(|e| HookError::HookHandlerError {
        handler_type: "prompt".into(),
        message: format!("failed to serialize input: {e}"),
    })?;

    let user_message = prompt_template.replace("$ARGUMENTS", &input_json);

    let runner = match llm_runner {
        Some(r) => r,
        None => {
            warn!("prompt hook: no HookLlmRunner injected, returning default allow");
            return Ok(PromptAgentDecision {
                ok: true,
                reason: None,
            });
        }
    };

    let system_prompt = concat!(
        "You are a hook handler for a coding agent. ",
        "Evaluate the request and respond with ONLY a JSON object: ",
        "{\"ok\": true, \"reason\": \"...\"} to allow, or ",
        "{\"ok\": false, \"reason\": \"...\"} to block. ",
        "No markdown, no code fences, just raw JSON."
    );

    let response_text = runner
        .evaluate(system_prompt, &user_message, model, timeout)
        .await
        .map_err(|e| HookError::HookHandlerError {
            handler_type: "prompt".into(),
            message: format!("LLM evaluation failed: {e}"),
        })?;

    // Parse response as PromptAgentDecision.
    // Response may contain markdown fences or extra text — extract JSON.
    let json_str = extract_json_from_response(&response_text);
    serde_json::from_str::<PromptAgentDecision>(json_str).map_err(|e| {
        warn!(
            response = %response_text,
            error = %e,
            "prompt hook response not valid JSON, defaulting to allow"
        );
        HookError::HookHandlerError {
            handler_type: "prompt".into(),
            message: format!("failed to parse LLM response as JSON: {e}"),
        }
    })
}

// ---------------------------------------------------------------------------
// Agent hook execution (feature-gated)
// ---------------------------------------------------------------------------

/// Execute an agent hook handler.
///
/// Per design §Agent Hook Fields:
/// 1. Replace $ARGUMENTS in task prompt.
/// 2. Spawn subagent with Read/Grep/Glob tools only via `HookAgentRunner`.
/// 3. Max 50 turns.
/// 4. Parse final response as PromptAgentDecision.
/// 5. On timeout/max turns → default allow.
#[cfg(feature = "llm_hooks")]
async fn execute_agent(
    agent_runner: Option<&dyn HookAgentRunner>,
    prompt_template: &str,
    model: Option<&str>,
    input: &HookInput,
    timeout: Duration,
) -> Result<PromptAgentDecision, HookError> {
    let input_json = serde_json::to_string(input).map_err(|e| HookError::HookHandlerError {
        handler_type: "agent".into(),
        message: format!("failed to serialize input: {e}"),
    })?;

    let task_prompt = prompt_template.replace("$ARGUMENTS", &input_json);

    let runner = match agent_runner {
        Some(r) => r,
        None => {
            warn!("agent hook: no HookAgentRunner injected, returning default allow");
            return Ok(PromptAgentDecision {
                ok: true,
                reason: None,
            });
        }
    };

    const MAX_TURNS: u32 = 50;

    let response_text = runner
        .run_agent(&task_prompt, model, MAX_TURNS, timeout)
        .await
        .map_err(|e| HookError::HookHandlerError {
            handler_type: "agent".into(),
            message: format!("agent execution failed: {e}"),
        })?;

    // Parse response as PromptAgentDecision.
    let json_str = extract_json_from_response(&response_text);
    serde_json::from_str::<PromptAgentDecision>(json_str).map_err(|e| {
        warn!(
            response = %response_text,
            error = %e,
            "agent hook response not valid JSON, defaulting to allow"
        );
        HookError::HookHandlerError {
            handler_type: "agent".into(),
            message: format!("failed to parse agent response as JSON: {e}"),
        }
    })
}

/// Extract JSON object from a response that may contain markdown fences or extra text.
/// Returns the first `{...}` substring found, or the original text if no braces found.
#[cfg(feature = "llm_hooks")]
fn extract_json_from_response(text: &str) -> &str {
    let trimmed = text.trim();
    // Find the first '{' and last '}' to extract JSON.
    if let Some(start) = trimmed.find('{') {
        if let Some(end) = trimmed.rfind('}') {
            if end > start {
                return &trimmed[start..=end];
            }
        }
    }
    trimmed
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- expand_env_vars tests ---

    #[test]
    fn test_expand_env_vars_dollar_form() {
        temp_env::with_var("TEST_HOOK_VAR_1", Some("hello"), || {
            assert_eq!(expand_env_vars("Bearer $TEST_HOOK_VAR_1"), "Bearer hello");
        });
    }

    #[test]
    fn test_expand_env_vars_brace_form() {
        temp_env::with_var("TEST_HOOK_VAR_2", Some("world"), || {
            assert_eq!(
                expand_env_vars("prefix-${TEST_HOOK_VAR_2}-suffix"),
                "prefix-world-suffix"
            );
        });
    }

    #[test]
    fn test_expand_env_vars_missing() {
        assert_eq!(expand_env_vars("$NONEXISTENT_VAR_12345"), "");
    }

    #[test]
    fn test_expand_env_vars_no_vars() {
        assert_eq!(expand_env_vars("no variables here"), "no variables here");
    }

    // --- Decision aggregation tests ---

    #[test]
    fn test_decision_aggregation_all_allow() {
        let cmd = vec![
            CommandHttpDecision {
                decision: "allow".into(),
                reason: None,
                context_message: None,
                suppress_output: false,
            },
            CommandHttpDecision {
                decision: "allow".into(),
                reason: None,
                context_message: Some("info msg".into()),
                suppress_output: false,
            },
        ];
        let result = aggregate_decisions(&cmd, &[]);
        assert_eq!(result.decision, HookDecision::Allow);
        assert_eq!(result.block_count, 0);
        assert_eq!(result.context_messages, vec!["info msg"]);
    }

    #[test]
    fn test_decision_aggregation_one_block() {
        let cmd = vec![
            CommandHttpDecision {
                decision: "allow".into(),
                ..Default::default()
            },
            CommandHttpDecision {
                decision: "block".into(),
                reason: Some("dangerous".into()),
                ..Default::default()
            },
        ];
        let result = aggregate_decisions(&cmd, &[]);
        assert_eq!(result.decision, HookDecision::Block);
        assert_eq!(result.block_count, 1);
        assert_eq!(result.reasons, vec!["dangerous"]);
    }

    #[test]
    fn test_decision_aggregation_prompt_block() {
        let prompt = vec![PromptAgentDecision {
            ok: false,
            reason: Some("not safe".into()),
        }];
        let result = aggregate_decisions(&[], &prompt);
        assert_eq!(result.decision, HookDecision::Block);
        assert_eq!(result.block_count, 1);
        assert_eq!(result.reasons, vec!["not safe"]);
    }

    #[test]
    fn test_decision_aggregation_mixed_types() {
        let cmd = vec![CommandHttpDecision {
            decision: "allow".into(),
            ..Default::default()
        }];
        let prompt = vec![PromptAgentDecision {
            ok: false,
            reason: Some("LLM says no".into()),
        }];
        let result = aggregate_decisions(&cmd, &prompt);
        assert_eq!(result.decision, HookDecision::Block);
        assert_eq!(result.block_count, 1);
    }

    #[test]
    fn test_decision_aggregation_empty() {
        let result = aggregate_decisions(&[], &[]);
        assert_eq!(result.decision, HookDecision::Allow);
        assert_eq!(result.handler_count, 0);
        assert_eq!(result.block_count, 0);
    }

    // --- Match subject extraction ---

    #[test]
    fn test_extract_match_subject_tool_name() {
        let input = HookInput {
            session_id: None,
            hook_event: "pre_tool_execute".into(),
            timestamp: "2026-01-01T00:00:00Z".into(),
            extra: serde_json::json!({ "tool_name": "Bash" }),
        };
        assert_eq!(extract_match_subject(&input), "Bash");
    }

    #[test]
    fn test_extract_match_subject_fallback() {
        let input = HookInput {
            session_id: None,
            hook_event: "session_created".into(),
            timestamp: "2026-01-01T00:00:00Z".into(),
            extra: serde_json::json!({}),
        };
        assert_eq!(extract_match_subject(&input), "session_created");
    }

    // --- Hook point parsing ---

    #[test]
    fn test_parse_hook_point_valid() {
        assert_eq!(
            parse_hook_point("pre_tool_execute").unwrap(),
            HookPoint::PreToolExecute
        );
        assert_eq!(
            parse_hook_point("agent_loop_end").unwrap(),
            HookPoint::AgentLoopEnd
        );
    }

    #[test]
    fn test_parse_hook_point_invalid() {
        assert!(parse_hook_point("nonexistent_hook").is_err());
    }

    // --- Metrics ---

    #[test]
    fn test_metrics_snapshot() {
        let metrics = HookHandlerMetrics::default();
        metrics.invocations.fetch_add(5, Ordering::Relaxed);
        metrics.blocks.fetch_add(1, Ordering::Relaxed);

        let snap = metrics.snapshot();
        assert_eq!(snap.invocations, 5);
        assert_eq!(snap.blocks, 1);
        assert_eq!(snap.errors, 0);
    }

    // --- Command hook tests ---

    #[cfg(feature = "hook_handlers")]
    mod handler_tests {
        use super::*;
        use crate::config::HookHandlerGroupConfig;

        #[tokio::test]
        async fn test_command_hook_exit_0_allow() {
            let executor = CommandExecutor {
                allowed_hook_dirs: vec![],
            };
            let result = executor
                .execute("/bin/echo", b"{}", Duration::from_secs(5))
                .await
                .unwrap();
            // echo outputs "{}\n" which is valid JSON but as CommandHttpDecision
            // defaults to allow
            assert_eq!(result.decision, "allow");
        }

        #[tokio::test]
        async fn test_command_hook_exit_2_block() {
            let executor = CommandExecutor {
                allowed_hook_dirs: vec![],
            };
            // exit 2 → block
            let result = executor
                .execute(
                    "/bin/sh -c 'echo blocked >&2; exit 2'",
                    b"{}",
                    Duration::from_secs(5),
                )
                .await
                .unwrap();
            assert_eq!(result.decision, "block");
        }

        #[tokio::test]
        async fn test_command_hook_exit_1_error_continue() {
            let executor = CommandExecutor {
                allowed_hook_dirs: vec![],
            };
            let result = executor
                .execute(
                    "/bin/sh -c 'echo error >&2; exit 1'",
                    b"{}",
                    Duration::from_secs(5),
                )
                .await
                .unwrap();
            assert_eq!(result.decision, "allow");
        }

        #[tokio::test]
        async fn test_command_hook_timeout_killed() {
            let executor = CommandExecutor {
                allowed_hook_dirs: vec![],
            };
            let result = executor
                .execute("/bin/sleep 60", b"{}", Duration::from_millis(100))
                .await;
            assert!(matches!(result, Err(HookError::HookHandlerTimeout { .. })));
        }

        #[tokio::test]
        async fn test_command_hook_json_stdout() {
            let executor = CommandExecutor {
                allowed_hook_dirs: vec![],
            };
            let result = executor
                .execute(
                    r#"/bin/sh -c 'echo "{\"decision\":\"block\",\"reason\":\"test\"}"'"#,
                    b"{}",
                    Duration::from_secs(5),
                )
                .await;
            // exit 0 + JSON stdout with block decision
            // Note: exit 0 means allow, but the JSON decision field is parsed
            let r = result.unwrap();
            assert_eq!(r.decision, "block");
            assert_eq!(r.reason.as_deref(), Some("test"));
        }

        #[tokio::test]
        async fn test_command_hook_receives_json_stdin() {
            let executor = CommandExecutor {
                allowed_hook_dirs: vec![],
            };
            // Cat stdin back to stdout.
            let result = executor
                .execute(
                    "/bin/cat",
                    br#"{"hook_event":"test"}"#,
                    Duration::from_secs(5),
                )
                .await
                .unwrap();
            // cat outputs the JSON input unchanged — not a valid CommandHttpDecision,
            // so it defaults to allow.
            assert_eq!(result.decision, "allow");
        }

        #[tokio::test]
        async fn test_matcher_regex_match() {
            // Test via executor
            let config = HookConfig {
                hook_handlers: {
                    let mut m = HashMap::new();
                    m.insert(
                        "pre_tool_execute".into(),
                        vec![HookHandlerGroupConfig {
                            matcher: "Bash|ShellExec".into(),
                            timeout_ms: Some(5000),
                            handlers: vec![HandlerConfig::Command {
                                command: "/bin/true".into(),
                                r#async: false,
                            }],
                        }],
                    );
                    m
                },
                ..HookConfig::default()
            };

            let executor = HookHandlerExecutor::from_config(&config).unwrap();

            // Should match "Bash"
            let input_bash = HookInput {
                session_id: None,
                hook_event: "pre_tool_execute".into(),
                timestamp: "2026-01-01T00:00:00Z".into(),
                extra: serde_json::json!({ "tool_name": "Bash" }),
            };
            let result = executor
                .execute(HookPoint::PreToolExecute, &input_bash)
                .await;
            assert_eq!(result.handler_count, 1);

            // Should NOT match "Search"
            let input_search = HookInput {
                session_id: None,
                hook_event: "pre_tool_execute".into(),
                timestamp: "2026-01-01T00:00:00Z".into(),
                extra: serde_json::json!({ "tool_name": "Search" }),
            };
            let result = executor
                .execute(HookPoint::PreToolExecute, &input_search)
                .await;
            assert_eq!(result.handler_count, 0);
        }

        #[tokio::test]
        async fn test_matcher_star_matches_all() {
            let config = HookConfig {
                hook_handlers: {
                    let mut m = HashMap::new();
                    m.insert(
                        "pre_tool_execute".into(),
                        vec![HookHandlerGroupConfig {
                            matcher: "*".into(),
                            timeout_ms: Some(5000),
                            handlers: vec![HandlerConfig::Command {
                                command: "/bin/true".into(),
                                r#async: false,
                            }],
                        }],
                    );
                    m
                },
                ..HookConfig::default()
            };

            let executor = HookHandlerExecutor::from_config(&config).unwrap();
            let input = HookInput {
                session_id: None,
                hook_event: "pre_tool_execute".into(),
                timestamp: "2026-01-01T00:00:00Z".into(),
                extra: serde_json::json!({ "tool_name": "AnyTool" }),
            };
            let result = executor.execute(HookPoint::PreToolExecute, &input).await;
            assert_eq!(result.handler_count, 1);
        }

        #[tokio::test]
        async fn test_no_handlers_noop() {
            let config = HookConfig::default();
            let executor = HookHandlerExecutor::from_config(&config).unwrap();
            let input = HookInput {
                session_id: None,
                hook_event: "pre_tool_execute".into(),
                timestamp: "2026-01-01T00:00:00Z".into(),
                extra: serde_json::json!({}),
            };
            let result = executor.execute(HookPoint::PreToolExecute, &input).await;
            assert_eq!(result.decision, HookDecision::Allow);
            assert_eq!(result.handler_count, 0);
        }

        #[tokio::test]
        async fn test_async_handler_fires_forget() {
            let config = HookConfig {
                hook_handlers: {
                    let mut m = HashMap::new();
                    m.insert(
                        "post_tool_execute".into(),
                        vec![HookHandlerGroupConfig {
                            matcher: "*".into(),
                            timeout_ms: Some(5000),
                            handlers: vec![HandlerConfig::Command {
                                command: "/bin/sleep 60".into(),
                                r#async: true, // fire and forget
                            }],
                        }],
                    );
                    m
                },
                ..HookConfig::default()
            };

            let executor = HookHandlerExecutor::from_config(&config).unwrap();
            let input = HookInput {
                session_id: None,
                hook_event: "post_tool_execute".into(),
                timestamp: "2026-01-01T00:00:00Z".into(),
                extra: serde_json::json!({}),
            };

            // Should return immediately even though handler sleeps.
            let start = std::time::Instant::now();
            let result = executor.execute(HookPoint::PostToolExecute, &input).await;
            let elapsed = start.elapsed();

            // Async handler is spawned but not awaited, so handler_count is 1.
            assert_eq!(result.handler_count, 1);
            // Should complete quickly (< 1s), not wait for sleep 60.
            assert!(elapsed < Duration::from_secs(1));
            // Async handlers don't contribute to decisions.
            assert_eq!(result.decision, HookDecision::Allow);
        }

        // --- Validation tests ---

        #[test]
        fn test_prompt_on_unsupported_hook_point_rejected() {
            let config = HookConfig {
                hook_handlers: {
                    let mut m = HashMap::new();
                    m.insert(
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
                    m
                },
                ..HookConfig::default()
            };
            assert!(HookHandlerExecutor::from_config(&config).is_err());
        }

        #[test]
        fn test_agent_on_unsupported_hook_point_rejected() {
            let config = HookConfig {
                hook_handlers: {
                    let mut m = HashMap::new();
                    m.insert(
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
                    m
                },
                ..HookConfig::default()
            };
            assert!(HookHandlerExecutor::from_config(&config).is_err());
        }

        #[test]
        fn test_command_relative_path_rejected() {
            let config = HookConfig {
                hook_handlers: {
                    let mut m = HashMap::new();
                    m.insert(
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
                    m
                },
                ..HookConfig::default()
            };
            assert!(HookHandlerExecutor::from_config(&config).is_err());
        }

        #[test]
        fn test_command_outside_allowed_dirs_rejected() {
            let config = HookConfig {
                allowed_hook_dirs: vec!["/home/user/.y-agent/hooks".into()],
                hook_handlers: {
                    let mut m = HashMap::new();
                    m.insert(
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
                    m
                },
                ..HookConfig::default()
            };
            assert!(HookHandlerExecutor::from_config(&config).is_err());
        }
    }

    // --- extract_json_from_response tests ---
    #[cfg(feature = "llm_hooks")]
    mod json_extraction_tests {
        use super::super::extract_json_from_response;

        #[test]
        fn test_clean_json() {
            assert_eq!(
                extract_json_from_response(r#"{"ok": true, "reason": "looks good"}"#),
                r#"{"ok": true, "reason": "looks good"}"#
            );
        }

        #[test]
        fn test_json_with_markdown_fences() {
            let input = "```json\n{\"ok\": false, \"reason\": \"dangerous\"}\n```";
            assert_eq!(
                extract_json_from_response(input),
                r#"{"ok": false, "reason": "dangerous"}"#
            );
        }

        #[test]
        fn test_json_with_surrounding_text() {
            let input = "Here is my decision:\n{\"ok\": true}\nThat's it.";
            assert_eq!(extract_json_from_response(input), r#"{"ok": true}"#);
        }

        #[test]
        fn test_no_json_returns_trimmed() {
            assert_eq!(extract_json_from_response("  no json  "), "no json");
        }
    }

    // --- Mock runner tests ---
    #[cfg(feature = "llm_hooks")]
    mod llm_runner_tests {
        use super::super::*;
        use std::sync::Arc;
        use y_core::hook::{HookAgentRunner, HookLlmRunner};

        struct MockLlmRunner {
            response: String,
        }

        #[async_trait::async_trait]
        impl HookLlmRunner for MockLlmRunner {
            async fn evaluate(
                &self,
                _system_prompt: &str,
                _user_message: &str,
                _model: Option<&str>,
                _timeout: Duration,
            ) -> Result<String, String> {
                Ok(self.response.clone())
            }
        }

        struct MockLlmRunnerError {
            error: String,
        }

        #[async_trait::async_trait]
        impl HookLlmRunner for MockLlmRunnerError {
            async fn evaluate(
                &self,
                _system_prompt: &str,
                _user_message: &str,
                _model: Option<&str>,
                _timeout: Duration,
            ) -> Result<String, String> {
                Err(self.error.clone())
            }
        }

        struct MockAgentRunner {
            response: String,
        }

        #[async_trait::async_trait]
        impl HookAgentRunner for MockAgentRunner {
            async fn run_agent(
                &self,
                _task_prompt: &str,
                _model: Option<&str>,
                _max_turns: u32,
                _timeout: Duration,
            ) -> Result<String, String> {
                Ok(self.response.clone())
            }
        }

        fn test_input() -> HookInput {
            HookInput {
                session_id: Some("test-session".into()),
                hook_event: "pre_tool_execute".into(),
                timestamp: "2026-01-01T00:00:00Z".into(),
                extra: serde_json::json!({ "tool_name": "Bash" }),
            }
        }

        #[tokio::test]
        async fn test_prompt_hook_allow() {
            let runner = MockLlmRunner {
                response: r#"{"ok": true, "reason": "safe operation"}"#.into(),
            };
            let result = execute_prompt(
                Some(&runner),
                "Check if this is safe: $ARGUMENTS",
                None,
                &test_input(),
                Duration::from_secs(5),
            )
            .await
            .unwrap();

            assert!(result.ok);
            assert_eq!(result.reason.unwrap(), "safe operation");
        }

        #[tokio::test]
        async fn test_prompt_hook_block() {
            let runner = MockLlmRunner {
                response: r#"{"ok": false, "reason": "dangerous command"}"#.into(),
            };
            let result = execute_prompt(
                Some(&runner),
                "Check: $ARGUMENTS",
                None,
                &test_input(),
                Duration::from_secs(5),
            )
            .await
            .unwrap();

            assert!(!result.ok);
            assert_eq!(result.reason.unwrap(), "dangerous command");
        }

        #[tokio::test]
        async fn test_prompt_hook_no_runner() {
            let result = execute_prompt(
                None,
                "Check: $ARGUMENTS",
                None,
                &test_input(),
                Duration::from_secs(5),
            )
            .await
            .unwrap();

            assert!(result.ok);
            assert!(result.reason.is_none());
        }

        #[tokio::test]
        async fn test_prompt_hook_llm_error() {
            let runner = MockLlmRunnerError {
                error: "rate limited".into(),
            };
            let result = execute_prompt(
                Some(&runner),
                "Check: $ARGUMENTS",
                None,
                &test_input(),
                Duration::from_secs(5),
            )
            .await;

            assert!(result.is_err());
        }

        #[tokio::test]
        async fn test_prompt_hook_markdown_fenced_response() {
            let runner = MockLlmRunner {
                response: "```json\n{\"ok\": true, \"reason\": \"all good\"}\n```".into(),
            };
            let result = execute_prompt(
                Some(&runner),
                "Check: $ARGUMENTS",
                None,
                &test_input(),
                Duration::from_secs(5),
            )
            .await
            .unwrap();

            assert!(result.ok);
            assert_eq!(result.reason.unwrap(), "all good");
        }

        #[tokio::test]
        async fn test_agent_hook_allow() {
            let runner = MockAgentRunner {
                response: r#"{"ok": true, "reason": "tests pass"}"#.into(),
            };
            let result = execute_agent(
                Some(&runner),
                "Verify tests: $ARGUMENTS",
                None,
                &test_input(),
                Duration::from_secs(30),
            )
            .await
            .unwrap();

            assert!(result.ok);
            assert_eq!(result.reason.unwrap(), "tests pass");
        }

        #[tokio::test]
        async fn test_agent_hook_block() {
            let runner = MockAgentRunner {
                response: r#"{"ok": false, "reason": "tests failing"}"#.into(),
            };
            let result = execute_agent(
                Some(&runner),
                "Verify: $ARGUMENTS",
                None,
                &test_input(),
                Duration::from_secs(30),
            )
            .await
            .unwrap();

            assert!(!result.ok);
            assert_eq!(result.reason.unwrap(), "tests failing");
        }

        #[tokio::test]
        async fn test_agent_hook_no_runner() {
            let result = execute_agent(
                None,
                "Verify: $ARGUMENTS",
                None,
                &test_input(),
                Duration::from_secs(30),
            )
            .await
            .unwrap();

            assert!(result.ok);
            assert!(result.reason.is_none());
        }
    }
}
