# y-agent Database Schema

This directory serves as the entry point for database schema documentation.

## Schema Design Document

The comprehensive schema design is maintained in:

- **[DATABASE_SCHEMA.md](../standards/DATABASE_SCHEMA.md)** — Full schema definitions for SQLite and Qdrant

## Runtime Schema Source

The runtime `SQLite` schema is embedded directly into the binary:

- **[crates/y-storage/src/schema.sql](../../crates/y-storage/src/schema.sql)** — Embedded operational + diagnostics schema
- **[crates/y-storage/src/migration.rs](../../crates/y-storage/src/migration.rs)** — Compatibility check, reset, and schema initialization helpers

## Storage Backends

| Backend | Purpose | Runtime Source |
|---------|---------|----------------|
| SQLite (WAL) | Operational + diagnostics state: sessions, checkpoints, schedules, chat history, traces, provider metrics | `crates/y-storage/src/schema.sql` |
| Qdrant | Vector store: LTM memories, knowledge base documents | N/A (configured via API) |

## Active SQLite Tables

| Category | Tables |
|----------|--------|
| Sessions | `session_metadata` |
| Orchestration | `orchestrator_workflows`, `orchestrator_checkpoints` |
| Scheduling | `schedule_definitions`, `schedule_executions` |
| Chat | `chat_checkpoints`, `chat_messages` |
| Diagnostics | `diag_traces`, `diag_observations`, `diag_scores`, `provider_metrics_log` |

## Applying the Embedded Schema

```bash
# From the application path:
y-agent init

# Or by starting the service/CLI, which will reconcile the on-disk database
# against the embedded schema before opening the normal SQLite pool.
```
