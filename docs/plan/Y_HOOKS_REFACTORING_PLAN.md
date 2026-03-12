# Y-Hooks Module Refactoring Plan

**Module**: `crates/y-hooks` + `crates/y-core/src/hook.rs`
**Design Doc**: `docs/design/hooks-plugin-design.md` (v0.9)
**Created**: 2026-03-11
**Status**: Ready for Implementation
**Supersedes**: Phase R3 of `Y_HOOKS_REMEDIATION_PLAN.md` (plugin loading is now removed by design)

---

## 1. Objective

Refactor the `y-hooks` module to align with the v0.9 design:

1. **Remove** the `Plugin`/`PluginLoader`/`libloading` native plugin loading mechanism
2. **Add** four configuration-driven Hook Handler types: command, HTTP, prompt, and agent
3. **Clean up** all obsolete code, config, features, error variants, and re-exports

---

## 2. Code Inventory

### Files to Keep (No Changes)

| File | Lines | Notes |
|------|-------|-------|
| [chain.rs](file:///Users/gorgias/Projects/y-agent/crates/y-hooks/src/chain.rs) | 386 | Priority-sorted middleware chain; 10 unit tests |
| [chain_runner.rs](file:///Users/gorgias/Projects/y-agent/crates/y-hooks/src/chain_runner.rs) | 270 | Timeout-guarded execution; 4 unit tests |
| [event_bus.rs](file:///Users/gorgias/Projects/y-agent/crates/y-hooks/src/event_bus.rs) | 409 | Channel-per-subscriber; 8 unit tests |
| [hook_registry.rs](file:///Users/gorgias/Projects/y-agent/crates/y-hooks/src/hook_registry.rs) | 308 | Handler registration/dispatch; 5 unit tests |
| [hooks_bench.rs](file:///Users/gorgias/Projects/y-agent/crates/y-hooks/benches/hooks_bench.rs) | 131 | 3 benchmarks |

### Files to Modify

| File | Key Changes |
|------|-------------|
| [y-core/src/hook.rs](file:///Users/gorgias/Projects/y-agent/crates/y-core/src/hook.rs) | Delete lines 503-627 (`Plugin`, `PluginRegistrar`, ABI types) |
| [lib.rs](file:///Users/gorgias/Projects/y-agent/crates/y-hooks/src/lib.rs) | Remove `plugin` module + `PluginLoader` re-export; add `hook_handler` module |
| [error.rs](file:///Users/gorgias/Projects/y-agent/crates/y-hooks/src/error.rs) | Remove `PluginError`; add `HookHandlerError`, `HookHandlerTimeout`, `HookHandlerValidation` |
| [config.rs](file:///Users/gorgias/Projects/y-agent/crates/y-hooks/src/config.rs) | Add `HookHandlerGroupConfig`, `HandlerConfig`, `HookContextVerbosity`; extend `HookConfig` |
| [hook_system.rs](file:///Users/gorgias/Projects/y-agent/crates/y-hooks/src/hook_system.rs) | Add optional `HookHandlerExecutor` field; add `execute_hook_handlers()` method |
| [Cargo.toml](file:///Users/gorgias/Projects/y-agent/crates/y-hooks/Cargo.toml) | Remove `plugin_loading`/`libloading`; add `reqwest`, `regex`, `y-provider` |

### Files to Delete

| File | Reason |
|------|--------|
| [plugin.rs](file:///Users/gorgias/Projects/y-agent/crates/y-hooks/src/plugin.rs) (320 lines) | Entire `PluginLoader`/`PluginConfig`/`LoadedPlugin` — obsolete |

### Files to Create

| File | Purpose |
|------|---------|
| `y-hooks/src/hook_handler.rs` | Hook Handler Executor: 4 handler types, matcher, JSON I/O, decision aggregation |

---

## 3. Refactoring Phases

### Phase H1: Remove Obsolete Plugin Code

**Goal**: Clean removal of all plugin-related code. No new functionality — just subtraction.

**TDD approach**: Run existing tests after each deletion step to confirm nothing breaks.

#### H1.1 — Delete `y-hooks/src/plugin.rs`

Remove the entire file. Contains:
- `PluginConfig`, `LoadedPlugin`, `PluginLoader` structs
- `load_plugin()`, `unload_plugin()`, `is_loaded()`, `loaded_plugins()` methods
- 5 unit tests

#### H1.2 — Remove Plugin API from `y-core/src/hook.rs`

Delete lines 503-627 (Section "Plugin API"):

```diff
- // Plugin API
- pub trait Plugin: Send + Sync { ... }
- pub struct PluginRegistrar { ... }
- impl PluginRegistrar { ... }
- impl Default for PluginRegistrar { ... }
- pub const PLUGIN_ABI_VERSION: u32 = 1;
- pub type CreatePluginFn = unsafe extern "C" fn() -> *mut std::ffi::c_void;
- pub type PluginAbiVersionFn = unsafe extern "C" fn() -> u32;
```

Update module doc comment (lines 1-11): remove "Plugin" mentions.

#### H1.3 — Update `y-hooks/src/lib.rs`

Remove:
```diff
- #[cfg(feature = "plugin_loading")]
- pub mod plugin;
- #[cfg(feature = "plugin_loading")]
- pub use plugin::PluginLoader;
```

Update crate-level doc comment: `"Hook system, middleware chains, async event bus"` (remove "plugin loading").

#### H1.4 — Update `y-hooks/Cargo.toml`

```diff
- description = "Hook system, middleware chains, async event bus, plugin loading"
+ description = "Hook system, middleware chains, async event bus, hook handlers"

  [features]
- default = ["hooks_enabled", "event_bus", "middleware_chains", "plugin_loading"]
+ default = ["hooks_enabled", "event_bus", "middleware_chains"]
  hooks_enabled = []
  event_bus = []
  middleware_chains = []
- plugin_loading = ["dep:libloading"]

  [dependencies]
- libloading = { version = "0.8", optional = true }

  [dev-dependencies]
- toml = "0.8"
```

#### H1.5 — Update `y-hooks/src/error.rs`

```diff
- #[error("plugin error: {message}")]
- PluginError { message: String },
```

#### H1.6 — Verify clean compilation

```bash
cargo check --workspace
cargo test -p y-hooks
cargo test -p y-core
cargo clippy -p y-hooks -p y-core -- -W warnings
```

---

### Phase H2: Add Hook Handler Configuration Types

**Goal**: Define the complete data model for all 4 handler types in config.

#### H2.1 — Extend `y-hooks/src/config.rs`

**Handler group config** (per design §Hook Handler Types → Handler Configuration Model):

```rust
use std::collections::HashMap;

/// Configuration for a handler group bound to a hook point.
/// Each group has an optional matcher and one or more handlers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookHandlerGroupConfig {
    /// Regex pattern to filter when handlers fire.
    /// Omit or "*" for all events at this hook point.
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

fn default_matcher() -> String { "*".to_string() }
```

**Handler config enum** (per design §Common Handler Fields + per-type fields):

```rust
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
        /// HTTP headers. Values support $ENV_VAR substitution.
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
```

**Extend `HookConfig`** (per design §Configuration):

```rust
/// Controls what data is serialized to hook handlers.
/// Per design §Security: "Hook handlers receive summaries by default, not raw content."
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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

pub struct HookConfig {
    // --- Existing fields (unchanged) ---
    pub middleware_timeout_ms: u64,
    pub event_channel_capacity: usize,
    pub max_subscribers: usize,

    // --- New fields ---
    /// External hook handler groups, keyed by hook point name (snake_case).
    /// Example key: "pre_tool_execute"
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
```

#### H2.2 — Add new error variants to `y-hooks/src/error.rs`

```rust
#[error("hook handler error ({handler_type}): {message}")]
HookHandlerError { handler_type: String, message: String },

#[error("hook handler timeout ({handler_type}): exceeded {timeout_ms}ms")]
HookHandlerTimeout { handler_type: String, timeout_ms: u64 },

#[error("hook handler validation error: {message}")]
HookHandlerValidation { message: String },
```

#### H2.3 — Config validation function

```rust
/// Validate handler config at load time.
/// - Prompt/agent handlers only on DECISION_CAPABLE_HOOK_POINTS.
/// - Command scripts must be absolute paths.
/// - If allowed_hook_dirs is non-empty, command scripts must be within allowed dirs.
pub fn validate_hook_handler_config(config: &HookConfig) -> Result<(), HookError>
```

**Decision-capable hook points** (per design §Prompt Hook Fields → Supported hook points):

```rust
/// Hook points that support prompt and agent handlers.
const DECISION_CAPABLE_HOOK_POINTS: &[&str] = &[
    "pre_tool_execute",
    "post_tool_execute",
    "pre_llm_call",
    "agent_loop_start",
    "agent_loop_end",
    "pre_compaction",
];
```

#### H2.4 — Unit tests for config parsing

| Test | Scenario |
|------|----------|
| `test_handler_config_command` | Parse command handler TOML |
| `test_handler_config_http` | Parse HTTP handler TOML with headers |
| `test_handler_config_prompt` | Parse prompt handler TOML with model override |
| `test_handler_config_agent` | Parse agent handler TOML |
| `test_handler_config_with_matcher` | Parse handler group with regex matcher |
| `test_handler_config_empty` | No handlers configured → empty map |
| `test_handler_config_default_timeouts` | Verify defaults: 5s command/HTTP, 30s prompt, 120s agent |
| `test_validate_prompt_on_unsupported_point` | Prompt handler on `session_created` → validation error |
| `test_validate_agent_on_unsupported_point` | Agent handler on `memory_stored` → validation error |
| `test_validate_command_script_not_absolute` | Relative path → validation error |
| `test_validate_allowed_hook_dirs` | Script outside allowed dirs → validation error |

---

### Phase H3: Implement Hook Handler Executor

**Goal**: Core runtime for executing all 4 handler types.

#### H3.1 — Create `y-hooks/src/hook_handler.rs`

**Core types**:

```rust
use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
use std::time::Duration;

use regex::Regex;
use y_core::hook::HookPoint;

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
```

**Decision types per handler kind**:

```rust
/// Decision returned by a command/HTTP hook handler (JSON stdout or response body).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CommandHttpDecision {
    #[serde(default = "default_allow")]
    pub decision: String,       // "allow" | "block"
    pub reason: Option<String>,
    pub context_message: Option<String>,
    #[serde(default)]
    pub suppress_output: bool,
}

/// Decision returned by a prompt/agent hook handler.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptAgentDecision {
    pub ok: bool,               // true = allow, false = block
    pub reason: Option<String>,
}
```

**Hook input** (per design §Hook Input Format):

```rust
/// Common hook input fields serialized as JSON.
#[derive(Debug, Serialize)]
pub struct HookInput {
    pub session_id: Option<String>,
    pub hook_event: String,         // hook point name (e.g., "pre_tool_execute")
    pub timestamp: String,          // ISO 8601
    #[serde(flatten)]
    pub extra: serde_json::Value,   // hook-specific fields from HookPoints table
}
```

**Internal compiled representation**:

```rust
/// A handler group with compiled matcher regex.
struct CompiledHandlerGroup {
    matcher: Option<Regex>,         // None = match all ("*")
    timeout: Duration,
    handlers: Vec<HandlerConfig>,
}
```

**Executor struct**:

```rust
pub struct HookHandlerExecutor {
    /// Compiled handlers per hook point.
    handlers: HashMap<HookPoint, Vec<CompiledHandlerGroup>>,
    /// Shared HTTP client with connection pooling.
    /// Per design §Optimization: "shared reqwest::Client with connection pooling"
    http_client: reqwest::Client,
    /// Allowed hook script directories.
    allowed_hook_dirs: Vec<String>,
    /// Context verbosity level.
    verbosity: HookContextVerbosity,
    /// Metrics counters.
    metrics: HookHandlerMetrics,
}
```

#### H3.2 — Executor methods

**Constructor**:

```rust
/// Parse config, compile regex matchers, validate prompt/agent hook points.
/// Returns Err if config validation fails.
pub fn from_config(config: &HookConfig) -> Result<Self, HookError>
```

Steps:
1. Call `validate_hook_handler_config(config)` (from H2.3).
2. Iterate `config.hook_handlers`: parse hook point name → `HookPoint` enum.
3. Compile `matcher` regex for each group (or `None` for `"*"`).
4. Apply default timeouts per handler type when `timeout_ms` is `None`.
5. Build `reqwest::Client` with default pooling config.

**Main dispatch**:

```rust
/// Execute all matching handlers for a hook point.
/// Returns aggregate decision.
/// Per design §Failure Handling: if any sync handler returns "block", aggregate is "block".
pub async fn execute(
    &self,
    hook_point: HookPoint,
    input: &HookInput,
) -> HookHandlerResult
```

Steps:
1. Look up `hook_point` in `self.handlers`. If not found → return default (allow).
2. Serialize `input` to JSON bytes.
3. Determine match subject from input (e.g., `tool_name` for tool hooks, per Matcher Patterns).
4. For each `CompiledHandlerGroup`, check `matcher.is_match(subject)`.
5. For each matching group, iterate handlers:
   - If `async = true` → `tokio::spawn` handler execution, skip result.
   - If `async = false` → await handler, collect decision.
6. Aggregate: if any sync handler returned "block" or `ok: false` → aggregate = Block.
7. Collect all reasons and context_messages.
8. Update metrics.

#### H3.3 — Command hook execution

Per design §Command Hook Fields:

```rust
async fn execute_command(
    &self,
    cmd: &str,
    input_json: &[u8],
    timeout: Duration,
) -> Result<CommandHttpDecision, HookError>
```

Steps:
1. Validate script path: absolute path, within `allowed_hook_dirs` (if configured).
2. Spawn `tokio::process::Command::new("sh").arg("-c").arg(cmd)`.
3. Write `input_json` to stdin, close stdin.
4. `tokio::time::timeout(timeout, child.wait_with_output())`.
5. On timeout → kill process, return `HookHandlerTimeout`.
6. Parse exit code:
   - `0` → allow. If stdout is valid JSON, parse as `CommandHttpDecision`.
   - `1` → non-blocking error. Log stderr. Return allow.
   - `2` → block. Parse reason from JSON stdout or stderr.
   - Other → non-blocking error. Log. Return allow.

#### H3.4 — HTTP hook execution

Per design §HTTP Hook Fields:

```rust
async fn execute_http(
    &self,
    url: &str,
    headers: &HashMap<String, String>,
    input_json: &[u8],
    timeout: Duration,
) -> Result<CommandHttpDecision, HookError>
```

Steps:
1. Build request: `self.http_client.post(url)`.
2. Expand headers: replace `$ENV_VAR` / `${ENV_VAR}` patterns with `std::env::var()`.
3. Set `Content-Type: application/json`.
4. Send with `reqwest::Client::timeout(timeout)`.
5. Parse response:
   - 2xx + JSON body → parse as `CommandHttpDecision`.
   - 2xx + empty body → default allow.
   - Non-2xx → log status + body, return allow.
   - Connection error / timeout → log, return allow.

**Env var substitution utility**:

```rust
/// Replace $VAR_NAME and ${VAR_NAME} patterns in a string with env var values.
fn expand_env_vars(s: &str) -> String
```

#### H3.5 — Prompt hook execution

Per design §Prompt Hook Fields:

```rust
#[cfg(feature = "llm_hooks")]
async fn execute_prompt(
    &self,
    prompt_template: &str,
    model: Option<&str>,
    input: &HookInput,
    timeout: Duration,
) -> Result<PromptAgentDecision, HookError>
```

Steps:
1. Serialize `input` to JSON string.
2. Replace `$ARGUMENTS` in `prompt_template` with the JSON string.
3. Send single-turn completion request via `y-provider::ProviderPool`:
   - Select model: `model` override, or fastest available ("haiku"-class).
   - System prompt: "Respond with JSON: `{\"ok\": true/false, \"reason\": \"...\"}`. No other output."
   - User message: the expanded prompt.
4. Parse response as `PromptAgentDecision`.
5. On LLM error → log, return `{ok: true}` (default allow).
6. On invalid JSON → log, return `{ok: true}` (default allow).

#### H3.6 — Agent hook execution

Per design §Agent Hook Fields:

```rust
#[cfg(feature = "llm_hooks")]
async fn execute_agent(
    &self,
    prompt_template: &str,
    model: Option<&str>,
    input: &HookInput,
    timeout: Duration,
) -> Result<PromptAgentDecision, HookError>
```

Steps:
1. Serialize `input` to JSON string.
2. Replace `$ARGUMENTS` in `prompt_template` with the JSON string.
3. Spawn a subagent with:
   - Restricted tool set: `Read`, `Grep`, `Glob` only (read-only sandbox).
   - Max turns: 50.
   - Model: `model` override or fastest available.
   - System prompt: "You are a verification agent. Investigate and return JSON: `{\"ok\": true/false, \"reason\": \"...\"}`. You have read-only access to the workspace."
4. Run agent loop with `tokio::time::timeout(timeout, ...)`.
5. Parse final response as `PromptAgentDecision`.
6. On max turns exceeded → terminate, return `{ok: true}` (default allow).
7. On timeout → terminate, return `{ok: true}` (default allow).

> [!NOTE]
> The agent hook implementation depends on `y-agent` crate for the agent loop. To avoid circular dependencies, the agent hook executor should accept a trait `AgentHookRunner` that is implemented in `y-agent` and injected at startup. This keeps `y-hooks` independent.

#### H3.7 — Decision aggregation

```rust
/// Aggregate decisions from all synchronous handlers.
fn aggregate_decisions(
    cmd_http_decisions: &[CommandHttpDecision],
    prompt_agent_decisions: &[PromptAgentDecision],
) -> HookHandlerResult
```

Logic (per design §Failure Handling → "Multiple handlers disagree"):
- If **any** `CommandHttpDecision.decision == "block"` → aggregate = Block.
- If **any** `PromptAgentDecision.ok == false` → aggregate = Block.
- Collect all `reason` and `context_message` values.
- If all allow → aggregate = Allow.

#### H3.8 — Metrics

Per design §Observability → Hook handlers:

```rust
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

impl HookHandlerMetrics {
    pub fn snapshot(&self) -> HookHandlerMetricsSnapshot { ... }
}
```

Each metric should be tagged by handler type (`command`/`http`/`prompt`/`agent`) and hook point.

---

### Phase H4: Integrate into HookSystem

**Goal**: Wire executor into the existing `HookSystem` facade.

#### H4.1 — Update `y-hooks/src/hook_system.rs`

Add executor as optional field:

```rust
pub struct HookSystem {
    // ... existing 5 chains, hooks, events, runner ...

    /// External hook handler executor (command/HTTP/prompt/agent).
    /// None if no handlers are configured or handlers_enabled = false.
    handler_executor: Option<HookHandlerExecutor>,
}
```

Update constructor:

```rust
impl HookSystem {
    pub fn new(config: &HookConfig) -> Self {
        let handler_executor = if config.handlers_enabled
            && !config.hook_handlers.is_empty()
        {
            match HookHandlerExecutor::from_config(config) {
                Ok(executor) => Some(executor),
                Err(e) => {
                    tracing::error!(error = %e, "failed to initialize hook handlers");
                    None
                }
            }
        } else {
            None
        };

        Self {
            // ... existing fields ...
            handler_executor,
        }
    }
}
```

Add dispatch method:

```rust
/// Execute external hook handlers for a hook point.
/// Returns the aggregate decision. Noop if no executor or no handlers for this point.
pub async fn execute_hook_handlers(
    &self,
    hook_point: HookPoint,
    input: &HookInput,
) -> HookHandlerResult {
    match &self.handler_executor {
        Some(executor) => executor.execute(hook_point, input).await,
        None => HookHandlerResult::default(),  // allow
    }
}

/// Get the handler executor (for diagnostics/metrics).
pub fn handler_executor(&self) -> Option<&HookHandlerExecutor> {
    self.handler_executor.as_ref()
}
```

#### H4.2 — Update `y-hooks/src/lib.rs`

```rust
pub mod hook_handler;

pub use hook_handler::{
    HookHandlerExecutor, HookHandlerResult, HookDecision,
    HookInput, CommandHttpDecision, PromptAgentDecision,
    HookHandlerMetrics,
};
```

#### H4.3 — Update Debug impl

Update `HookSystem`'s `Debug` impl to include handler executor status:

```rust
.field("handler_executor", &self.handler_executor.is_some())
```

---

### Phase H5: Tests and Verification

#### H5.1 — Unit tests in `hook_handler.rs`

**Command hooks**:

| Test | Scenario |
|------|----------|
| `test_command_hook_exit_0_allow` | Script exits 0 → decision = allow |
| `test_command_hook_exit_2_block` | Script exits 2 → decision = block |
| `test_command_hook_exit_1_error_continue` | Script exits 1 → logged, operation continues |
| `test_command_hook_timeout_killed` | Script sleeps → killed, operation continues |
| `test_command_hook_json_stdout` | Script writes JSON to stdout → parsed as `CommandHttpDecision` |
| `test_command_hook_receives_json_stdin` | Verify hook receives correct JSON on stdin |

**HTTP hooks**:

| Test | Scenario |
|------|----------|
| `test_http_hook_200_allow` | Mock returns 200 empty → allow |
| `test_http_hook_200_block_json` | Mock returns 200 with `{"decision":"block"}` → block |
| `test_http_hook_500_continue` | Mock returns 500 → logged, continues |
| `test_http_hook_timeout_continue` | Mock slow → timeout, continues |
| `test_http_hook_env_var_headers` | `$MY_TOKEN` in header → expanded from env |

**Prompt hooks**:

| Test | Scenario |
|------|----------|
| `test_prompt_hook_ok_true_allow` | Mock LLM returns `{ok: true}` → allow |
| `test_prompt_hook_ok_false_block` | Mock LLM returns `{ok: false, reason: "..."}` → block with reason |
| `test_prompt_hook_llm_error_default_allow` | LLM call fails → logged, default allow |
| `test_prompt_hook_invalid_json_default_allow` | LLM returns non-JSON → logged, default allow |
| `test_prompt_hook_arguments_substitution` | `$ARGUMENTS` replaced with hook input JSON |

**Agent hooks**:

| Test | Scenario |
|------|----------|
| `test_agent_hook_ok_true_allow` | Mock subagent returns `{ok: true}` → allow |
| `test_agent_hook_ok_false_block` | Mock subagent returns `{ok: false}` → block |
| `test_agent_hook_max_turns_default_allow` | Subagent exceeds 50 turns → terminated, default allow |

**Matching and aggregation**:

| Test | Scenario |
|------|----------|
| `test_matcher_regex_match` | `"Bash\|ShellExec"` matches "Bash" but not "Search" |
| `test_matcher_star_matches_all` | `"*"` matches any tool name |
| `test_decision_aggregation_all_allow` | Two handlers both allow → allow |
| `test_decision_aggregation_one_block` | One allow + one block → aggregate block |
| `test_decision_aggregation_mixed_types` | Command allow + prompt ok:false → aggregate block |
| `test_async_handler_fires_forget` | Async handler does not block; result not awaited |
| `test_no_handlers_noop` | Empty config → noop, allow |

**Validation**:

| Test | Scenario |
|------|----------|
| `test_prompt_on_unsupported_hook_point_rejected` | Prompt on `session_created` → validation error |
| `test_agent_on_unsupported_hook_point_rejected` | Agent on `memory_stored` → validation error |
| `test_command_relative_path_rejected` | `./script.sh` → validation error |
| `test_command_outside_allowed_dirs_rejected` | Script outside `allowed_hook_dirs` → rejected |

#### H5.2 — Integration test: `tests/hook_handler_integration_test.rs`

| Test | Scenario |
|------|----------|
| `test_hook_system_command_handler_e2e` | Configure command hook in `HookConfig` → create `HookSystem` → trigger hook point → verify script executed + decision returned |
| `test_hook_system_no_executor_when_disabled` | `handlers_enabled = false` → `handler_executor` is `None` |
| `test_hook_system_handler_metrics` | Execute handlers → verify metrics (invocations, blocks, errors) |

#### H5.3 — Verification commands

```bash
# Full workspace compilation
cargo check --workspace

# y-hooks tests (all phases)
cargo test -p y-hooks

# y-core tests (plugin removal)
cargo test -p y-core

# Clippy
cargo clippy -p y-hooks -p y-core -- -W warnings

# Benchmarks still pass
cargo bench -p y-hooks

# Feature flags compile independently
cargo check -p y-hooks --no-default-features --features hooks_enabled
cargo check -p y-hooks --no-default-features --features hook_handlers
```

---

## 4. Dependency Changes

### `y-hooks/Cargo.toml` — Dependencies

| Action | Dependency | Reason |
|--------|-----------|--------|
| **Remove** | `libloading = { version = "0.8", optional = true }` | Plugin loading removed |
| **Add** | `reqwest = { version = "0.12", features = ["json"], optional = true }` | HTTP hook handlers |
| **Add** | `regex = { version = "1", optional = true }` | Matcher pattern compilation |
| **Add** | `y-provider = { workspace = true, optional = true }` | Prompt/agent hooks need LLM access via `ProviderPool` |
| **Existing** | `tokio` | `tokio::process::Command` for command hooks (already present) |
| **Existing** | `serde_json` | JSON serialization for hook I/O (already present) |

### `y-hooks/Cargo.toml` — Feature Flags

```toml
[features]
default = ["hooks_enabled", "event_bus", "middleware_chains", "hook_handlers"]
hooks_enabled = []
event_bus = []
middleware_chains = []
hook_handlers = ["dep:reqwest", "dep:regex"]
llm_hooks = ["dep:y-provider", "hook_handlers"]
```

| Flag | Gates | Description |
|------|-------|-------------|
| `hook_handlers` | `reqwest`, `regex`, `HookHandlerExecutor`, command/HTTP handler execution | Core command + HTTP hook support |
| `llm_hooks` | `y-provider`, prompt/agent handler execution | LLM-based hook handlers (prompt + agent) |

---

## 5. Priority and Ordering

| Phase | Priority | Estimated Effort | Dependencies | Deliverable |
|-------|----------|-----------------|--------------|-------------|
| **H1** Remove Plugin Code | **High** | 1 hour | None | Clean compile, all existing tests pass |
| **H2** Handler Config Types | **High** | 2-3 hours | H1 | Config structs, validation, 11 unit tests |
| **H3** Handler Executor | **High** | 5-7 hours | H2 | `hook_handler.rs`: 4 handler types, aggregation, metrics |
| **H4** HookSystem Integration | **Medium** | 1-2 hours | H3 | Wired into facade, `execute_hook_handlers()` method |
| **H5** Tests and Verification | **High** | 3-4 hours | H3, H4 | 30+ unit tests, 3 integration tests, clean CI |

**Total estimated effort**: 1.5-2 days

**Recommended execution order**: H1 → H2 (with unit tests) → H3 (command/HTTP first, then prompt/agent) → H4 → H5 (integration tests)

---

## 6. Design Compliance Checklist

| Design Section | Plan Coverage |
|----------------|---------------|
| §Hook Handler Types (4 types) | H2 config, H3 executor |
| §Common Handler Fields (`type`, `timeout_ms`, `async`) | H2.1 `HandlerConfig` |
| §Command Hook Fields + exit codes | H3.3 |
| §HTTP Hook Fields + `$ENV_VAR` substitution | H3.4 |
| §Prompt Hook Fields + `$ARGUMENTS` + `{ok, reason}` | H3.5 |
| §Agent Hook Fields + read-only tools + 50 max turns | H3.6 |
| §Hook Input Format (common fields) | H3.1 `HookInput` |
| §Matcher Patterns (regex, tool_name matching) | H3.2 dispatch logic |
| §Failure Handling (all 16 scenarios) | H3 per-handler error handling |
| §Security (allowed_hook_dirs, env var substitution, verbosity) | H2.1 config, H3.3/H3.4 |
| §Performance (connection pooling, inline fast path, async handlers) | H3.1 executor struct, H3.2 dispatch |
| §Observability (5 metrics) | H3.8 metrics |
| §Rollout Phase 3 (hook handlers) | H3+H4 combined |
| §Rollback (feature flags, config disable) | H4.1 `handlers_enabled` + §4 feature flags |

---

## 7. Impact on Y_HOOKS_REMEDIATION_PLAN.md

The existing remediation plan's **Phase R3** (Plugin System) is now **entirely obsolete**:

| Old Phase R3 Item | New Status |
|-------------------|-----------|
| R3.1 — Add `Plugin` trait | **Removed** (delete existing trait in H1.2) |
| R3.2 — Add `PluginRegistrar` struct | **Removed** (delete existing struct in H1.2) |
| R3.3 — Implement `PluginLoader` with `libloading` | **Removed** (delete existing impl in H1.1) |
| R3.4 — Plugin configuration | **Replaced** by H2 (hook handler configuration) |

All other remediation phases (R1, R2, R4, R5) remain valid and are unaffected.
