# Development Guide

This section provides a comprehensive technical reference for developers working on or contributing to y-agent. It covers the system architecture, runtime flows, crate responsibilities, and collaboration practices.

## Quick Index

| Document | Description |
|----------|-------------|
| [Architecture](./architecture) | 6-layer architecture, dependency graph, data stores |
| [Request Lifecycle](./request-lifecycle) | End-to-end flow of a chat request with sequence diagrams |
| [Crate Reference](./crate-reference) | All 24 workspace crates -- purpose, key types, and exports |
| [Agent System](./agent-system) | Agent framework, delegation protocol, multi-agent patterns |
| [Tool System](./tool-system) | Tool registry, lazy loading, execution pipeline, JSON Schema validation |
| [Context Pipeline](./context-pipeline) | Context assembly stages, compaction, pruning strategies |
| [Provider Pool](./provider-pool) | LLM provider routing, freeze/thaw failover, priority scheduling |
| [Middleware & Hooks](./middleware-hooks) | Middleware chains, guardrails, event bus, HITL protocol |
| [Storage & Sessions](./storage-sessions) | SQLite persistence, session tree, transcripts, file journal |
| [Contributing](./contributing) | TDD workflow, quality gates, commit discipline, code review |

## Architecture at a Glance

```
                        +---------------------+
                        |    Presentation      |
                        |  y-cli  y-web  y-gui |
                        +----------+----------+
                                   |
                        +----------v----------+
                        |      Service         |
                        |     y-service        |
                        +----------+----------+
                                   |
              +--------------------+--------------------+
              |                    |                     |
   +----------v------+  +---------v--------+  +---------v--------+
   |   Orchestration  |  |   Middleware      |  |   Capabilities   |
   |  y-agent  y-bot  |  | y-hooks           |  | y-tools  y-skills|
   |                  |  | y-guardrails      |  | y-runtime        |
   +----------+------+  | y-prompt          |  | y-scheduler      |
              |          | y-mcp             |  | y-browser        |
              |          +---------+--------+  | y-journal        |
              |                    |           +---------+--------+
              +--------------------+--------------------+
                                   |
                        +----------v----------+
                        |   Infrastructure     |
                        | y-provider y-session |
                        | y-context  y-storage |
                        | y-knowledge          |
                        | y-diagnostics        |
                        +----------+----------+
                                   |
                        +----------v----------+
                        |       Core           |
                        |      y-core          |
                        +---------------------+
```

All dependencies point inward toward `y-core`. The service layer (`y-service`) is the sole orchestration hub -- presentation crates are thin I/O wrappers with no domain logic.

## Key Design Decisions

1. **SQLite (WAL mode)** for all operational state; PostgreSQL for analytics; Qdrant for vectors
2. **Tool lazy loading** saves 60--90% of context tokens via `ToolIndex` + `ToolActivationSet`
3. **Every LLM call** goes through the agent framework -- single entry point for observability and guardrails
4. **Guardrails as middleware** -- uniform priority enforcement, not a parallel system
5. **Dual transcripts** per session: context (LLM-facing, compactable) and display (UI-facing, immutable)
6. **Content-addressable skill versioning** with JSONL reflog (Git-like, no external VCS)
7. **Three-tier runtime security**: Docker (strongest) -> Native/bubblewrap -> SSH
