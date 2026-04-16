# y-web -- Web API Server

HTTP REST API server for y-agent, built on [axum](https://github.com/tokio-rs/axum).

The Web API is a thin presentation layer with full feature parity with the
desktop GUI.  All business logic lives in `y-service::ServiceContainer`.
Real-time push events are delivered via Server-Sent Events (SSE).

## Quick Start

```bash
# Start the API server (default: http://0.0.0.0:3000)
y-agent serve

# Custom host/port
y-agent serve --host 127.0.0.1 --port 8080
```

## API Endpoints

### System

```bash
# Health check (liveness probe)
curl http://localhost:3000/health

# Full system status
curl http://localhost:3000/api/v1/status

# List registered LLM providers
curl http://localhost:3000/api/v1/providers

# Application paths (config_dir, data_dir)
curl http://localhost:3000/api/v1/app-paths
```

### Events (SSE)

```bash
# Subscribe to all real-time events
curl -N http://localhost:3000/api/v1/events

# Filter events by session
curl -N "http://localhost:3000/api/v1/events?session_id=SESSION_ID"
```

Event types: `ChatStarted`, `ChatProgress`, `ChatComplete`, `ChatError`,
`AskUser`, `PermissionRequest`, `TitleUpdated`, `DiagnosticsEvent`,
`KbBatchProgress`, `KbEntryIngested`.

### Chat

```bash
# Synchronous single turn (blocks until complete)
curl -X POST http://localhost:3000/api/v1/chat \
  -H "Content-Type: application/json" \
  -d '{"message": "Hello, what can you do?"}'

# Continue an existing session
curl -X POST http://localhost:3000/api/v1/chat \
  -H "Content-Type: application/json" \
  -d '{"message": "Tell me more", "session_id": "SESSION_ID"}'

# Async turn (returns immediately, streams progress via SSE)
curl -X POST http://localhost:3000/api/v1/chat/send \
  -H "Content-Type: application/json" \
  -d '{"message": "Hello", "session_id": "SESSION_ID"}'

# Cancel a running turn
curl -X POST http://localhost:3000/api/v1/chat/cancel \
  -H "Content-Type: application/json" \
  -d '{"run_id": "RUN_ID"}'

# Undo to a checkpoint
curl -X POST http://localhost:3000/api/v1/chat/undo \
  -H "Content-Type: application/json" \
  -d '{"session_id": "SESSION_ID", "checkpoint_id": "CP_ID"}'

# Resend from a checkpoint (async, SSE-streamed)
curl -X POST http://localhost:3000/api/v1/chat/resend \
  -H "Content-Type: application/json" \
  -d '{"session_id": "SESSION_ID", "checkpoint_id": "CP_ID"}'

# List checkpoints
curl http://localhost:3000/api/v1/chat/checkpoints/SESSION_ID

# Find a checkpoint matching a user message
curl -X POST http://localhost:3000/api/v1/chat/find-checkpoint \
  -H "Content-Type: application/json" \
  -d '{"session_id": "SESSION_ID", "user_message_content": "hello"}'

# Messages with active/tombstone branch status
curl http://localhost:3000/api/v1/chat/messages-with-status/SESSION_ID

# Swap active and tombstone branches
curl -X POST http://localhost:3000/api/v1/chat/restore-branch \
  -H "Content-Type: application/json" \
  -d '{"session_id": "SESSION_ID"}'

# Context compaction
curl -X POST http://localhost:3000/api/v1/chat/compact/SESSION_ID

# Deliver an AskUser answer
curl -X POST http://localhost:3000/api/v1/chat/answer-question \
  -H "Content-Type: application/json" \
  -d '{"session_id": "SESSION_ID", "answer": "yes"}'

# Deliver a permission decision
curl -X POST http://localhost:3000/api/v1/chat/answer-permission \
  -H "Content-Type: application/json" \
  -d '{"session_id": "SESSION_ID", "granted": true}'

# Last turn metadata (provider, model, tokens)
curl http://localhost:3000/api/v1/chat/last-turn-meta/SESSION_ID
```

**Synchronous response:**
```json
{
  "content": "I can help you with...",
  "model": "gpt-4",
  "session_id": "abc123",
  "input_tokens": 150,
  "output_tokens": 200,
  "cost_usd": 0.0035,
  "tool_calls": [],
  "iterations": 1
}
```

### Sessions

```bash
# List sessions (newest first, default: active only)
curl http://localhost:3000/api/v1/sessions

# Filter by state or agent
curl "http://localhost:3000/api/v1/sessions?state=Archived"
curl "http://localhost:3000/api/v1/sessions?agent_id=AGENT_ID"

# Create a new session
curl -X POST http://localhost:3000/api/v1/sessions \
  -H "Content-Type: application/json" \
  -d '{"title": "My Project", "agent_id": "default"}'

# Get session details
curl http://localhost:3000/api/v1/sessions/SESSION_ID

# Delete a session
curl -X DELETE http://localhost:3000/api/v1/sessions/SESSION_ID

# List messages
curl http://localhost:3000/api/v1/sessions/SESSION_ID/messages

# Get last 5 messages
curl "http://localhost:3000/api/v1/sessions/SESSION_ID/messages?last=5"

# Archive a session
curl -X POST http://localhost:3000/api/v1/sessions/SESSION_ID/archive

# Branch a session
curl -X POST http://localhost:3000/api/v1/sessions/SESSION_ID/branch \
  -H "Content-Type: application/json" \
  -d '{"label": "experiment-1"}'

# Truncate messages (keep first N)
curl -X POST http://localhost:3000/api/v1/sessions/SESSION_ID/truncate \
  -H "Content-Type: application/json" \
  -d '{"keep_count": 10}'

# Get/set context reset index
curl http://localhost:3000/api/v1/sessions/SESSION_ID/context-reset
curl -X PUT http://localhost:3000/api/v1/sessions/SESSION_ID/context-reset \
  -H "Content-Type: application/json" \
  -d '{"index": 5}'

# Get/set custom system prompt
curl http://localhost:3000/api/v1/sessions/SESSION_ID/custom-prompt
curl -X PUT http://localhost:3000/api/v1/sessions/SESSION_ID/custom-prompt \
  -H "Content-Type: application/json" \
  -d '{"prompt": "You are a helpful coding assistant."}'

# Fork a session at a message index
curl -X POST http://localhost:3000/api/v1/sessions/SESSION_ID/fork \
  -H "Content-Type: application/json" \
  -d '{"message_index": 4, "title": "Forked conversation"}'

# Rename (set manual title)
curl -X PUT http://localhost:3000/api/v1/sessions/SESSION_ID/rename \
  -H "Content-Type: application/json" \
  -d '{"title": "New Title"}'
```

### Agents

```bash
# List registered agents
curl http://localhost:3000/api/v1/agents

# Get agent details
curl http://localhost:3000/api/v1/agents/AGENT_ID

# Get raw TOML source
curl http://localhost:3000/api/v1/agents/AGENT_ID/source

# Save (create/update) an agent
curl -X PUT http://localhost:3000/api/v1/agents/AGENT_ID \
  -H "Content-Type: application/json" \
  -d '{"content": "[agent]\nname = \"my-agent\"\n..."}'

# Reset to built-in defaults
curl -X POST http://localhost:3000/api/v1/agents/AGENT_ID/reset

# Reload all agents from disk
curl -X POST http://localhost:3000/api/v1/agents/reload

# Parse and validate TOML without saving
curl -X POST http://localhost:3000/api/v1/agents/parse-toml \
  -H "Content-Type: application/json" \
  -d '{"content": "[agent]\nname = \"test\"\n..."}'

# List available tools
curl http://localhost:3000/api/v1/agents/tools

# List prompt section identifiers
curl http://localhost:3000/api/v1/agents/prompt-sections

# Translate text
curl -X POST http://localhost:3000/api/v1/agents/translate \
  -H "Content-Type: application/json" \
  -d '{"text": "Hello world", "target_language": "zh"}'
```

### Tools

```bash
# List registered tools
curl http://localhost:3000/api/v1/tools
```

### Config

```bash
# Get full config (all sections merged as JSON)
curl http://localhost:3000/api/v1/config

# Get a single section as raw TOML
curl http://localhost:3000/api/v1/config/providers

# Save a section (raw TOML)
curl -X PUT http://localhost:3000/api/v1/config/providers \
  -H "Content-Type: application/json" \
  -d '{"content": "[[provider]]\nname = \"openai\"\n..."}'

# Hot-reload all configuration
curl -X POST http://localhost:3000/api/v1/config/reload

# MCP server configuration
curl http://localhost:3000/api/v1/config/mcp
curl -X PUT http://localhost:3000/api/v1/config/mcp \
  -H "Content-Type: application/json" \
  -d '{"mcpServers": {}}'

# Prompt files
curl http://localhost:3000/api/v1/config/prompts
curl http://localhost:3000/api/v1/config/prompts/system.txt
curl -X PUT http://localhost:3000/api/v1/config/prompts/system.txt \
  -H "Content-Type: application/json" \
  -d '{"content": "You are a helpful assistant."}'
curl http://localhost:3000/api/v1/config/prompts/system.txt/default

# Test a provider configuration
curl -X POST http://localhost:3000/api/v1/providers/test \
  -H "Content-Type: application/json" \
  -d '{"provider_type": "openai", "model": "gpt-4", "api_key": "", "api_key_env": "OPENAI_API_KEY"}'

# List models from a provider endpoint
curl -X POST http://localhost:3000/api/v1/providers/list-models \
  -H "Content-Type: application/json" \
  -d '{"base_url": "https://api.openai.com/v1", "api_key": "", "api_key_env": "OPENAI_API_KEY"}'
```

Config sections: `providers`, `storage`, `session`, `runtime`, `hooks`,
`tools`, `guardrails`, `browser`, `knowledge`.

### Workspaces

```bash
# List workspaces
curl http://localhost:3000/api/v1/workspaces

# Create a workspace
curl -X POST http://localhost:3000/api/v1/workspaces \
  -H "Content-Type: application/json" \
  -d '{"name": "My Project", "path": "/home/user/project"}'

# Update a workspace
curl -X PUT http://localhost:3000/api/v1/workspaces/WS_ID \
  -H "Content-Type: application/json" \
  -d '{"name": "Renamed", "path": "/new/path"}'

# Delete a workspace
curl -X DELETE http://localhost:3000/api/v1/workspaces/WS_ID

# Session-to-workspace mapping
curl http://localhost:3000/api/v1/workspaces/session-map

# Assign/unassign sessions
curl -X POST http://localhost:3000/api/v1/workspaces/assign \
  -H "Content-Type: application/json" \
  -d '{"workspace_id": "WS_ID", "session_id": "SESSION_ID"}'

curl -X POST http://localhost:3000/api/v1/workspaces/unassign \
  -H "Content-Type: application/json" \
  -d '{"session_id": "SESSION_ID"}'
```

### Skills

```bash
# List installed skills
curl http://localhost:3000/api/v1/skills

# Get skill details
curl http://localhost:3000/api/v1/skills/SKILL_NAME

# Uninstall a skill
curl -X DELETE http://localhost:3000/api/v1/skills/SKILL_NAME

# Enable/disable a skill
curl -X PUT http://localhost:3000/api/v1/skills/SKILL_NAME/enabled \
  -H "Content-Type: application/json" \
  -d '{"enabled": true}'

# List files in skill directory
curl http://localhost:3000/api/v1/skills/SKILL_NAME/files

# Read/write a skill file
curl http://localhost:3000/api/v1/skills/SKILL_NAME/files/main.py
curl -X PUT http://localhost:3000/api/v1/skills/SKILL_NAME/files/main.py \
  -H "Content-Type: application/json" \
  -d '{"content": "print(\"hello\")"}'
```

### Knowledge

```bash
# List collections
curl http://localhost:3000/api/v1/knowledge/collections

# Create a collection
curl -X POST http://localhost:3000/api/v1/knowledge/collections \
  -H "Content-Type: application/json" \
  -d '{"name": "docs"}'

# Delete a collection
curl -X DELETE http://localhost:3000/api/v1/knowledge/collections/docs

# Rename a collection
curl -X POST http://localhost:3000/api/v1/knowledge/collections/docs/rename \
  -H "Content-Type: application/json" \
  -d '{"new_name": "documentation"}'

# List entries in a collection
curl http://localhost:3000/api/v1/knowledge/collections/docs/entries

# Get/delete an entry
curl http://localhost:3000/api/v1/knowledge/entries/ENTRY_ID
curl -X DELETE http://localhost:3000/api/v1/knowledge/entries/ENTRY_ID

# Update entry metadata
curl -X PATCH http://localhost:3000/api/v1/knowledge/entries/ENTRY_ID/metadata \
  -H "Content-Type: application/json" \
  -d '{"tags": ["important"]}'

# Semantic search
curl -X POST http://localhost:3000/api/v1/knowledge/search \
  -H "Content-Type: application/json" \
  -d '{"query": "how to configure providers", "collection": "docs", "limit": 5}'

# Ingest a document
curl -X POST http://localhost:3000/api/v1/knowledge/ingest \
  -H "Content-Type: application/json" \
  -d '{"collection": "docs", "source": "/path/to/file.md"}'

# Batch ingest (progress via SSE)
curl -X POST http://localhost:3000/api/v1/knowledge/ingest-batch \
  -H "Content-Type: application/json" \
  -d '{"collection": "docs", "sources": ["/path/a.md", "/path/b.md"]}'

# Expand a folder into file paths
curl -X POST http://localhost:3000/api/v1/knowledge/expand-folder \
  -H "Content-Type: application/json" \
  -d '{"path": "/path/to/folder"}'

# Knowledge base statistics
curl http://localhost:3000/api/v1/knowledge/stats
```

### Observability

```bash
# Live system state snapshot
curl http://localhost:3000/api/v1/observability/snapshot

# Snapshot with history (RFC 3339 timestamps)
curl "http://localhost:3000/api/v1/observability/history?since=2024-01-01T00:00:00Z&until=2024-12-31T23:59:59Z"
```

### Rewind

```bash
# List rewind points for a session
curl http://localhost:3000/api/v1/rewind/SESSION_ID/points

# Execute full rewind (transcript + files + checkpoints)
curl -X POST http://localhost:3000/api/v1/rewind/SESSION_ID/execute \
  -H "Content-Type: application/json" \
  -d '{"target_message_id": "MSG_ID"}'

# Restore files only (no transcript truncation)
curl -X POST http://localhost:3000/api/v1/rewind/SESSION_ID/restore-files \
  -H "Content-Type: application/json" \
  -d '{"target_message_id": "MSG_ID"}'
```

### Attachments

```bash
# Read image files as base64 (png, jpg, jpeg, gif, webp; max 20 MB)
curl -X POST http://localhost:3000/api/v1/attachments/read \
  -H "Content-Type: application/json" \
  -d '{"paths": ["/path/to/image.png"]}'
```

### Workflows

```bash
# List workflows
curl http://localhost:3000/api/v1/workflows

# Create a workflow
curl -X POST http://localhost:3000/api/v1/workflows \
  -H "Content-Type: application/json" \
  -d '{"name": "my-workflow", "definition": {...}}'

# Get workflow details
curl http://localhost:3000/api/v1/workflows/WORKFLOW_ID

# Update a workflow
curl -X PUT http://localhost:3000/api/v1/workflows/WORKFLOW_ID \
  -H "Content-Type: application/json" \
  -d '{"definition": {...}}'

# Delete a workflow
curl -X DELETE http://localhost:3000/api/v1/workflows/WORKFLOW_ID

# Validate a workflow definition
curl -X POST http://localhost:3000/api/v1/workflows/validate \
  -H "Content-Type: application/json" \
  -d '{"definition": {...}}'

# Get DAG visualization
curl http://localhost:3000/api/v1/workflows/WORKFLOW_ID/dag

# Execute a workflow
curl -X POST http://localhost:3000/api/v1/workflows/WORKFLOW_ID/execute
```

### Schedules

```bash
# List schedules
curl http://localhost:3000/api/v1/schedules

# Create a schedule
curl -X POST http://localhost:3000/api/v1/schedules \
  -H "Content-Type: application/json" \
  -d '{"name": "daily-report", "cron": "0 9 * * *", "workflow_id": "WORKFLOW_ID"}'

# Get schedule details
curl http://localhost:3000/api/v1/schedules/SCHEDULE_ID

# Update a schedule
curl -X PUT http://localhost:3000/api/v1/schedules/SCHEDULE_ID \
  -H "Content-Type: application/json" \
  -d '{"cron": "0 10 * * *"}'

# Delete a schedule
curl -X DELETE http://localhost:3000/api/v1/schedules/SCHEDULE_ID

# Pause/resume
curl -X POST http://localhost:3000/api/v1/schedules/SCHEDULE_ID/pause
curl -X POST http://localhost:3000/api/v1/schedules/SCHEDULE_ID/resume

# Execution history
curl http://localhost:3000/api/v1/schedules/SCHEDULE_ID/executions

# Get a specific execution
curl http://localhost:3000/api/v1/schedules/executions/EXECUTION_ID

# Trigger immediately
curl -X POST http://localhost:3000/api/v1/schedules/SCHEDULE_ID/trigger
```

### Diagnostics

```bash
# List recent traces
curl http://localhost:3000/api/v1/diagnostics/traces

# Filter by session
curl "http://localhost:3000/api/v1/diagnostics/traces?session_id=SESSION_ID&limit=10"

# Get trace detail
curl http://localhost:3000/api/v1/diagnostics/traces/TRACE_UUID

# Get diagnostics for a session
curl http://localhost:3000/api/v1/diagnostics/sessions/SESSION_ID

# Sub-agent execution history
curl http://localhost:3000/api/v1/diagnostics/subagents
```

### Bot Webhooks

```bash
# Feishu webhook
curl -X POST http://localhost:3000/api/v1/bots/feishu/webhook

# Discord webhook
curl -X POST http://localhost:3000/api/v1/bots/discord/webhook
```

## Error Format

All errors return JSON:

```json
{
  "error": "not_found",
  "message": "session abc123 not found"
}
```

| HTTP Status | Error Code | Description |
|-------------|-----------|-------------|
| 400 | `bad_request` | Invalid request body or parameters |
| 404 | `not_found` | Resource not found |
| 500 | `internal_error` | Server-side error |

## Current Limitations

> [!WARNING]
> The following features are **not yet implemented**:

- **Authentication** -- No API key or token-based auth middleware
- **Rate Limiting** -- No request throttling
- **Request Size Limits** -- No body size constraints

## OpenAPI Specification

Full API specification: [`docs/api/openapi.yaml`](../../docs/api/openapi.yaml)

## Architecture

```
HTTP Client  ->  axum Router  ->  handlers  ->  y-service  ->  domain crates
             <-  SSE events   <-
```

The server is a thin presentation layer with full feature parity with the
desktop GUI.  All business logic lives in `y-service::ServiceContainer`.
CORS is enabled via `tower-http` CorsLayer.
