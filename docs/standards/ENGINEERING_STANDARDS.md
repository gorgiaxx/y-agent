# y-agent Engineering Standards

**Version**: v0.1
**Created**: 2026-03-08
**Status**: Draft

---

## 1. Purpose

This document defines the engineering conventions, coding standards, and quality expectations for y-agent. All contributors (human and AI) must follow these standards. The goal is consistency, safety, and maintainability across 18+ Rust crates.

---

## 2. Rust Conventions

### 2.1 Naming

| Element | Convention | Example |
|---------|-----------|---------|
| Crate | `y-{name}` (kebab-case) | `y-core`, `y-provider` |
| Module / file | `snake_case` | `session_tree.rs` |
| Type / Trait / Enum | `PascalCase` | `SessionNode`, `RuntimeAdapter` |
| Function / method | `snake_case` | `execute_tool()` |
| Constant | `SCREAMING_SNAKE_CASE` | `MAX_RETRY_COUNT` |
| Feature flag | `snake_case` | `runtime_docker`, `memory_ltm` |
| Config key (TOML) | `snake_case` | `max_concurrency` |

### 2.2 Formatting and Linting

All code is formatted and linted before commit. CI enforces both.

**rustfmt.toml** (workspace root):

```toml
edition = "2021"
max_width = 100
tab_spaces = 4
use_field_init_shorthand = true
use_try_shorthand = true
# Note: imports_granularity and group_imports require nightly.
# Enable when nightly is adopted or these stabilize.
```

**clippy configuration** (workspace root `.clippy.toml` and `Cargo.toml`):

```toml
# Cargo.toml [workspace.lints.clippy]
pedantic = { level = "warn", priority = -1 }
# Allow patterns that are idiomatic in async Rust:
module_name_repetitions = "allow"
must_use_candidate = "allow"
missing_errors_doc = "allow"
# Deny unsafe without documented justification:
undocumented_unsafe_blocks = "deny"
```

### 2.3 Minimum Supported Rust Version (MSRV)

- MSRV: **stable** (latest stable release at time of project initialization)
- Tracked in `rust-toolchain.toml` at workspace root
- CI tests against MSRV

### 2.4 Unsafe Code Policy

- `unsafe` is prohibited by default
- Exceptions require:
  1. A `// SAFETY:` comment explaining the invariant
  2. Approval from the project owner
  3. Encapsulation in a safe wrapper function
  4. A dedicated test proving the invariant holds

---

## 3. Error Handling

### 3.1 Strategy Overview

y-agent uses a layered error handling strategy:

| Layer | Crate | Approach |
|-------|-------|----------|
| Core trait errors | `y-core` | Enum-based with `thiserror`, defines shared error types |
| Crate-internal errors | Each crate | `thiserror` enums, specific to subsystem |
| Application boundary | `y-cli`, `y-agent` | `anyhow` for top-level error reporting with context chains |

### 3.2 Error Enum Design

Each crate defines its own error enum in `src/error.rs`:

```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("rate limited by {provider}: retry after {retry_after_secs}s")]
    RateLimited {
        provider: String,
        retry_after_secs: u64,
    },

    #[error("authentication failed for {provider}")]
    AuthenticationFailed { provider: String },

    #[error("network error: {source}")]
    Network {
        #[from]
        source: reqwest::Error,
    },

    // ...
}
```

Rules:
- Each variant carries structured context (not just a string)
- Use `#[from]` for direct conversion from upstream errors
- Use `.context()` (via `anyhow`) at application boundaries to add call-site info
- Never use `unwrap()` or `expect()` in library code; use `?` propagation
- `unwrap()` is permitted only in tests and in startup code with clear invariant comments

### 3.3 Error Classification

Errors that cross crate boundaries carry classification metadata:

```rust
pub trait ClassifiedError {
    fn is_retryable(&self) -> bool;
    fn error_code(&self) -> &str;
    fn severity(&self) -> ErrorSeverity;
}

pub enum ErrorSeverity {
    /// Transient, safe to retry
    Transient,
    /// Permanent, do not retry
    Permanent,
    /// User action required (e.g., invalid config, missing API key)
    UserActionRequired,
}
```

This classification drives:
- Provider freeze duration selection
- Orchestrator retry/compensation decisions
- Guardrail escalation triggers

### 3.4 Error Redaction

Errors that may contain sensitive data (API keys, credentials, user content) must implement redaction before logging:

```rust
pub trait Redactable {
    fn redacted(&self) -> String;
}
```

Patterns to redact: API keys, email addresses, file paths containing usernames, bearer tokens. Redaction applies at the logging boundary, not at error construction.

---

## 4. Async Patterns

### 4.1 Runtime

- **Tokio** is the sole async runtime
- Multi-threaded runtime with work-stealing scheduler (default)
- Single configuration point in `y-cli/src/main.rs`

```rust
#[tokio::main]
async fn main() -> anyhow::Result<()> { ... }
```

No other crate starts a runtime. Library crates are runtime-agnostic (they use `async fn` but never call `tokio::runtime::Runtime::new()`).

### 4.2 Task Spawning

| Pattern | When to Use | Example |
|---------|------------|---------|
| `tokio::spawn` | Fire-and-forget background work | Event bus dispatch, diagnostic flush |
| `JoinSet` | Fan-out with structured collection | Parallel tool execution, multi-provider fallback |
| `tokio::select!` | First-completes-wins | Timeout wrappers, interrupt handling |
| Direct `.await` | Sequential dependency | Most request processing |

Rules:
- Always capture `JoinHandle` for spawned tasks unless truly fire-and-forget
- Fire-and-forget tasks must handle their own errors (log and continue)
- Never spawn in trait implementations without documenting it in the trait contract

### 4.3 Cancellation Safety

- All public async functions must be cancellation-safe or documented as not
- Use `tokio::pin!` for streams that must not be dropped mid-iteration
- Checkpoint writes use `committed/pending` separation specifically for cancellation safety
- When in doubt, use `tokio::select! { biased; ... }` to control evaluation order

### 4.4 Channel Usage

| Channel Type | When to Use | Backpressure |
|-------------|------------|--------------|
| `mpsc` (bounded) | Work queues, middleware chains | Yes (bounded capacity) |
| `broadcast` | Event bus (multi-subscriber) | Drop oldest on slow subscriber |
| `watch` | Configuration reload, state observation | Latest-value-wins |
| `oneshot` | Single-response request/reply | N/A |

Default capacities:
- Event bus subscribers: 1024
- Work queues: 256
- Middleware chains: 64

### 4.5 Timeouts

Every external call (LLM, Docker, network, DB) must have an explicit timeout:

```rust
tokio::time::timeout(Duration::from_secs(30), provider.chat_completion(req)).await??;
```

Default timeouts:
- LLM provider call: 120s (streaming), 60s (non-streaming)
- Tool execution: 300s (Docker), 60s (Native)
- Database query: 5s
- Middleware execution: 5s per middleware
- Health check: 10s

---

## 5. Configuration

### 5.1 Format

All user-facing configuration uses TOML. Internal serialization uses JSON (serde_json).

### 5.2 Config Struct Pattern

Every configurable subsystem follows this pattern:

```rust
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderConfig {
    pub id: String,
    pub provider_type: ProviderType,
    pub model: String,

    #[serde(default = "default_max_concurrency")]
    pub max_concurrency: usize,

    #[serde(default)]
    pub tags: Vec<String>,
}

fn default_max_concurrency() -> usize { 5 }

impl ProviderConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.id.is_empty() {
            return Err(ConfigError::MissingField("id"));
        }
        // ...
        Ok(())
    }
}
```

Rules:
- `deny_unknown_fields` catches typos in config files
- All optional fields have documented defaults via `#[serde(default)]`
- Validation is a separate `validate()` method, called after deserialization
- Config structs are `Clone + Debug`

### 5.3 Configuration Hierarchy

Override priority (highest wins):
1. CLI arguments
2. Environment variables (prefix: `Y_AGENT_`)
3. User config file (`~/.config/y-agent/config.toml`)
4. Project config file (`./y-agent.toml`)
5. Compiled defaults

### 5.4 Secrets

- Secrets (API keys, tokens) are **never** stored in config files
- Secrets come from environment variables only
- Config files reference secrets by env var name: `api_key_env = "OPENAI_API_KEY"`
- Secrets are redacted in logs and error messages (see Section 3.4)

---

## 6. Logging and Tracing

### 6.1 Library

`tracing` crate is the sole instrumentation library. Do not use `log` or `println!` (except in test fixtures).

### 6.2 Log Levels

| Level | When to Use | Example |
|-------|------------|---------|
| `ERROR` | Unrecoverable failure requiring attention | Provider auth failure, DB connection lost |
| `WARN` | Recoverable anomaly, degraded behavior | Provider frozen, tool timeout (retrying), config override |
| `INFO` | Significant lifecycle events | Session created, workflow started/completed, provider thawed |
| `DEBUG` | Detailed operational info for troubleshooting | Middleware chain execution, tool parameter resolution, checkpoint write |
| `TRACE` | High-volume fine-grained data | Raw LLM request/response, channel state mutations, individual token counts |

### 6.3 Span Naming

Format: `{crate}::{module}::{operation}`

```rust
#[tracing::instrument(skip(self, request))]
async fn chat_completion(&self, request: ChatRequest) -> Result<ChatResponse, ProviderError> {
    // Span name auto-derived: "provider::openai::chat_completion"
    tracing::info!(provider = %self.id, model = %request.model, "sending LLM request");
    // ...
}
```

### 6.4 Structured Fields

Every span and event should carry relevant context as structured fields, not string interpolation:

```rust
// Correct:
tracing::info!(session_id = %id, message_count = count, "session created");

// Incorrect:
tracing::info!("session {} created with {} messages", id, count);
```

Mandatory fields by context:
- LLM calls: `provider`, `model`, `input_tokens`, `output_tokens`, `duration_ms`
- Tool calls: `tool_name`, `tool_type`, `duration_ms`, `exit_code`
- Session ops: `session_id`, `session_type`
- Middleware: `middleware_name`, `chain_type`, `priority`

### 6.5 Cost-Conscious Logging

- `TRACE`-level logging must be gated behind `tracing::enabled!(Level::TRACE)` if constructing log data is expensive
- Raw LLM payloads logged only at TRACE level
- Production default level: INFO

---

## 7. Dependency Management

### 7.1 Workspace Dependencies

All shared dependencies declared at workspace level in root `Cargo.toml`:

```toml
[workspace.dependencies]
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
anyhow = "1"
tracing = "0.1"
async-trait = "0.1"
uuid = { version = "1", features = ["v4", "serde"] }
```

Crate-local `Cargo.toml` references workspace deps:

```toml
[dependencies]
tokio = { workspace = true }
serde = { workspace = true }
```

### 7.2 Adding New Dependencies

Before adding a dependency, verify:

1. **Necessity**: Can this be done with std or an existing dep?
2. **Maintenance**: Is it actively maintained? (Last commit < 6 months, issues triaged)
3. **License**: Must be MIT, Apache-2.0, or BSD-compatible
4. **Size**: Does it pull in a large transitive dependency tree?
5. **Security**: Run `cargo audit` after adding

### 7.3 Feature Flags

Subsystem-level feature flags in the workspace:

```toml
[features]
default = ["runtime_native", "memory_stm"]
runtime_docker = ["dep:bollard"]
runtime_ssh = ["dep:russh"]
memory_ltm = ["dep:qdrant-client"]
memory_stm = []
diagnostics_pg = ["dep:sqlx-postgres"]
provider_openai = []
provider_anthropic = []
provider_ollama = []
tool_lazy_loading = []
evolution_fast_path = []
```

Rules:
- Every non-trivial subsystem gates behind a feature flag
- `default` includes the minimum viable feature set
- Heavy dependencies (Docker, Qdrant) are opt-in
- Feature flag names use `snake_case`

---

## 8. Crate Architecture

### 8.1 Dependency Direction

All crates depend inward on `y-core`. No lateral dependencies between peer crates.

```
                    y-cli
                      |
                  y-agent (orchestrator)
                 /    |    \
          y-provider  |  y-hooks
                 \    |    /
                   y-core
                 /    |    \
          y-tools  y-session  y-storage
```

If crate A needs functionality from crate B (where both are peers), the shared abstraction must be a trait in `y-core`.

### 8.2 Crate Internal Structure

Every crate follows:

```
y-{name}/
  Cargo.toml
  src/
    lib.rs          # Public API surface (re-exports)
    error.rs        # Crate-specific error types
    config.rs       # Configuration structs
    {module}.rs     # Domain modules
    tests/          # Integration tests (if separate from unit tests)
```

Rules:
- `lib.rs` is a thin re-export layer, not business logic
- Each file stays under 500 lines; split into submodules when exceeding
- Private modules are `pub(crate)`; only the public API is `pub`
- Every public type and trait has a doc comment

### 8.3 y-core Trait Design

`y-core` traits are the contracts between crates. They must be:
- Minimal (only the methods that consumers actually need)
- `async_trait`-based for async methods
- `Send + Sync + 'static` bounded (for use with `Arc<dyn Trait>`)
- Version-stable (adding methods is a breaking change; use extension traits for optional capabilities)

```rust
#[async_trait]
pub trait RuntimeAdapter: Send + Sync {
    async fn execute(&self, request: ExecutionRequest) -> Result<ExecutionResult, RuntimeError>;
    async fn health_check(&self) -> Result<HealthStatus, RuntimeError>;
    // ...
}
```

---

## 9. Serialization Conventions

### 9.1 Formats by Context

| Context | Format | Rationale |
|---------|--------|-----------|
| User-facing config | TOML | Human-readable, typed |
| Inter-module data | JSON (serde_json::Value) | Flexible, well-supported |
| Message persistence | JSONL (newline-delimited JSON) | Append-friendly |
| High-performance IPC | Protobuf (tonic/prost) | Binary, schema-enforced |
| Tool parameters | JSON Schema (Draft 7) | LLM function calling standard |
| Skill manifests | TOML | Consistent with config |

### 9.2 Serde Conventions

- All serializable structs derive `Serialize, Deserialize`
- Use `#[serde(rename_all = "snake_case")]` for enums
- Use `#[serde(tag = "type")]` for internally-tagged enums
- Timestamps: ISO 8601 string format (`chrono::DateTime<Utc>`)
- UUIDs: String format (`uuid::Uuid` with serde feature)
- Durations: Milliseconds as `u64`

---

## 10. Database Conventions

### 10.1 SQLite (Operational)

- WAL mode enabled at connection time
- Connection pool via `sqlx::SqlitePool`
- Embedded schema in `crates/y-storage/src/schema.sql`
- Tables prefixed by owning module (e.g., `orchestrator_checkpoints`, `session_metadata`)
- All timestamps stored as ISO 8601 TEXT (SQLite has no native datetime)
- Foreign keys enforced (`PRAGMA foreign_keys = ON`)
- Indexes on all columns used in WHERE clauses

### 10.2 SQLite Diagnostics

- Diagnostics tables share the same `SQLite` database file as operational state
- Schema compatibility tracked via `PRAGMA user_version`
- Incompatible on-disk schemas are archived and recreated during startup
- JSON payloads stored as TEXT
- Retention handled by application-level cleanup jobs

### 10.3 Vector Store (Qdrant)

- Collections per memory type: `ltm_memories`, `kb_documents`
- Payload indexes on frequently filtered fields
- HNSW index with tunable `ef` and `m` parameters
- Batch upsert for bulk operations

### 10.4 Schema Discipline

- Update the embedded schema in `crates/y-storage/src/schema.sql`
- Bump the compatibility contract in `crates/y-storage/src/migration.rs` when the runtime shape changes
- Startup must either adopt a compatible database or archive and recreate an incompatible one
- Schema changes require review regardless of risk tier

---

## 11. API Design

### 11.1 Public API Principles

- **Builder pattern** for complex constructors (3+ required fields)
- **Type-state pattern** for multi-step initialization (e.g., `Pool::new().with_provider().build()`)
- **Iterator/Stream** for collections and paginated results
- Return `impl Trait` for concrete types behind trait bounds (avoid `Box<dyn>` when possible)

### 11.2 Breaking Change Policy

During `0.x` development:
- Breaking changes are permitted between minor versions
- Internal crate APIs may change freely
- `y-core` trait changes require updating all dependent crates in the same commit
- Deprecation warnings via `#[deprecated]` for at least one minor version before removal

---

## 12. Git Workflow

### 12.1 Branch Strategy

- `main`: always builds, always passes CI
- `feat/{name}`: feature branches, one concern per branch
- `fix/{name}`: bug fix branches
- `docs/{name}`: documentation-only branches

### 12.2 Commit Messages

Format: `{type}({scope}): {description}`

Types: `feat`, `fix`, `refactor`, `docs`, `test`, `ci`, `chore`

Scope: crate name or `workspace` for cross-cutting changes

```
feat(y-provider): add tag-based routing with priority scheduler
fix(y-storage): handle WAL checkpoint race in concurrent writes
docs(design): update memory-architecture-design for 3-tier model
test(y-hooks): add middleware chain cancellation tests
ci: add cargo-deny license check to pipeline
```

### 12.3 Merge Policy

- Squash merge to `main` (clean linear history)
- PR title becomes the squash commit message (follow format above)
- Delete branch after merge
