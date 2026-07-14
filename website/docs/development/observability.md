# Observability

y-agent records agent work as traces and observations so execution can be
inspected, costed, replayed, and evaluated without adding vendor calls to
business logic.

## Built-in Diagnostics

`y-diagnostics` records:

- agent traces and status;
- LLM generations, model, token usage, latency, and cost;
- tool calls and failures;
- sub-agent relationships;
- scores and feedback;
- replay context.

Diagnostics are persisted locally in SQLite. Provider, tool, and delegation
wrappers emit observations around existing interfaces, keeping capture separate
from the agent loop.

## Langfuse Export

Langfuse support is compile-time optional:

```bash
cargo build -p y-cli --features langfuse
cp config/langfuse.example.toml config/langfuse.toml
```

Enable the integration and supply credentials through environment variables:

```toml
enabled = true
base_url = "https://cloud.langfuse.com"
```

```bash
export LANGFUSE_PUBLIC_KEY="pk-lf-..."
export LANGFUSE_SECRET_KEY="sk-lf-..."
```

The exporter subscribes to diagnostics events and sends asynchronous batches to
Langfuse's native `/api/public/ingestion` endpoint. Retry, sampling, redaction,
and circuit-breaker settings are defined in `config/langfuse.example.toml`.
Export failures never block the agent execution path.

## Content and Privacy

Prompt and response capture is disabled by default. When enabled, configure
length limits and redaction patterns for credentials, personal data, and
project-specific secrets.

```toml
[content]
capture_input = false
capture_output = false
max_content_length = 10000

[redaction]
enabled = true
replacement = "[REDACTED]"
```

## Feedback Loop

The optional feedback importer pulls Langfuse scores for locally known traces.
Imported scores can support evaluation and skill-evolution workflows while the
local diagnostics store remains the system of record.

## OpenTelemetry Boundary

Earlier documentation described the Langfuse bridge as an OTLP exporter. The
current implementation uses Langfuse-native ingestion types and does not install
or configure an OpenTelemetry SDK.

A future general OTel exporter should:

1. subscribe to the existing diagnostics event stream;
2. map traces and observations to stable semantic conventions;
3. remain behind its own Cargo feature;
4. export asynchronously with bounded buffering and failure isolation;
5. preserve the same content-capture and redaction policy.

Until that adapter exists in code, y-agent should be described as having an
OTel-ready observability boundary, not a general OTel exporter.
