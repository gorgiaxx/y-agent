# y-agent

> A modular, extensible AI agent runtime written in Rust.

## Features

- **Modular Architecture** — 21 specialized crates with clean trait boundaries
- **Multi-Provider LLM** — Provider pool with failover, routing, and streaming
- **Context Management** — 7-stage middleware pipeline for prompt assembly
- **Tool System** — Lazy-loaded tools with JSON Schema validation and LRU activation
- **Memory** — Short-term, long-term, and working memory with semantic search
- **Workflow Engine** — DAG-based orchestration with checkpointing and interrupt/resume
- **Multi-Agent** — Session tree with parent/child delegation
- **Guardrails** — Content filtering, PII detection, and safety checks as middleware

## Quick Start

### Prerequisites

- Rust 1.76+ (`rustup update stable`)
- SQLite 3.35+ (embedded; CLI tool optional)
- PostgreSQL 14+ (optional — diagnostics and analytics)
- Qdrant (optional — vector search for memory/knowledge)

### Build

```bash
cargo build --release
```

### Initialize

Run the interactive setup wizard to detect dependencies, select an LLM provider, and generate all configuration files:

```bash
y-agent init
```

This will:
1. Check environment dependencies (Rust, Docker, PostgreSQL, Qdrant, sqlx-cli)
2. Guide you through LLM provider selection (OpenAI, Anthropic, DeepSeek, Groq, Together AI, Ollama, or a custom endpoint)
3. Generate 8 configuration files in `config/`, a `.env` file, and `data/` directories

See [Project Initialization](#project-initialization) below for full usage details.

### Run

```bash
# Set your API key (provider-specific, shown after init)
export OPENAI_API_KEY="sk-..."

# Start a session
y-agent chat
```

### Test

```bash
# Run all tests (~1600+ tests)
cargo test

# Run a specific crate
cargo test -p y-core

# Run E2E integration tests
cargo test -p y-cli --test chat_flow --test session_lifecycle
```

### Benchmark

```bash
# Run all benchmarks
cargo bench

# Run a specific benchmark
cargo bench --bench hooks_bench
```

## Project Initialization

The `y-agent init` command bootstraps a new project in one step. It runs before the configuration loader and requires no pre-existing files.

### Interactive Mode (default)

```bash
y-agent init
```

Sample session:

```
  y-agent v0.1.0 — Project Initialization
  =========================================

  Checking environment dependencies...

  Dependency       Status     Detail
  ----------       ------     ------
  rustc            found      rustc 1.76.0
  cargo            found      cargo 1.76.0
  sqlite3          found      3.43.2
  docker           found      Docker version 24.0.7
  docker compose   found      Docker Compose version v2.24.5
  PostgreSQL       found      port 5432 reachable
  Qdrant           not found  not found (optional — for vector search)
  sqlx-cli         not found  not found (optional — for migrations)

  Select LLM provider:
  > OpenAI (GPT-4o)
    Anthropic (Claude 3.5 Sonnet)
    DeepSeek (Chat)
    DeepSeek (Reasoner)
    Groq (Llama 3.1 70B)
    Together AI (Llama 3.1 70B)
    Ollama (Local — no API key needed)
    Custom (OpenAI-compatible endpoint)

  API key environment variable name [OPENAI_API_KEY]: OPENAI_API_KEY
  Add another provider? [y/N]: N

  Created config/y-agent.toml
  Created config/providers.toml (OpenAI GPT-4o)
  Created config/storage.toml
  Created config/session.toml
  Created config/runtime.toml
  Created config/hooks.toml
  Created config/tools.toml
  Created config/guardrails.toml
  Created .env
  Created data/
  Created data/transcripts/

  Next steps:
  1. Set your API key:  export OPENAI_API_KEY="sk-..."
  2. Review config:     config/
  3. Validate config:   y-agent config validate
  4. Start chatting:    y-agent chat
```

### Non-Interactive Mode (CI / scripting)

```bash
# Use a built-in provider preset
y-agent init --non-interactive --provider openai

# Override the API key env var name
y-agent init --non-interactive --provider anthropic --api-key-env MY_CLAUDE_KEY

# Use a custom OpenAI-compatible endpoint
y-agent init --non-interactive --provider custom \
  --model my-model \
  --base-url https://api.example.com/v1 \
  --api-key-env MY_API_KEY

# Use Ollama (local, no API key required)
y-agent init --non-interactive --provider ollama
```

### Available Flags

| Flag | Description |
|------|-------------|
| `--provider <KEY>` | Provider preset: `openai`, `anthropic`, `deepseek`, `deepseek-reasoner`, `groq`, `together`, `ollama`, `custom` |
| `--api-key-env <VAR>` | Override the API key environment variable name |
| `--model <NAME>` | Model name (for `custom` provider) |
| `--base-url <URL>` | Base URL (for `custom` provider) |
| `--non-interactive` | Skip all interactive prompts; use defaults and flags |
| `--dir <PATH>` | Target directory for generated files (default: `.`) |
| `--force` | Overwrite existing config files without asking |

### Provider Presets

| Key | Provider | Model | API Key Env Var |
|-----|----------|-------|-----------------|
| `openai` | OpenAI | GPT-4o | `OPENAI_API_KEY` |
| `anthropic` | Anthropic | Claude 3.5 Sonnet | `ANTHROPIC_API_KEY` |
| `deepseek` | DeepSeek | deepseek-chat | `DEEPSEEK_API_KEY` |
| `deepseek-reasoner` | DeepSeek | deepseek-reasoner | `DEEPSEEK_API_KEY` |
| `groq` | Groq | Llama 3.1 70B | `GROQ_API_KEY` |
| `together` | Together AI | Llama 3.1 70B | `TOGETHER_API_KEY` |
| `ollama` | Ollama (local) | llama3.1 | (none) |
| `custom` | Any OpenAI-compatible | (user-specified) | (user-specified) |

### Generated Files

After running `init`, the following files and directories are created:

```
./
├── .env                       # Environment variables (API key placeholders)
├── config/
│   ├── y-agent.toml           # Global settings (log level, output format)
│   ├── providers.toml         # LLM provider pool (patched with your selection)
│   ├── storage.toml           # Database and transcript settings
│   ├── session.toml           # Session tree, compaction, auto-archive
│   ├── runtime.toml           # Docker/Native sandbox, resource limits
│   ├── hooks.toml             # Middleware timeouts, event bus capacity
│   ├── tools.toml             # Tool registry limits
│   └── guardrails.toml        # Permission model, loop detection, risk scoring
└── data/
    └── transcripts/           # Session transcript storage
```

### Re-Running Init

Running `init` on an existing project will prompt before overwriting each file. Use `--force` to skip confirmation:

```bash
y-agent init --force
```

## Project Structure

```
crates/
├── y-core/           # Trait definitions, shared types, error types
├── y-agent/          # Unified agent: orchestrator, DAG engine, multi-agent pool, delegation
├── y-cli/            # CLI binary, TUI (ratatui), config, wire format
├── y-context/        # Context pipeline, token budget, memory integration
├── y-diagnostics/    # Tracing, metrics, health checks (PostgreSQL)
├── y-guardrails/     # Content filtering, PII, safety middleware
├── y-hooks/          # Middleware chains, event bus, plugin loading
├── y-journal/        # File journal, rollback, conflict detection
├── y-knowledge/      # Knowledge base chunking, indexing, retrieval
├── y-mcp/            # MCP protocol client/server
├── y-prompt/         # Prompt sections, templates, TOML store
├── y-provider/       # LLM provider pool, routing, streaming
├── y-runtime/        # Native/Docker/SSH sandbox execution
├── y-scheduler/      # Cron/interval scheduling, workflow triggers
├── y-service/        # Business/service layer (shared by CLI, TUI, Web API)
├── y-session/        # Session tree, transcript, branching
├── y-skills/         # Skill discovery, validation, manifest
├── y-storage/        # SQLite/Postgres/Qdrant backends
├── y-test-utils/     # Mocks, fixtures, assertion helpers
├── y-tools/          # Tool registry, JSON Schema validation
└── y-web/            # HTTP REST API server (axum)
docs/
└── guides/           # User guides (configuration, tool/skill authoring, web API, architecture)
```

## Documentation

| Guide | Description |
|-------|-------------|
| [Configuration](docs/guides/CONFIGURATION.md) | Environment variables, config files, provider setup |
| [Tool Authoring](docs/guides/TOOL_AUTHORING.md) | How to create custom tools |
| [Skill Authoring](docs/guides/SKILL_AUTHORING.md) | How to create agent skills |
| [Architecture](docs/guides/ARCHITECTURE.md) | Contributor architecture overview |

## Deployment

### Docker Quick Start

```bash
# Initialize project (generates .env and config files)
y-agent init

# Or manually: cp .env.example .env && edit .env with your API key

# Start the full stack (y-agent + PostgreSQL + Qdrant)
docker compose up -d

# Check service health
./scripts/health-check.sh

# View logs
docker compose logs -f y-agent
```

### Production Deployment

y-agent supports automated deployment via GitHub Actions:

1. **Configure GitHub Secrets** in your repository settings:
   - `DEPLOY_HOST` — Target server address
   - `DEPLOY_USER` — SSH username
   - `DEPLOY_SSH_KEY` — SSH private key
   - `DEPLOY_PATH` — Deployment directory on server

2. **Trigger a release** by pushing a version tag:
   ```bash
   git tag v0.1.0
   git push origin v0.1.0
   ```

3. The pipeline will automatically:
   - Run CI checks (clippy, tests, fmt, audit)
   - Build multi-arch Docker images (`linux/amd64`, `linux/arm64`)
   - Publish to GitHub Container Registry (`ghcr.io`)
   - Build native binaries for 4 platforms
   - Create a GitHub Release
   - Deploy to production via SSH

### Native Install (No Docker)

```bash
# Build from source and install to /usr/local/bin
./scripts/native-install.sh

# Or customize the installation
./scripts/native-install.sh --prefix ~/.local --data-dir ~/y-agent-data
```

This creates:
- Binary at `$PREFIX/bin/y-agent`
- Config at `~/.config/y-agent/config.toml` (from [`config/y-agent.example.toml`](config/y-agent.example.toml))
- Data at `~/.local/share/y-agent/`

### Manual Deployment

```bash
# Deploy a specific version (Docker-based)
DEPLOY_DIR=/opt/y-agent ./scripts/deploy.sh v0.1.0

# Deploy latest
DEPLOY_DIR=/opt/y-agent ./scripts/deploy.sh latest
```

### Configuration

Configuration files are split by concern in the `config/` directory:

| File | Description |
|------|-------------|
| [`y-agent.toml`](config/y-agent.example.toml) | Global settings (log level, output format) |
| [`providers.toml`](config/providers.example.toml) | LLM provider pool (API keys, models, routing tags) |
| [`storage.toml`](config/storage.example.toml) | SQLite database, JSONL transcripts, migrations |
| [`session.toml`](config/session.example.toml) | Session tree depth, compaction, auto-archive |
| [`runtime.toml`](config/runtime.example.toml) | Docker/Native sandbox, image whitelist, resource limits |
| [`hooks.toml`](config/hooks.example.toml) | Middleware timeouts, event bus capacity |
| [`tools.toml`](config/tools.example.toml) | Tool registry limits, dynamic tool creation |
| [`guardrails.toml`](config/guardrails.example.toml) | Permission model, loop detection, risk scoring |

To generate all config files automatically, run `y-agent init`. To copy them manually instead:
```bash
for f in config/*.example.toml; do cp "$f" "config/$(basename "$f" .example.toml).toml"; done
```

## License

MIT OR Apache-2.0
