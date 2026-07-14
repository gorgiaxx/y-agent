# y-agent

> A Rust-first, model-agnostic agent harness for turning goals into controlled,
> recoverable, and observable work.

y-agent occupies the same systems category as Codex: it wraps language models
with context management, tools, planning, delegation, permissions, persistence,
and diagnostics. It is not tied to one model vendor or one user interface.

The project is under active implementation.

## Core Capabilities

| Area | Current capability |
| --- | --- |
| Goal-directed execution | Preserves goal, constraints, decisions, and progress through working memory and handoff documents |
| Plan and loop modes | Structured plan review and DAG execution for known work; iterative loop execution for open-ended work |
| Self-orchestration | Agents can delegate to sub-agents and create, inspect, and execute reusable workflows |
| Self-evolving skills | Skill versioning, experience capture, pattern extraction, proposal generation, regression checks, and HITL-controlled refinement |
| Knowledge | Ingestion, metadata extraction, multi-resolution chunking, keyword/vector retrieval, and context injection |
| Tools and MCP | Built-in, dynamic, and MCP tools with schema validation, lazy discovery, permissions, and sandboxed execution |
| Recovery | SQLite WAL state, workflow checkpoints, session transcripts, file journaling, rewind, and provider freeze/thaw |
| Observability | Local traces, observations, token and cost accounting, replay, and optional Langfuse export |
| Interfaces | CLI, TUI, REST/SSE Web API, shared React Web UI, Tauri desktop shell, and bot adapters |

Goal support currently describes execution semantics carried in context and
handoffs. There is not yet a standalone persistent Goal CRUD service.

The Langfuse bridge currently uses Langfuse's native ingestion API. The
diagnostics model is suitable for an OpenTelemetry exporter, but a general OTel
SDK/exporter is not yet wired into the workspace.

## Execution Model

```text
User goal
  -> mode selection: fast, plan, or loop
  -> context assembly: persona, memory, knowledge, skills, tools, history
  -> model turn
  -> tool execution, delegation, or workflow orchestration
  -> guardrails and human approval where required
  -> checkpoints, transcripts, diagnostics, and skill experience capture
  -> result or resumable state
```

The LLM is one component in this loop. `y-service` owns business orchestration;
presentation layers only translate user or transport I/O.

## Quick Start

### Prerequisites

- Rust 1.94, pinned by `rust-toolchain.toml`
- Node.js 18 or newer for the React/Tauri frontend
- Chrome or Chromium only when browser automation is needed

SQLite is embedded. Qdrant and Langfuse are optional integrations.

### Build and initialize

```bash
cargo build --release
cargo run --release -- init
```

Configure at least one provider in `config/providers.toml` or through the GUI.
Provider credentials should be supplied through environment variables whenever
possible.

### Run

```bash
# Direct prompt or interactive chat
cargo run --release -- "inspect this repository"
cargo run --release -- chat

# Terminal UI
cargo run --release -- tui

# REST API and optional Web UI
cargo run --release -- serve

# Desktop development
cd crates/y-gui
npm install
npm run tauri dev
```

Run `y-agent --help` for the current command surface. The CLI is actively
evolving, so generated help is more authoritative than copied command lists.

## Configuration

Configuration is layered from defaults, project files, user files, environment
variables, and CLI overrides. The main configuration areas are:

- providers and routing
- sessions, context, and storage
- runtime and browser isolation
- tools, MCP, hooks, and guardrails
- agents, prompts, knowledge, skills, workflows, and schedules
- diagnostics and optional Langfuse export

Examples live under `config/`. User-facing configuration guidance is maintained
in `website/docs/guide/configuration.md`.

## Architecture

Dependencies point inward toward `y-core`:

```text
Presentation:    y-cli, y-web, y-gui/src-tauri
Service:         y-service
Orchestration:   y-agent, y-bot
Capabilities:    y-tools, y-skills, y-runtime, y-scheduler, y-browser, y-journal
Middleware:      y-hooks, y-guardrails, y-prompt, y-mcp
Infrastructure:  y-provider, y-session, y-context, y-storage, y-knowledge,
                 y-diagnostics
Core:            y-core
```

`crates/y-gui` is the shared React/Vite frontend used by both the Tauri desktop
shell and the browser-hosted Web UI.

See `docs/guides/ARCHITECTURE.md` for the canonical contributor architecture.

## Documentation

The repository intentionally keeps a small documentation surface:

| Location | Purpose |
| --- | --- |
| `docs/guides/ARCHITECTURE.md` | Canonical harness architecture and capability maturity |
| `docs/guides/SKILL_AUTHORING.md` | Skill authoring guide |
| `docs/guides/TOOL_AUTHORING.md` | Tool authoring guide |
| `docs/standards/` | Normative engineering, testing, autonomy, DSL, skill, tool-call, and frontend standards |
| `docs/schema/README.md` | Runtime schema source-of-truth pointers |
| `website/docs/guide/` | User-facing setup and interface guides |
| `website/docs/development/` | Concise public development overview |
| `AGENTS.md` | Repository engineering protocol for coding agents |
| `VISION.md` | Project vision in Chinese |

Historical design drafts, completed plans, copied provider API references,
generated website output, and point-in-time reviews are not maintained as
repository documentation. Git history and current code provide the audit trail.

## Development

The project follows test-driven development. For Rust changes, run the quality
gates in `AGENTS.md`. For shared frontend changes, run the Vitest, ESLint,
desktop build, and Web build gates from `crates/y-gui`.

## License

MIT
