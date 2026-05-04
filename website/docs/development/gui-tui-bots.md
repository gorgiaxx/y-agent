# GUI, TUI, and Bots Development

This guide covers the local development loop for the presentation surfaces that
sit on top of `y-service`: the Tauri GUI, the terminal TUI, and bot adapters.

Presentation crates should stay thin. UI code may adapt input and output, but
business behavior belongs in `y-service` or lower crates.

## Prerequisites

- Rust stable toolchain with `rustfmt` and `clippy`.
- Node.js 22 or newer for the shared React frontend.
- Platform dependencies required by Tauri. On Linux this includes
  `libwebkit2gtk-4.1-dev`, `libappindicator3-dev`, `librsvg2-dev`, `patchelf`,
  `libssl-dev`, `libsqlite3-dev`, and GTK development packages.
- Provider credentials in the normal y-agent configuration files when testing
  real chat flows.

## Desktop GUI

The desktop GUI is a Tauri shell in `crates/y-gui/src-tauri` that hosts the
shared React application in `crates/y-gui/src`.

Install frontend dependencies:

```bash
cd crates/y-gui
npm install
```

Run the desktop app in development mode:

```bash
cd crates/y-gui
npx @tauri-apps/cli dev
```

The Tauri config runs `npm run dev` automatically through
`beforeDevCommand`, so one command is enough for the normal desktop loop. If
you only need the browser-rendered React surface, run:

```bash
cd crates/y-gui
npm run dev
```

Build the desktop frontend bundle:

```bash
cd crates/y-gui
npm run build
```

Build the Tauri desktop bundle:

```bash
cd crates/y-gui
npx @tauri-apps/cli build
```

### GUI Debugging

- Use the browser devtools attached to the Tauri webview for React state,
  console output, CSS, and network-like diagnostics.
- Use `VITE_LOG_LEVEL=debug` when the issue is in frontend logging paths.
- Use `RUST_LOG=debug` when the issue crosses the Tauri command boundary.
- Keep host-specific behavior behind `platform` capabilities in
  `crates/y-gui/src/lib/platform.ts`.
- Keep backend calls behind `Transport`. Shared UI code should not call
  Tauri `invoke`, `fetch`, or `EventSource` directly.

Focused frontend tests are useful while iterating:

```bash
cd crates/y-gui
npm test -- --run src/__tests__/webApiParity.test.ts
```

Before finishing GUI changes, run the full GUI gate:

```bash
cd crates/y-gui
npm test
npm run lint
npm run build
npm run build:web
```

## Terminal TUI

The TUI is part of the `y-cli` crate and is enabled by the default `tui`
feature.

Run the TUI:

```bash
cargo run --bin y-agent -- tui
```

Resume or fork an existing session:

```bash
cargo run --bin y-agent -- resume
cargo run --bin y-agent -- resume <session-id-prefix>
cargo run --bin y-agent -- fork <session-id-prefix> --label experiment
```

Debug a TUI session with tracing:

```bash
cargo run --bin y-agent -- --log-level debug tui
```

TUI source is organized under `crates/y-cli/src/tui`. Prefer focused unit tests
for renderers, state transitions, key handling, and command handlers. If the
terminal is left in an invalid state during a failed debug run, run `reset` in
the shell before starting another session.

Before finishing TUI changes, run the Rust gate:

```bash
cargo fmt --all
cargo clippy --workspace -- -D warnings
cargo check --workspace
cargo doc --workspace --no-deps
```

## Bot Adapters

Bot transports are implemented in `y-bot`, exposed through `y-web`, and routed
through `y-service::BotService`.

Current platform wiring:

| Platform | Development entry point | Runtime route |
|----------|--------------------------|---------------|
| Discord Gateway | `y_bot::discord_gateway` | Started by `y-agent serve` when Discord config exists |
| Discord webhook | `crates/y-web/src/routes/bots.rs` | `POST /api/v1/bots/discord/webhook` |
| Feishu webhook | `crates/y-web/src/routes/bots.rs` | `POST /api/v1/bots/feishu/webhook` |
| Telegram | `y-bot` interface | Reserved for implementation |

Bot credentials are loaded from `bots.toml` in the user config directory. For
repo-local development, pass the project config directory explicitly:

```bash
cargo run --bin y-agent -- --user-config-dir config serve --host 127.0.0.1 --port 3000
```

Minimal local `config/bots.toml` shape:

```toml
[discord]
token = "discord-bot-token"
application_id = "discord-application-id"
public_key = "discord-ed25519-public-key"

[feishu]
app_id = "feishu-app-id"
app_secret = "feishu-app-secret"
encrypt_key = ""
verification_token = ""
domain = "feishu"
```

For webhook testing, expose the local server with a tunnel and register the
public tunnel URL in the platform developer console:

```text
https://example-tunnel.test/api/v1/bots/discord/webhook
https://example-tunnel.test/api/v1/bots/feishu/webhook
```

Use service-specific tracing when debugging bot flows:

```bash
RUST_LOG=y_web=debug,y_bot=debug,y_service=debug \
  cargo run --bin y-agent -- --user-config-dir config serve --host 127.0.0.1 --port 3000
```

When changing bot behavior, test the smallest layer first:

1. Parser and signature logic in `y-bot`.
2. Webhook routing and response status in `y-web`.
3. Session and response behavior through `BotService`.
4. End-to-end platform callback only after unit and route tests pass.

## Change Checklist

Use this checklist when a change touches these surfaces:

1. Add or update tests before behavior changes.
2. Keep domain decisions out of presentation crates.
3. Update shared Web API or frontend contract tests when commands cross host
   boundaries.
4. Run all applicable Rust and frontend quality gates.
5. Update user-facing guide docs if a command, config shape, or route changes.
