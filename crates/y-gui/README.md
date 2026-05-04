# y-gui

Shared React/Vite/TypeScript frontend for y-agent.

This package is used by two hosts:

| Host | Command transport | Event transport |
|------|-------------------|-----------------|
| Tauri desktop | Tauri `invoke` commands | Tauri events |
| Web UI | y-web REST endpoints | y-web SSE |

The shared UI must call backend functionality through `src/lib/transport.ts`
and host-specific capabilities through `src/lib/platform.ts`. UI components
should not call Tauri APIs, `fetch`, or `EventSource` directly unless they are
inside a transport or platform adapter.

## Development

```bash
npm install

# Desktop frontend dev server for Tauri
npm run dev

# Browser-hosted UI against y-web
VITE_API_URL=http://localhost:3000 npm run dev:web
```

Run y-web separately when using `dev:web`:

```bash
y-agent serve
```

## Builds

```bash
# Desktop bundle consumed by Tauri
npm run build

# Browser SPA served by y-web
npm run build:web

# Serve the Web UI through y-agent
cd ../..
y-agent serve --static-dir crates/y-gui/dist-web
```

## Quality Gates

```bash
npm test
npm run lint
npm run build
npm run build:web
```

## Contract Files

| File | Purpose |
|------|---------|
| `src/lib/commandMap.ts` | Tauri command name to y-web endpoint mapping |
| `src/lib/httpTransport.ts` | HTTP/SSE transport implementation |
| `src/lib/tauriTransport.ts` | Tauri command/event transport implementation |
| `src/lib/sseAdapter.ts` | SSE event adapter with Tauri-compatible callback shape |
| `src/lib/platform.ts` | Host capability and file/dialog abstraction |
| `src/__tests__/webApiParity.test.ts` | Guards GUI command to Web API parity |

See `../../docs/standards/FRONTEND_REUSE_STANDARD.md` for the frontend reuse rules.
