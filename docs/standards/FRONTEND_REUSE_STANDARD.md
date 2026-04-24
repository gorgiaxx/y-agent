# y-agent Shared Frontend Reuse Standard

**Version**: v0.2
**Created**: 2026-04-24
**Status**: Active

---

## 1. Purpose

This standard defines how the React/Vite frontend is shared between the
desktop Tauri application and the Web API server.

The goal is one maintained product UI, not separate GUI and Web UI
implementations. All future frontend, Tauri GUI, and y-web UI work must follow
this standard.

---

## 2. Authoritative Decision

The y-agent frontend is a shared UI application with two host environments:

| Host | Owner | Backend transport | Runtime capabilities |
|------|-------|-------------------|----------------------|
| Desktop | `crates/y-gui/src-tauri` | Tauri IPC and Tauri events | Native dialogs, window controls, filesystem reveal, bundled resources |
| Web | `crates/y-web` | HTTP REST and SSE | Browser upload, remote access, bearer auth, static SPA serving |

The frontend must not be forked into separate desktop and Web UI codebases.
Feature work must extend the shared UI and adapt host differences through
transport and platform capability layers.

---

## 3. Repository Layout Policy

### 3.1 Current Layout

The current shared frontend package remains under:

```text
crates/y-gui/
  src/          # Shared React/Vite/TypeScript UI
  src-tauri/    # Tauri Rust shell and desktop command host
```

This layout is acceptable while the frontend is still tightly coupled to the
desktop shell and existing scripts.

### 3.2 Preferred Future Layout

If the frontend is separated from the Tauri crate, use a dedicated app package:

```text
apps/y-ui/
  src/          # Shared React/Vite/TypeScript UI
  package.json
  vite.config.ts

crates/y-gui/src-tauri/
  tauri.conf.json
  src/

crates/y-web/
  src/
```

Do not place the frontend package directly at the repository root. The root is
the Rust workspace and project coordination surface. A bare root-level frontend
package would blur Cargo, Node, release, and documentation ownership.

### 3.3 Migration Rule

A directory migration is allowed only when it is a dedicated change with:

1. Updated Tauri build paths.
2. Updated y-web static asset instructions.
3. Updated frontend quality gates.
4. No behavior changes mixed into the move.

---

## 4. Layering Rules

### 4.1 Shared UI

Shared React components, hooks, styles, and view state belong in the frontend
package. They must depend on abstract client interfaces, not directly on Tauri
or y-web implementation details.

### 4.2 Transport Layer

All backend calls from shared UI code must go through the frontend transport
interface:

```text
UI hooks/components -> Transport -> TauriTransport or HttpTransport
```

The shared UI must not call `fetch`, `EventSource`, or Tauri `invoke` directly
unless the file is part of the transport or platform adapter layer.

### 4.3 Platform Layer

Host-specific non-command features must go through a platform abstraction:

```text
UI hooks/components -> Platform -> TauriPlatform or WebPlatform
```

Examples:

| Capability | Desktop behavior | Web behavior |
|------------|------------------|--------------|
| File selection | Native dialog returns local paths | Browser file picker returns files or uploaded server paths |
| Reveal in file manager | Native filesystem reveal | Disabled or replaced with a server-side action |
| Open URL | Tauri opener plugin | `window.open` |
| Window controls | Native window commands | Hidden or disabled |
| App version | Tauri app metadata | API status endpoint |

New host-specific behavior must add or update a named platform capability.

### 4.4 Presentation Boundary

Business logic must remain in `y-service` or lower crates. The frontend,
`y-web`, and `crates/y-gui/src-tauri` are presentation layers. They may adapt
input/output shapes, handle rendering, and bridge events, but must not own
domain decisions.

---

## 5. Protocol Contract Rules

### 5.1 Single Product Contract

The shared frontend depends on a single product-level command contract. Tauri
commands and y-web endpoints may use different wire protocols, but they must
provide equivalent behavior for every shared UI command.

### 5.2 HTTP Adapter Responsibility

`HttpTransport` owns conversion from frontend command arguments to y-web
requests. This includes:

- Command name to HTTP method/path mapping.
- Camel-case frontend arguments to snake-case API fields.
- Path and query parameter encoding.
- JSON body shaping.
- HTTP error normalization.
- SSE event adaptation to the same callback shape used by Tauri events.

The shared UI must not contain y-web-specific request shaping.

### 5.3 Tauri Adapter Responsibility

`TauriTransport` owns the Tauri IPC boundary. Tauri command handlers may accept
frontend-shaped arguments, but their domain behavior must be delegated to
`y-service` or lower crates.

### 5.4 No Silent No-Ops

Host differences must be explicit. A command must not silently become a no-op
in one host unless all of the following are true:

1. The command is lifecycle-only or purely cosmetic.
2. The UI remains correct when nothing happens.
3. A test documents the behavior.

For user-visible operations, use capability gating, a disabled control, a Web
alternative, or a clear error.

### 5.5 Version Compatibility

y-web must expose enough version information for the shared frontend to detect
API compatibility before enabling features. The preferred shape is:

```json
{
  "api_schema_version": "1",
  "app_version": "0.5.5",
  "features": ["chat", "sse", "attachments_upload"]
}
```

Until a dedicated endpoint exists, `/api/v1/status` or `/health` may carry this
metadata.

---

## 6. Capability Gating

Every feature that is not supported equally in desktop and Web must have an
explicit capability entry.

Minimum capability groups:

| Capability | Required for |
|------------|--------------|
| `nativeWindowControls` | Minimize, maximize, close, custom decorations |
| `nativeFilePaths` | Reading local paths directly from the frontend |
| `browserFileUpload` | Uploading or encoding browser-selected files through a Web-safe flow |
| `revealFileManager` | Opening a file or folder in the OS file manager |
| `skillImportFromPath` | Importing a skill from a path reachable by the active backend |
| `knowledgeIngestFromPath` | Ingesting knowledge from a path reachable by the active backend |
| `remoteAuth` | Bearer-token protected y-web access |
| `sseEvents` | Real-time browser event stream |

UI components must branch on capabilities, not on ad hoc environment checks.
Environment detection may exist inside the platform adapter.

---

## 7. Attachment and File Handling

Desktop and Web file handling are intentionally different:

| Flow | Desktop | Web |
|------|---------|-----|
| Chat image attachment | Select local path, read via Tauri command | Select browser `File`, upload or encode through Web-safe flow |
| Skill import | Local path import through Tauri/y-service | Upload or server-side path import only when explicitly supported |
| Knowledge folder ingest | Native directory selection | Disabled unless y-web exposes a safe server-side folder expansion flow |

Browser UI must not assume it can read arbitrary local filesystem paths.

---

## 8. Build and Serving Rules

### 8.1 Desktop Build

The Tauri host builds and embeds the shared frontend as desktop assets.

### 8.2 Web Build

The Web host serves the shared frontend as static SPA assets through y-web.
The current output directory is:

```text
crates/y-gui/dist-web/
```

The y-web server must serve API routes before SPA fallback routes, so
`/api/v1/*` and `/health` never resolve to `index.html`.

### 8.3 Static Asset Ownership

y-web serves built assets. It must not import frontend source files or depend on
Node tooling at Rust compile time.

---

## 9. Testing Requirements

Frontend behavior changes must follow the existing frontend TDD rules.

In addition, shared frontend work must add or update tests for:

1. Transport command mapping when y-web endpoints are affected.
2. Capability-gated rendering when a feature differs by host.
3. SSE event adaptation when streaming payloads change.
4. File/attachment behavior when browser and desktop paths diverge.
5. Version or feature negotiation when API compatibility is involved.

Completion gates for frontend-only shared UI changes:

```bash
cd crates/y-gui
npm test
npm run lint
npm run build
npm run build:web
```

Completion gates for mixed frontend plus Rust contract changes:

```bash
cargo fmt --all
cargo clippy --fix --allow-dirty --workspace -- -D warnings
cargo clippy --workspace -- -D warnings
cargo check --workspace
cargo doc --workspace --no-deps

cd crates/y-gui
npm test
npm run lint
npm run build
npm run build:web
```

---

## 10. Change Review Checklist

Before merging any shared frontend, y-web, or Tauri GUI change, verify:

- The feature is implemented once in shared UI code.
- Host differences are isolated to transport or platform adapters.
- y-web and Tauri command behavior remain equivalent for shared commands.
- Browser-only and desktop-only capabilities are explicit.
- No user-visible action silently no-ops in one host.
- API field naming is adapted at the transport boundary.
- Streaming events have the same semantic payload in both hosts.
- Tests cover contract and capability changes.
- Documentation or standards are updated when a new host capability is added.

---

## 11. Current Compliance Baseline

The shared frontend baseline is:

1. `HttpTransport` maps shared command names to y-web endpoints, including
   path/query encoding, camel-case to snake-case request bodies, and response
   normalization back to Tauri-shaped payloads.
2. `SseAdapter` unwraps y-web SSE payloads to the same semantic payload shape
   used by Tauri events.
3. `Platform` exposes named host capabilities. Shared UI components must use
   these capabilities instead of direct environment checks.
4. Browser chat attachments use browser `File` objects encoded into shared
   attachment payloads. Browser code must not pass local filenames as backend
   paths.
5. y-web `/health` exposes `api_schema_version`, `app_version`, and `features`.
6. y-web exposes `/api/v1/memory-stats` for observability parity with the Tauri
   command.

No mandatory frontend fork or root-level package move is required for the
current architecture.

## 12. Optional Future Work

Optional future work must remain separate from behavior changes:

1. Move `crates/y-gui/src` to `apps/y-ui` only after a dedicated migration plan
   updates Tauri build paths, y-web static serving, and frontend gates.
2. Add a startup compatibility banner if `/health.api_schema_version` is newer
   than the frontend contract version.
