# Configuration Guide

y-agent is configured through environment variables, a TOML config file, and CLI flags. Values are resolved in this order (highest wins):

1. CLI flags
2. Environment variables
3. Config file (`~/.config/y-agent/config.toml`)
4. Built-in defaults

## Provider Configuration

### Environment Variables

```bash
# Required: at least one provider API key
export Y_PROVIDER_API_KEY="sk-..."

# Optional: specific provider configuration
export Y_OPENAI_API_KEY="sk-..."
export Y_ANTHROPIC_API_KEY="sk-ant-..."
export Y_OPENAI_BASE_URL="https://api.openai.com/v1"
export Y_ANTHROPIC_BASE_URL="https://api.anthropic.com"
```

### Config File

```toml
# ~/.config/y-agent/config.toml

[provider.openai]
api_key = "sk-..."                    # or use env: Y_OPENAI_API_KEY
model = "gpt-4"                       # default model
base_url = "https://api.openai.com/v1"
max_concurrency = 5                   # max parallel requests
context_window = 128000               # token limit

[provider.anthropic]
api_key = "sk-ant-..."
model = "claude-sonnet-4-20250514"
max_concurrency = 3
context_window = 200000

# Provider pool configuration
[provider_pool]
default = "openai"                    # primary provider
fallback = ["anthropic"]              # failover chain
routing_strategy = "tag_match"        # or "round_robin", "least_loaded"
```

## Storage Configuration

```toml
[storage]
# SQLite for operational state (sessions, checkpoints)
sqlite_path = "~/.local/share/y-agent/state.db"

# PostgreSQL for diagnostics (optional)
postgres_url = "postgresql://localhost:5432/y_agent"

# Qdrant for semantic search (optional)
qdrant_url = "http://localhost:6334"
qdrant_collection = "y_agent_memory"
```

## Runtime Configuration

```toml
[runtime]
backend = "native"                    # "native", "docker", or "wasm"
timeout_seconds = 30                  # default execution timeout
max_concurrent_tools = 5              # parallel tool limit

[runtime.docker]
image = "y-agent-sandbox:latest"
memory_limit = "512m"
cpu_limit = "1.0"
network = false                       # disable network by default

[runtime.limits]
max_memory_mb = 512
max_cpu_seconds = 30
max_output_bytes = 10485760           # 10 MB
```

## Context Configuration

```toml
[context]
max_tokens = 100000                   # context window budget
compaction_threshold = 0.8            # compact at 80% usage
history_window = 50                   # messages to include
```

## Guardrail Configuration

```toml
[guardrails]
content_filter = true                 # enable content filtering
pii_detection = true                  # redact PII in logs
max_output_length = 50000             # character limit
blocked_domains = []                  # blocked network domains
```

## Session Configuration

```toml
[session]
auto_save = true                      # persist sessions automatically
max_branches = 10                     # max child sessions
transcript_limit = 1000               # max messages per session
```

## CLI Flags

```bash
y-agent chat [OPTIONS]

Options:
  -m, --model <MODEL>         Override the default model
  -p, --provider <PROVIDER>   Override the default provider
  -t, --temperature <FLOAT>   Sampling temperature (0.0-2.0)
  --max-tokens <N>            Max output tokens
  --session <ID>              Resume an existing session
  --no-memory                 Disable memory integration
  -v, --verbose               Enable verbose logging
  -h, --help                  Print help
```
