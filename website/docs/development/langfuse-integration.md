# Langfuse Integration

Non-invasive LLM diagnostics export via OTLP/HTTP to [Langfuse](https://langfuse.com). The integration subscribes to existing diagnostics events and exports completed traces without modifying business logic.

## Prerequisites

- A Langfuse account (cloud or self-hosted)
- Your Langfuse project public key and secret key

No external collector, sidecar, or OpenTelemetry SDK installation is required. The bridge sends directly to Langfuse's OTLP endpoint.

## Build with Langfuse Support

The integration is behind a Cargo feature flag. Enable it at compile time:

```bash
cargo build --features langfuse
```

Or add it to your development profile in the workspace root:

```bash
cargo run --features langfuse -- <args>
```

When the `langfuse` feature is not enabled, zero Langfuse-related code is compiled and there is no runtime overhead.

## Configuration

Copy the example config to your active config directory:

```bash
cp config/langfuse.example.toml config/langfuse.toml
```

Edit `config/langfuse.toml`:

```toml
enabled = true
base_url = "https://cloud.langfuse.com"   # or your self-hosted URL
public_key = ""   # leave empty if using env vars
secret_key = ""   # leave empty if using env vars
```

### Credentials

Credentials can be provided in two ways (env vars take precedence):

**Environment variables (recommended for production):**

```bash
export LANGFUSE_PUBLIC_KEY="pk-lf-..."
export LANGFUSE_SECRET_KEY="sk-lf-..."
```

**Config file (convenient for local development):**

```toml
public_key = "pk-lf-..."
secret_key = "sk-lf-..."
```

If `enabled = true` but credentials are missing or empty, the bridge logs a warning and disables itself without affecting agent execution.

## What Gets Exported

Each completed agent trace is mapped to an OTLP trace with nested spans:

| y-agent Concept | OTLP/Langfuse Mapping |
|----------------|----------------------|
| Trace | Root span (SPAN_KIND_SERVER) |
| LLM Generation | Child span (SPAN_KIND_CLIENT) with `gen_ai.*` attributes |
| Tool Call | Child span (SPAN_KIND_INTERNAL) with `tool.*` attributes |
| Sub-Agent Delegation | Child span with link to sub-agent trace |
| Scores | Pushed via Langfuse REST API (`POST /api/public/scores`) |

### Span Attributes

Generation spans include:

- `gen_ai.request.model` / `gen_ai.response.model`
- `gen_ai.usage.input_tokens` / `gen_ai.usage.output_tokens`
- `gen_ai.cost_usd`
- `gen_ai.duration_ms`

Root trace spans include:

- `langfuse.trace.name` / `langfuse.trace.session_id`
- `langfuse.trace.cost_usd`
- `langfuse.trace.tag` (one per tag)
- `langfuse.score.*` (one per local score)

## Content Capture (Opt-in)

By default, prompt and response content is NOT exported. Enable selectively:

```toml
[content]
capture_input = true    # export prompt messages as gen_ai.*.message events
capture_output = true   # export responses as gen_ai.choice events
max_content_length = 10000  # truncate at this character limit
```

When enabled, content follows the OpenTelemetry GenAI semantic conventions (v1.36.0 event model).

## Redaction

When content capture is enabled, a regex-based redaction pipeline runs before export:

```toml
[redaction]
enabled = true
patterns = [
    '(?i)(api[_-]?key|secret|token|password|bearer)\s*[:=]\s*\S+',
    '\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Z|a-z]{2,}\b',
]
replacement = "[REDACTED]"
```

Add custom patterns for project-specific PII or secrets.

## Sampling

Control which traces get exported:

```toml
[sampling]
rate = 1.0          # 1.0 = all, 0.5 = 50% (deterministic hash-based)
include_tags = []   # only export traces with these tags (empty = all)
exclude_tags = []   # never export traces with these tags
```

Sampling is decided before trace assembly to avoid wasted work. Hash-based sampling ensures the same trace is consistently included/excluded across retries.

## Retry and Circuit Breaker

The sender uses exponential backoff for transient failures:

```toml
[retry]
max_retries = 3
initial_backoff_ms = 1000
max_backoff_ms = 30000
```

After consecutive failures, a circuit breaker prevents repeated attempts:

```toml
[circuit_breaker]
failure_threshold = 5       # open after 5 consecutive failures
recovery_timeout_secs = 60  # try again after 60s
```

### Failure Behavior

| Scenario | Behavior |
|----------|----------|
| Missing credentials | Log warning, disable export, agent runs normally |
| Endpoint unreachable | Retry with backoff, then circuit breaker opens |
| Auth failure (401/403) | Log error, drop payload |
| Rate limited (429) | Retry with backoff |
| Broadcast channel overflow | Log warning, drop events (SQLite retains all data) |

Langfuse export failures never block or slow agent execution.

## Feedback Import

Pull human annotations from Langfuse back into the local diagnostics store:

```toml
[feedback]
import_enabled = true
poll_interval_secs = 300    # check every 5 minutes
```

The importer polls `GET /api/public/scores`, deduplicates against previously seen IDs, and inserts new annotations as `ScoreSource::External` records. Only scores for locally-known traces are imported.

## Architecture

```
Agent Execution (unchanged)
    |
    | DiagnosticsEvent broadcast
    v
LangfuseExportBridge (tokio task)
    |-- TraceCompleted event triggers flush
    |-- Reads full trace + observations from SqliteTraceStore
    |-- OtelSpanMapper converts to OTLP JSON
    |-- OtlpHttpSender posts to Langfuse with retry
    |
    v
Langfuse Cloud / Self-Hosted
    POST /api/public/otel/v1/traces (OTLP/HTTP JSON)
    POST /api/public/scores (REST API)
```

Key design properties:

- The bridge runs in its own tokio task; panics are isolated.
- Read-only access to `SqliteTraceStore`; never writes diagnostics tables.
- Subscribes to existing `broadcast::Sender<DiagnosticsEvent>` (256 capacity).
- Abandoned in-flight traces are reaped after 10 minutes.

## Source Layout

```
crates/y-diagnostics/src/langfuse/
    mod.rs          -- module re-exports
    config.rs       -- LangfuseConfig deserialization
    bridge.rs       -- LangfuseExportBridge (event loop + trace assembly)
    mapper.rs       -- OtelSpanMapper (Trace/Observation -> OTLP spans)
    sender.rs       -- OtlpHttpSender (HTTP client + retry + circuit breaker)
    redaction.rs    -- RedactionPipeline (regex-based content sanitization)
    feedback.rs     -- LangfuseFeedbackImporter (periodic score pull)
    types.rs        -- OTLP JSON type definitions
```

## Feature Flag Propagation

```
y-diagnostics  (feature "langfuse")
    <- y-service (feature "langfuse" forwards to y-diagnostics)
        <- y-cli / y-gui (forward as needed)
```

## Experiment Tracking with Tags

Use trace tags to organize experiments in Langfuse:

- `experiment:<id>` -- group traces by experiment run
- `skill:<name>@<version>` -- track skill version performance
- `provider:<id>` -- filter by LLM provider
- `agent:<name>` -- filter by agent

Tags are set by the agent execution layer and exported as `langfuse.trace.tag` span attributes.

## Self-Hosted Langfuse

For self-hosted deployments, point both endpoints to your instance:

```toml
base_url = "https://langfuse.internal.company.com"
```

The OTLP endpoint is derived as `{base_url}/api/public/otel/v1/traces` and the scores API as `{base_url}/api/public/scores`.

## Troubleshooting

**Traces not appearing in Langfuse:**

1. Verify `enabled = true` in `config/langfuse.toml`.
2. Check credentials are set (env vars or config).
3. Look for `"Langfuse export bridge started"` in logs (INFO level).
4. Check for circuit breaker warnings in logs.

**Content not showing in generations:**

- Verify `content.capture_input = true` and/or `content.capture_output = true`.
- Check `max_content_length` is not set too low.

**High latency concerns:**

- The bridge runs asynchronously; it does not block agent execution.
- If export causes backpressure, the broadcast channel drops events (logged at WARN).
- Reduce `sampling.rate` for high-throughput scenarios.
