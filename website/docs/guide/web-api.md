# Web API

HTTP REST API server for y-agent, built on [axum](https://github.com/tokio-rs/axum).

## Quick Start

```bash
# Start the API server (default: http://0.0.0.0:3000)
y-agent serve

# Custom host/port
y-agent serve --host 127.0.0.1 --port 8080
```

## Endpoints

### System

```bash
# Health check (liveness probe)
curl http://localhost:3000/health

# Full system status
curl http://localhost:3000/api/v1/status
```

### Chat

```bash
# Send a message (auto-creates a new session)
curl -X POST http://localhost:3000/api/v1/chat \
  -H "Content-Type: application/json" \
  -d '{"message": "Hello, what can you do?"}'

# Continue an existing session
curl -X POST http://localhost:3000/api/v1/chat \
  -H "Content-Type: application/json" \
  -d '{"message": "Tell me more", "session_id": "SESSION_ID"}'
```

**Response:**
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
# List all sessions
curl http://localhost:3000/api/v1/sessions

# List only active sessions
curl "http://localhost:3000/api/v1/sessions?state=Active"

# Create a new session
curl -X POST http://localhost:3000/api/v1/sessions \
  -H "Content-Type: application/json" \
  -d '{"title": "My Project"}'

# Get session details
curl http://localhost:3000/api/v1/sessions/SESSION_ID

# List messages in a session
curl http://localhost:3000/api/v1/sessions/SESSION_ID/messages

# Archive a session
curl -X POST http://localhost:3000/api/v1/sessions/SESSION_ID/archive

# Branch a session
curl -X POST http://localhost:3000/api/v1/sessions/SESSION_ID/branch \
  -H "Content-Type: application/json" \
  -d '{"label": "experiment-1"}'
```

### Agents

```bash
# List registered agents
curl http://localhost:3000/api/v1/agents
```

### Tools

```bash
# List registered tools
curl http://localhost:3000/api/v1/tools
```

### Diagnostics

```bash
# List recent traces
curl http://localhost:3000/api/v1/diagnostics/traces

# Filter by session
curl "http://localhost:3000/api/v1/diagnostics/traces?session_id=SESSION_ID&limit=10"

# Get trace detail
curl http://localhost:3000/api/v1/diagnostics/traces/TRACE_UUID
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

## Architecture

```
HTTP Client  ->  axum Router  ->  handlers  ->  y-service  ->  domain crates
```

The server is a thin presentation layer. All business logic lives in `y-service::ServiceContainer`.

::: warning Current Limitations
The Web API is currently in development. Authentication, SSE streaming, rate limiting, CORS, and request size limits are not yet implemented.
:::
