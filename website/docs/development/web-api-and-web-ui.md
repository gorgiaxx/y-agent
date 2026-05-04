# Web API and Web UI Development

This guide covers the split Web development loop: `y-web` serves the REST and
SSE API, while the shared React frontend can run separately in browser mode or
be served as static assets by the API server.

## Architecture

The Web stack reuses the same product UI as the desktop app:

```text
Shared React UI
  -> Transport interface
     -> TauriTransport for desktop
     -> HttpTransport for Web API
  -> Platform interface
     -> TauriPlatform for native capabilities
     -> WebPlatform for browser capabilities
```

Important files:

| Area | Path |
|------|------|
| Web API router | `crates/y-web/src/routes` |
| Web API state | `crates/y-web/src/state.rs` |
| CLI server command | `crates/y-cli/src/commands/serve.rs` |
| HTTP command map | `crates/y-gui/src/lib/commandMap.ts` |
| HTTP transport | `crates/y-gui/src/lib/httpTransport.ts` |
| SSE adapter | `crates/y-gui/src/lib/sseAdapter.ts` |
| Platform capabilities | `crates/y-gui/src/lib/platform.ts` |
| Web API guide | `website/docs/guide/web-api.md` |

## Run the Web API

Start the API server without authentication for local-only development:

```bash
cargo run --bin y-agent -- serve --host 127.0.0.1 --port 3000
```

Start the API server with bearer authentication:

```bash
cargo run --bin y-agent -- serve \
  --host 127.0.0.1 \
  --port 3000 \
  --auth-token dev-token
```

Check public health:

```bash
curl http://127.0.0.1:3000/health
```

Check an authenticated endpoint:

```bash
curl -H "Authorization: Bearer dev-token" \
  http://127.0.0.1:3000/api/v1/status
```

SSE clients can authenticate with the same bearer token or with a query token
for browser `EventSource` compatibility:

```text
http://127.0.0.1:3000/api/v1/events?token=dev-token
```

## Run the Web Frontend Separately

The browser-hosted frontend is selected with `VITE_BACKEND=http`.

```bash
cd crates/y-gui
npm install
VITE_API_URL=http://127.0.0.1:3000 npm run dev:web
```

If the API server requires authentication, pass the token at build or dev time:

```bash
cd crates/y-gui
VITE_BACKEND=http \
VITE_API_URL=http://127.0.0.1:3000 \
VITE_API_TOKEN=dev-token \
npm run dev
```

Build the standalone Web frontend:

```bash
cd crates/y-gui
npm run build:web
```

The Web bundle is written to `crates/y-gui/dist-web`.

## Serve the Web UI from y-web

Build the browser bundle, then point `y-agent serve` at it:

```bash
cd crates/y-gui
npm run build:web

cd ../..
cargo run --bin y-agent -- serve \
  --host 127.0.0.1 \
  --port 3000 \
  --static-dir crates/y-gui/dist-web
```

The API server serves known REST and SSE routes first. Unknown paths fall back
to the Web UI so client-side routing works after refresh.

## Add or Change a Web API Capability

Use this workflow when aligning a GUI command with the Web API:

1. Add or update a `y-web` route test that captures the desired HTTP contract.
2. Implement the route as a thin adapter over `y-service` or lower crates.
3. Add the route to the protected or public router intentionally.
4. Update `COMMAND_MAP` when the shared UI needs to call the endpoint.
5. Update `HttpTransport` or `SseAdapter` only when argument shaping, response
   shaping, error normalization, or event adaptation changes.
6. Update frontend contract tests such as `httpTransportContract.test.ts`,
   `sseAdapterContract.test.ts`, and `webApiParity.test.ts`.
7. Update `website/docs/guide/web-api.md` for user-facing API behavior.

`webApiParity.test.ts` guards the shared command surface. A Tauri command used
by the shared UI should either have an HTTP mapping or be explicitly classified
as lifecycle-only or desktop-only.

## Contract Rules

- Shared React components and hooks use `transport.invoke` and `platform`
  capabilities, not direct host APIs.
- `HttpTransport` owns command-to-endpoint mapping, field casing, path
  encoding, query encoding, request bodies, and error normalization.
- `SseAdapter` owns browser SSE subscription behavior.
- Desktop-only operations must be capability-gated or listed as intentional
  exceptions in parity tests.
- Web API handlers must not own domain decisions. Delegate those to
  `y-service`, orchestration, middleware, capabilities, or infrastructure
  crates.

## Useful Test Commands

Run Web API tests:

```bash
cargo test -p y-web 2>&1 | grep -v '^\s*Compiling\|^\s*Running\|^\s*Downloading\|^\s*Downloaded\|^\s*Blocking\|^\s*Finished\|^\s*Doc-tests\|^running\|^test \|^$' | head -200
```

Run frontend contract tests:

```bash
cd crates/y-gui
npm test -- --run src/__tests__/httpTransportContract.test.ts
npm test -- --run src/__tests__/sseAdapterContract.test.ts
npm test -- --run src/__tests__/webApiParity.test.ts
```

Before finishing shared Web work, run both gates:

```bash
cargo fmt --all
cargo clippy --workspace -- -D warnings
cargo check --workspace
cargo doc --workspace --no-deps

cd crates/y-gui
npm test
npm run lint
npm run build
npm run build:web
```
