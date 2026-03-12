# Web API R&D Plan — y-web crate

**Created**: 2026-03-12
**Status**: Draft

---

## Scope

Add a new `y-web` crate that provides an HTTP REST API server for y-agent, built on **axum** (already chosen in DESIGN_OVERVIEW.md). The server is a thin presentation layer on top of `y-service::ServiceContainer`, exposing the same business logic as CLI/TUI.

## Goals

1. Design OpenAPI 3.1 spec before implementation (API-first approach)
2. Use axum as the web framework (consistent with DESIGN_OVERVIEW.md)
3. Expose core operations: chat, sessions, agents, tools, diagnostics, system status
4. Feature-flagged in workspace — does not affect existing crates
5. TDD: tests written before handler code

## Steps

1. **API design doc** — write `docs/design/web-api-design.md` with endpoint specification
2. **OpenAPI spec** — write `docs/api/openapi.yaml` with full endpoint definitions
3. **Crate scaffolding** — create `crates/y-web/` with Cargo.toml, feature flag
4. **Shared state** — `AppState` wrapping `Arc<ServiceContainer>`
5. **Router** — axum Router with route groups (sessions, chat, agents, tools, diag, health)
6. **Handlers** — one module per domain, delegating to y-service
7. **Error handling** — unified JSON error response type
8. **Tests** — integration tests using axum's `TestClient` approach
9. **Workspace integration** — add to workspace members, DESIGN_OVERVIEW update

## Dependencies

- `y-service` (business logic)
- `axum` + `tower` + `tower-http` (HTTP framework)
- `tokio` (runtime)
- `serde`/`serde_json` (serialization)

## Verification

- `cargo build -p y-web`
- `cargo test -p y-web`
- `cargo clippy -p y-web`
- Manual: `curl` against running server
