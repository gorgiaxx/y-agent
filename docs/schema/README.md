# y-agent Database Schema

This directory serves as the entry point for all database schema documentation.

## Schema Design Document

The comprehensive schema design is maintained in:

- **[DATABASE_SCHEMA.md](../standards/DATABASE_SCHEMA.md)** — Full schema definitions for SQLite, PostgreSQL, and Qdrant

## Migration Files

Executable migration SQL files are located in:

- **[migrations/sqlite/](../../migrations/sqlite/)** — SQLite operational schema migrations (6 migration pairs)
- **[migrations/postgres/](../../migrations/postgres/)** — PostgreSQL diagnostics schema migrations (3 migration pairs)

## Storage Backends

| Backend | Purpose | Migration Directory |
|---------|---------|-------------------|
| SQLite (WAL) | Operational state: sessions, checkpoints, workflows, file journal, tools, agents, STM | `migrations/sqlite/` |
| PostgreSQL | Diagnostics: traces, observations, scores, cost records | `migrations/postgres/` |
| Qdrant | Vector store: LTM memories, knowledge base documents | N/A (configured via API) |

## Migration Order

### SQLite

| # | Migration | Tables Created |
|---|-----------|---------------|
| 001 | `initial_sessions` | `session_metadata` |
| 002 | `orchestrator_checkpoints` | `orchestrator_workflows`, `orchestrator_checkpoints` |
| 003 | `file_journal` | `file_journal_entries` |
| 004 | `tools_and_agents` | `tool_dynamic_definitions`, `tool_activation_log`, `agent_definitions` |
| 005 | `schedules` | `schedule_definitions`, `schedule_executions` |
| 006 | `stm_experience` | `stm_experience_store` |

### PostgreSQL

| # | Migration | Objects Created |
|---|-----------|----------------|
| 001 | `observability_schema` | `observability` schema, `traces`, `observations` |
| 002 | `cost_records` | `scores`, `cost_records` |
| 003 | `materialized_views` | `daily_cost_summary` materialized view |

## Running Migrations

```bash
# Install sqlx-cli
cargo install sqlx-cli --features sqlite,postgres

# Run SQLite migrations
sqlx migrate run --source migrations/sqlite --database-url "sqlite:y-agent.db"

# Run PostgreSQL migrations
sqlx migrate run --source migrations/postgres --database-url "postgres://user:pass@localhost/y_agent"
```
