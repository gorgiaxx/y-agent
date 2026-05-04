# Web API

The y-agent Web API is an axum-based HTTP, JSON, and SSE presentation layer.
It uses the same `y-service::ServiceContainer` as the Tauri desktop GUI, so
business behavior stays in the service layer and the Web API only adapts wire
contracts.

The shared React UI can run in two hosts:

| Host | Backend transport | Notes |
|------|-------------------|-------|
| Desktop GUI | Tauri commands and Tauri events | Native dialogs, local paths, window controls |
| Web UI | REST endpoints and Server-Sent Events | Browser file selection, bearer auth, static SPA serving |

## Quick Start

```bash
# Start the API server on http://0.0.0.0:3000
y-agent serve

# Bind to localhost on a custom port
y-agent serve --host 127.0.0.1 --port 8080

# Protect API routes with a bearer token
y-agent serve --auth-token "$Y_AGENT_WEB_TOKEN"
```

Build and serve the shared Web UI from the same React package used by the
desktop app:

```bash
cd crates/y-gui
npm run build:web
cd ../..
y-agent serve --static-dir crates/y-gui/dist-web
```

## Authentication

`/health` is public for liveness probes. All `/api/v1/*` routes require
`Authorization: Bearer <token>` when `y-agent serve --auth-token` is set.

```bash
curl http://localhost:3000/api/v1/status \
  -H "Authorization: Bearer $Y_AGENT_WEB_TOKEN"
```

Browser `EventSource` cannot set custom headers, so the SSE endpoint also
accepts the token as a query parameter:

```bash
curl -N "http://localhost:3000/api/v1/events?token=$Y_AGENT_WEB_TOKEN"
```

## GUI Parity

Every shared GUI command must either map to a Web API endpoint or be declared
as a host-specific capability.

| GUI surface | Web API coverage |
|-------------|------------------|
| Chat | Async and sync turns, SSE progress, cancel, undo, resend, checkpoints, branch restore, compaction, HITL answers |
| Sessions | List, create, delete, messages, truncate, context reset, custom prompt, fork, rename |
| Agents | List, detail, source, parse/save TOML, reset, reload, tool list, prompt sections, translation |
| Settings | Raw TOML sections, reload, provider test, provider model listing, MCP JSON, prompt files |
| Workspaces | CRUD plus session assignment |
| Skills | List, detail, uninstall, enable/disable, import from server path, file tree, read/write files |
| Knowledge | Collections, entries, metadata, search, ingest, batch ingest, folder expansion, stats |
| Automation | Workflows, validation, DAG view, execution, schedules, pause/resume, history, trigger now |
| Observation | Diagnostics, subagent history, observability snapshots, in-memory stats |
| Attachments | Server-side image path read and multipart upload; browser UI can also inline base64 attachments |
| Background tasks | Per-session process list, poll, write, kill |
| Rewind | Rewind points, full rewind, file-only restore |

Host-specific commands:

| Capability | Desktop | Web |
|------------|---------|-----|
| Native window controls | Supported | Lifecycle no-op or hidden by capability |
| Open skill folder in file manager | Supported | Explicitly unsupported |
| Local path dialogs | Native filesystem paths | Browser file picker or server-reachable paths |
| SSE events | Tauri events | `/api/v1/events` |

## System

```bash
# Public liveness probe and feature negotiation
curl http://localhost:3000/health

# Protected status and path endpoints
curl http://localhost:3000/api/v1/status
curl http://localhost:3000/api/v1/providers
curl http://localhost:3000/api/v1/app-paths
curl http://localhost:3000/api/v1/memory-stats
```

`GET /health` returns:

```json
{
  "status": "ok",
  "version": "0.6.1",
  "api_schema_version": "1",
  "app_version": "0.6.1",
  "features": ["chat", "sse_events", "remote_auth", "static_spa"]
}
```

## Events

```bash
# Subscribe to all events
curl -N http://localhost:3000/api/v1/events

# Filter by session
curl -N "http://localhost:3000/api/v1/events?session_id=SESSION_ID"
```

SSE event names mirror the desktop GUI event names:

| Event | Payload |
|-------|---------|
| `chat:started` | `{ "run_id": "...", "session_id": "..." }` |
| `chat:progress` | Turn event JSON from `y-service` |
| `chat:complete` | Final turn payload |
| `chat:error` | `{ "run_id": "...", "session_id": "...", "error": "..." }` |
| `chat:AskUser` | User-interaction request with `interaction_id` |
| `chat:PermissionRequest` | Tool permission request with `request_id` |
| `session:title_updated` | Generated title update |
| `diagnostics:event` | Provider, tool, and agent diagnostics |
| `kb:batch_progress` | Knowledge batch ingest progress |
| `kb:entry_ingested` | Knowledge entry ingest completion |

## Chat

```bash
# Synchronous single turn
curl -X POST http://localhost:3000/api/v1/chat \
  -H "Content-Type: application/json" \
  -d '{"message": "Hello"}'

# Async turn, progress arrives through SSE
curl -X POST http://localhost:3000/api/v1/chat/send \
  -H "Content-Type: application/json" \
  -d '{
    "message": "Summarize this project",
    "session_id": "SESSION_ID",
    "provider_id": "openai-main",
    "knowledge_collections": ["docs"],
    "thinking_effort": "high",
    "plan_mode": "auto",
    "mcp_mode": "manual",
    "mcp_servers": ["filesystem"]
  }'

# Image-generation mode
curl -X POST http://localhost:3000/api/v1/chat/send \
  -H "Content-Type: application/json" \
  -d '{
    "message": "Generate a clean product mockup",
    "request_mode": "image_generation",
    "image_generation_options": {
      "max_images": 2,
      "size": "1024x1024",
      "watermark": true
    }
  }'
```

Async start response:

```json
{
  "session_id": "abc123",
  "run_id": "run-uuid"
}
```

Chat control endpoints:

```bash
curl -X POST http://localhost:3000/api/v1/chat/cancel \
  -H "Content-Type: application/json" \
  -d '{"run_id": "RUN_ID"}'

curl -X POST http://localhost:3000/api/v1/chat/undo \
  -H "Content-Type: application/json" \
  -d '{"session_id": "SESSION_ID", "checkpoint_id": "CP_ID"}'

curl -X POST http://localhost:3000/api/v1/chat/resend \
  -H "Content-Type: application/json" \
  -d '{"session_id": "SESSION_ID", "checkpoint_id": "CP_ID"}'

curl http://localhost:3000/api/v1/chat/checkpoints/SESSION_ID

curl -X POST http://localhost:3000/api/v1/chat/find-checkpoint \
  -H "Content-Type: application/json" \
  -d '{"session_id": "SESSION_ID", "user_message_content": "hello", "message_id": "MSG_ID"}'

curl http://localhost:3000/api/v1/chat/messages-with-status/SESSION_ID

curl -X POST http://localhost:3000/api/v1/chat/restore-branch \
  -H "Content-Type: application/json" \
  -d '{"session_id": "SESSION_ID", "checkpoint_id": "CP_ID"}'

curl -X POST http://localhost:3000/api/v1/chat/compact/SESSION_ID

curl http://localhost:3000/api/v1/chat/last-turn-meta/SESSION_ID
```

Human-in-the-loop endpoints:

```bash
# Answer a chat:AskUser event
curl -X POST http://localhost:3000/api/v1/chat/answer-question \
  -H "Content-Type: application/json" \
  -d '{"interaction_id": "INTERACTION_ID", "answers": {"choice": "yes"}}'

# Answer a chat:PermissionRequest event
curl -X POST http://localhost:3000/api/v1/chat/answer-permission \
  -H "Content-Type: application/json" \
  -d '{"request_id": "REQUEST_ID", "decision": "approve"}'
```

`decision` is one of `approve`, `deny`, or `allow_all_for_session`.

## Sessions

```bash
curl http://localhost:3000/api/v1/sessions
curl "http://localhost:3000/api/v1/sessions?state=Archived"
curl "http://localhost:3000/api/v1/sessions?agent_id=AGENT_ID"

curl -X POST http://localhost:3000/api/v1/sessions \
  -H "Content-Type: application/json" \
  -d '{"title": "My Project", "agent_id": "default"}'

curl http://localhost:3000/api/v1/sessions/SESSION_ID
curl -X DELETE http://localhost:3000/api/v1/sessions/SESSION_ID
curl http://localhost:3000/api/v1/sessions/SESSION_ID/messages
curl "http://localhost:3000/api/v1/sessions/SESSION_ID/messages?last=5"

curl -X POST http://localhost:3000/api/v1/sessions/SESSION_ID/archive

curl -X POST http://localhost:3000/api/v1/sessions/SESSION_ID/truncate \
  -H "Content-Type: application/json" \
  -d '{"keep_count": 10}'

curl http://localhost:3000/api/v1/sessions/SESSION_ID/context-reset
curl -X PUT http://localhost:3000/api/v1/sessions/SESSION_ID/context-reset \
  -H "Content-Type: application/json" \
  -d '{"index": 5}'

curl http://localhost:3000/api/v1/sessions/SESSION_ID/custom-prompt
curl -X PUT http://localhost:3000/api/v1/sessions/SESSION_ID/custom-prompt \
  -H "Content-Type: application/json" \
  -d '{"prompt": "You are a concise coding assistant."}'

curl -X POST http://localhost:3000/api/v1/sessions/SESSION_ID/fork \
  -H "Content-Type: application/json" \
  -d '{"message_index": 4, "title": "Forked conversation"}'

curl -X PUT http://localhost:3000/api/v1/sessions/SESSION_ID/rename \
  -H "Content-Type: application/json" \
  -d '{"title": "New Title"}'
```

## Agents

```bash
curl http://localhost:3000/api/v1/agents
curl http://localhost:3000/api/v1/agents/AGENT_ID
curl http://localhost:3000/api/v1/agents/AGENT_ID/source

curl -X POST http://localhost:3000/api/v1/agents/parse-toml \
  -H "Content-Type: application/json" \
  -d '{"toml_content": "id = \"writer\"\nname = \"Writer\"\n..."}'

curl -X PUT http://localhost:3000/api/v1/agents/AGENT_ID \
  -H "Content-Type: application/json" \
  -d '{"toml_content": "id = \"writer\"\nname = \"Writer\"\n..."}'

curl -X POST http://localhost:3000/api/v1/agents/AGENT_ID/reset
curl -X POST http://localhost:3000/api/v1/agents/reload
curl http://localhost:3000/api/v1/agents/tools
curl http://localhost:3000/api/v1/agents/prompt-sections

curl -X POST http://localhost:3000/api/v1/agents/translate \
  -H "Content-Type: application/json" \
  -d '{"text": "Hello world"}'
```

## Configuration

```bash
curl http://localhost:3000/api/v1/config
curl http://localhost:3000/api/v1/config/providers

curl -X PUT http://localhost:3000/api/v1/config/providers \
  -H "Content-Type: application/json" \
  -d '{"content": "[[providers]]\nid = \"openai-main\"\n..."}'

curl -X POST http://localhost:3000/api/v1/config/reload

curl -X POST http://localhost:3000/api/v1/providers/test \
  -H "Content-Type: application/json" \
  -d '{
    "provider_type": "openai",
    "model": "gpt-4o",
    "api_key": "",
    "api_key_env": "OPENAI_API_KEY",
    "probe_mode": "auto"
  }'

curl -X POST http://localhost:3000/api/v1/providers/list-models \
  -H "Content-Type: application/json" \
  -d '{
    "base_url": "https://api.openai.com/v1",
    "api_key": "",
    "api_key_env": "OPENAI_API_KEY"
  }'
```

Config sections: `providers`, `storage`, `session`, `runtime`, `hooks`,
`tools`, `guardrails`, `browser`, and `knowledge`.

MCP and prompt files:

```bash
curl http://localhost:3000/api/v1/config/mcp
curl -X PUT http://localhost:3000/api/v1/config/mcp \
  -H "Content-Type: application/json" \
  -d '{"mcpServers": {}}'

curl http://localhost:3000/api/v1/config/prompts
curl http://localhost:3000/api/v1/config/prompts/system.txt
curl -X PUT http://localhost:3000/api/v1/config/prompts/system.txt \
  -H "Content-Type: application/json" \
  -d '{"content": "You are a helpful assistant."}'
curl http://localhost:3000/api/v1/config/prompts/system.txt/default
```

## Workspaces

```bash
curl http://localhost:3000/api/v1/workspaces

curl -X POST http://localhost:3000/api/v1/workspaces \
  -H "Content-Type: application/json" \
  -d '{"name": "Project", "path": "/srv/project"}'

curl -X PUT http://localhost:3000/api/v1/workspaces/WS_ID \
  -H "Content-Type: application/json" \
  -d '{"name": "Renamed", "path": "/srv/project"}'

curl -X DELETE http://localhost:3000/api/v1/workspaces/WS_ID
curl http://localhost:3000/api/v1/workspaces/session-map

curl -X POST http://localhost:3000/api/v1/workspaces/assign \
  -H "Content-Type: application/json" \
  -d '{"workspace_id": "WS_ID", "session_id": "SESSION_ID"}'

curl -X POST http://localhost:3000/api/v1/workspaces/unassign \
  -H "Content-Type: application/json" \
  -d '{"session_id": "SESSION_ID"}'
```

## Skills

```bash
curl http://localhost:3000/api/v1/skills
curl http://localhost:3000/api/v1/skills/SKILL_NAME
curl -X DELETE http://localhost:3000/api/v1/skills/SKILL_NAME

curl -X PUT http://localhost:3000/api/v1/skills/SKILL_NAME/enabled \
  -H "Content-Type: application/json" \
  -d '{"enabled": true}'

curl -X POST http://localhost:3000/api/v1/skills/import \
  -H "Content-Type: application/json" \
  -d '{"path": "/srv/skills/writer.toml", "sanitize": true}'

curl http://localhost:3000/api/v1/skills/SKILL_NAME/files
curl http://localhost:3000/api/v1/skills/SKILL_NAME/files/root.md

curl -X PUT http://localhost:3000/api/v1/skills/SKILL_NAME/files/root.md \
  -H "Content-Type: application/json" \
  -d '{"content": "Updated skill guidance."}'
```

`skill_open_folder` is desktop-only because a browser cannot reveal a folder in
the user's file manager.

## Knowledge

```bash
curl http://localhost:3000/api/v1/knowledge/collections

curl -X POST http://localhost:3000/api/v1/knowledge/collections \
  -H "Content-Type: application/json" \
  -d '{"name": "docs", "description": "Project documentation"}'

curl -X DELETE http://localhost:3000/api/v1/knowledge/collections/docs

curl -X POST http://localhost:3000/api/v1/knowledge/collections/docs/rename \
  -H "Content-Type: application/json" \
  -d '{"new_name": "documentation"}'

curl http://localhost:3000/api/v1/knowledge/collections/docs/entries
curl "http://localhost:3000/api/v1/knowledge/entries/ENTRY_ID?resolution=l1"
curl -X DELETE http://localhost:3000/api/v1/knowledge/entries/ENTRY_ID

curl -X PATCH http://localhost:3000/api/v1/knowledge/entries/ENTRY_ID/metadata \
  -H "Content-Type: application/json" \
  -d '{"document_type": "spec", "tags": ["important"]}'

curl -X POST http://localhost:3000/api/v1/knowledge/search \
  -H "Content-Type: application/json" \
  -d '{"query": "provider configuration", "domain": "docs", "limit": 5}'

curl -X POST http://localhost:3000/api/v1/knowledge/ingest \
  -H "Content-Type: application/json" \
  -d '{
    "collection": "docs",
    "source": "/srv/project/README.md",
    "domain": "project",
    "use_llm_summary": false,
    "extract_metadata": true
  }'

curl -X POST http://localhost:3000/api/v1/knowledge/ingest-batch \
  -H "Content-Type: application/json" \
  -d '{"collection": "docs", "sources": ["/srv/a.md", "/srv/b.md"]}'

curl -X POST http://localhost:3000/api/v1/knowledge/expand-folder \
  -H "Content-Type: application/json" \
  -d '{"path": "/srv/project/docs"}'

curl http://localhost:3000/api/v1/knowledge/stats
```

Path-based knowledge ingest uses paths reachable by the y-web server process.
The browser UI should use browser file selection for local client files.

## Attachments

```bash
# Read server-side image files as base64 attachment records
curl -X POST http://localhost:3000/api/v1/attachments/read \
  -H "Content-Type: application/json" \
  -d '{"paths": ["/srv/image.png"]}'

# Upload one or more image files as multipart form data
curl -X POST http://localhost:3000/api/v1/attachments/upload \
  -F "file=@/path/to/image.png"
```

Supported image extensions: `png`, `jpg`, `jpeg`, `gif`, and `webp`. Each file
is limited to 20 MB.

## Automation

### Workflows

```bash
curl http://localhost:3000/api/v1/workflows

curl -X POST http://localhost:3000/api/v1/workflows \
  -H "Content-Type: application/json" \
  -d '{
    "name": "daily-report",
    "definition": "fetch -> summarize -> send",
    "format": "expression_dsl",
    "description": "Daily report workflow"
  }'

curl http://localhost:3000/api/v1/workflows/WORKFLOW_ID

curl -X PUT http://localhost:3000/api/v1/workflows/WORKFLOW_ID \
  -H "Content-Type: application/json" \
  -d '{"definition": "fetch -> summarize", "format": "expression_dsl"}'

curl -X DELETE http://localhost:3000/api/v1/workflows/WORKFLOW_ID

curl -X POST http://localhost:3000/api/v1/workflows/validate \
  -H "Content-Type: application/json" \
  -d '{"definition": "fetch -> summarize", "format": "expression_dsl"}'

curl http://localhost:3000/api/v1/workflows/WORKFLOW_ID/dag
curl -X POST http://localhost:3000/api/v1/workflows/WORKFLOW_ID/execute
```

### Schedules

```bash
curl http://localhost:3000/api/v1/schedules

curl -X POST http://localhost:3000/api/v1/schedules \
  -H "Content-Type: application/json" \
  -d '{
    "name": "daily-report",
    "workflow_id": "WORKFLOW_ID",
    "trigger": {
      "type": "cron",
      "expression": "0 9 * * *",
      "timezone": "UTC"
    },
    "parameter_values": {}
  }'

curl http://localhost:3000/api/v1/schedules/SCHEDULE_ID

curl -X PUT http://localhost:3000/api/v1/schedules/SCHEDULE_ID \
  -H "Content-Type: application/json" \
  -d '{"name": "morning-report"}'

curl -X DELETE http://localhost:3000/api/v1/schedules/SCHEDULE_ID
curl -X POST http://localhost:3000/api/v1/schedules/SCHEDULE_ID/pause
curl -X POST http://localhost:3000/api/v1/schedules/SCHEDULE_ID/resume
curl http://localhost:3000/api/v1/schedules/SCHEDULE_ID/executions
curl http://localhost:3000/api/v1/schedules/executions/EXECUTION_ID
curl -X POST http://localhost:3000/api/v1/schedules/SCHEDULE_ID/trigger
```

Supported trigger variants use `type`: `cron`, `interval`, `event`, or
`one_time`.

## Background Tasks

Long-running tool executions can expose process-scoped background task
handles. The Web API mirrors the GUI controls.

```bash
curl http://localhost:3000/api/v1/sessions/SESSION_ID/background-tasks

curl -X POST http://localhost:3000/api/v1/sessions/SESSION_ID/background-tasks/PROCESS_ID/poll \
  -H "Content-Type: application/json" \
  -d '{"yield_time_ms": 50, "max_output_bytes": 4096}'

curl -X POST http://localhost:3000/api/v1/sessions/SESSION_ID/background-tasks/PROCESS_ID/write \
  -H "Content-Type: application/json" \
  -d '{"input": "y\n", "yield_time_ms": 50}'

curl -X POST http://localhost:3000/api/v1/sessions/SESSION_ID/background-tasks/PROCESS_ID/kill \
  -H "Content-Type: application/json" \
  -d '{"yield_time_ms": 50}'
```

## Observability And Diagnostics

```bash
curl http://localhost:3000/api/v1/observability/snapshot
curl "http://localhost:3000/api/v1/observability/history?since=2026-05-04T00:00:00Z&until=2026-05-04T23:59:59Z"

curl http://localhost:3000/api/v1/diagnostics/traces
curl "http://localhost:3000/api/v1/diagnostics/traces?session_id=SESSION_ID&limit=10"
curl http://localhost:3000/api/v1/diagnostics/traces/TRACE_UUID
curl "http://localhost:3000/api/v1/diagnostics/sessions/SESSION_ID?limit=50"
curl http://localhost:3000/api/v1/diagnostics/subagents

curl -X DELETE http://localhost:3000/api/v1/diagnostics/sessions/SESSION_ID
curl -X DELETE http://localhost:3000/api/v1/diagnostics
```

## Rewind

```bash
curl http://localhost:3000/api/v1/rewind/SESSION_ID/points

curl -X POST http://localhost:3000/api/v1/rewind/SESSION_ID/execute \
  -H "Content-Type: application/json" \
  -d '{"target_message_id": "MSG_ID"}'

curl -X POST http://localhost:3000/api/v1/rewind/SESSION_ID/restore-files \
  -H "Content-Type: application/json" \
  -d '{"target_message_id": "MSG_ID"}'
```

## Bot Webhooks

Bot adapters are mounted on y-web and share the same service container.
Configure them in `config/bots.toml`.

```bash
curl -X POST http://localhost:3000/api/v1/bots/feishu/webhook
curl -X POST http://localhost:3000/api/v1/bots/discord/webhook
```

## Error Format

Errors return JSON:

```json
{
  "error": "not_found",
  "message": "session abc123 not found"
}
```

| HTTP status | Error code |
|-------------|------------|
| 400 | `bad_request` |
| 401 | `unauthorized` |
| 404 | `not_found` |
| 500 | `internal_error` |

## Limits

- Rate limiting is not implemented yet.
- Request body size limits are endpoint-specific today; attachment files are
  limited to 20 MB.
- Desktop-only features such as native window controls and revealing a folder
  in the file manager are intentionally not exposed to browsers.
