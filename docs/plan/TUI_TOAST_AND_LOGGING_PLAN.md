# TUI Toast/Flash Messages and Persistent File Logging

## Context

The TUI (`crates/y-cli/src/tui/`) currently has no mechanism for transient notifications. Errors and log messages either corrupt the TUI layout (when `tracing::warn!` writes to stderr in raw mode) or get permanently injected into the chat history as `ChatMessage` with `MessageRole::System`. There is also no file-based logging — all tracing output goes exclusively to stderr (`main.rs:75-81`). The user needs:

1. Toast/Flash overlay in TUI that auto-dismisses
2. Persistent file logging so messages survive after toast disappears
3. Tracing redirection away from stderr in TUI mode

---

## 1. Log Directory Convention

**Path**: `$XDG_STATE_HOME/y-agent/log/` (defaults to `~/.local/state/y-agent/log/`)

`XDG_STATE_HOME` is the correct XDG location for "state data that persists across restarts but is not important or portable enough for `$XDG_DATA_HOME`" — log files fit this definition exactly.

**Files to change**:
- `crates/y-cli/src/config.rs` — add `dirs_log()` helper alongside existing `dirs_user_config()` (line 354)
- `crates/y-cli/src/config.rs` — add `log_dir: Option<String>` and `log_retention_days: u32` (default 7) to `YAgentConfig`
- `crates/y-cli/src/commands/init.rs` — add log dir to `ensure_directories()` (line 493)

**`dirs_log()` logic**:
```
1. Check $XDG_STATE_HOME env → $XDG_STATE_HOME/y-agent/log/
2. Fallback: ~/.local/state/y-agent/log/
3. Config override: YAgentConfig.log_dir takes precedence
```

**Log cleanup**: `cleanup_old_logs(dir, retention_days)` runs at startup, deletes `y-agent.*.log` files older than retention period.

---

## 2. Persistent File Logging

**New dependency**: `tracing-appender = "0.2"` (add to workspace `Cargo.toml` and `y-cli/Cargo.toml`)

**Refactor** `main.rs:75-81` from monolithic `fmt().init()` to layered subscriber:

```
Registry
  + EnvFilter (from config.log_level / RUST_LOG)
  + File Layer      — always active, daily rotation → y-agent.YYYY-MM-DD.log
  + Stderr Layer    — active only in NON-TUI mode
  + Toast Bridge    — active only in TUI mode (warn/error → mpsc channel)
```

The command variant is already known before tracing init (CLI parsed at line 50), so we can conditionally include layers.

---

## 3. TUI Toast System

### 3a. State (`crates/y-cli/src/tui/state.rs`)

```rust
pub enum ToastLevel { Info, Success, Warning, Error }

pub struct Toast {
    pub message: String,
    pub level: ToastLevel,
    pub ticks_remaining: u16,  // 250ms per tick
    pub id: u64,
}
```

Add to `AppState`:
```rust
pub toasts: VecDeque<Toast>,
pub toast_counter: u64,
```

Methods: `push_toast()`, `tick_toasts()`, `dismiss_toast(id)`, `dismiss_all_toasts()`

Default durations: Info/Success=3s (12 ticks), Warning=5s (20 ticks), Error=7s (28 ticks). Max concurrent: 5 (oldest evicted).

### 3b. Rendering — new file `overlays/toast.rs`

- Bottom-right corner, stacked upward
- Each toast: 1-2 lines, colored left border (red=Error, yellow=Warning, green=Success, cyan=Info)
- Non-modal (does not capture keyboard input)
- Uses `Clear` widget pattern from existing `overlays/command_palette.rs`

Wired into `draw()` in `tui/mod.rs` after command palette rendering (line 278).

### 3c. Tick-driven lifecycle

In `TuiApp::run()` (`tui/mod.rs`), `AppEvent::Tick` handler (line 205):
1. Drain toast channel (from tracing bridge)
2. Call `state.tick_toasts()` to expire old toasts

### 3d. Integration points

| Current pattern | New pattern |
|----------------|-------------|
| `CommandResult::Error(msg)` → ChatMessage (`mod.rs:298`) | → Toast (Error) |
| `CommandResult::Ok(Some(msg))` → ChatMessage (`mod.rs:289`) | → Toast (Info) |
| `ChatEvent::Error` → replaces streaming msg (`chat_flow.rs:186`) | Keep in chat + emit Warning toast |
| `tracing::warn!` → stderr (corrupts TUI) | → file + toast via bridge |

---

## 4. Tracing-to-Toast Bridge

New file: `crates/y-cli/src/tui/tracing_bridge.rs` (feature-gated behind `tui`)

`ToastBridgeLayer` implements `tracing_subscriber::Layer<S>`:
- `on_event()`: filters WARN/ERROR, formats message, sends `Toast` via `mpsc::UnboundedSender<Toast>`
- `TuiApp` holds the `UnboundedReceiver<Toast>`, drains on tick

Channel created in `main.rs` before subscriber setup; sender → bridge layer, receiver → `TuiApp::new()`.

---

## 5. Files Summary

### New files (2)
| File | Purpose |
|------|---------|
| `crates/y-cli/src/tui/overlays/toast.rs` | Toast overlay rendering |
| `crates/y-cli/src/tui/tracing_bridge.rs` | Custom tracing Layer forwarding warn/error to toast channel |

### Modified files (8)
| File | Changes |
|------|---------|
| `Cargo.toml` (workspace) | Add `tracing-appender = "0.2"` |
| `crates/y-cli/Cargo.toml` | Add `tracing-appender` dep |
| `crates/y-cli/src/config.rs` | `dirs_log()`, `log_dir`/`log_retention_days` config fields |
| `crates/y-cli/src/commands/init.rs` | Log dir in `ensure_directories()` |
| `crates/y-cli/src/main.rs` | Layered tracing subscriber, toast channel creation |
| `crates/y-cli/src/tui/state.rs` | `Toast`, `ToastLevel`, `VecDeque<Toast>`, toast methods |
| `crates/y-cli/src/tui/mod.rs` | Toast receiver field, tick drain, overlay rendering, command error → toast |
| `crates/y-cli/src/tui/overlays/mod.rs` | `pub mod toast;` |

---

## 6. Implementation Phases

**Phase 1: Log Directory + File Logging**
1. Add `tracing-appender` dependency
2. TDD: `dirs_log()` tests → implement `dirs_log()` in config.rs
3. Add `log_dir`, `log_retention_days` to `YAgentConfig`
4. Update `ensure_directories()` to create log dir
5. TDD: `cleanup_old_logs()` tests → implement
6. Refactor main.rs tracing init to layered subscriber (file + conditional stderr)

**Phase 2: Toast State Model** (can parallel with Phase 1)
1. TDD: toast lifecycle tests → implement `Toast`, `ToastLevel` in state.rs
2. Implement `push_toast()`, `tick_toasts()`, `dismiss_toast()`, `dismiss_all_toasts()`

**Phase 3: Toast Rendering**
- Depends on: Phase 2
1. TDD: rendering tests → implement `overlays/toast.rs`
2. Wire into `draw()` and `overlays/mod.rs`

**Phase 4: Tracing Bridge + Integration**
- Depends on: Phase 1 + Phase 2
1. TDD: bridge layer tests → implement `tracing_bridge.rs`
2. Wire channel: main.rs → TuiApp
3. Drain channel on tick in `TuiApp::run()`
4. Migrate command errors and LLM errors to use toasts

---

## 7. Verification

1. `cargo test -p y-cli` — all new and existing tests pass
2. `cargo build -p y-cli` / `cargo build -p y-cli --no-default-features` — both compile
3. Manual: run `y-agent tui`, trigger an error (bad model name), verify:
   - Toast appears bottom-right with red border
   - Toast auto-dismisses after ~7s
   - Log file exists at `~/.local/state/y-agent/log/y-agent.YYYY-MM-DD.log`
   - Log file contains the error message
4. Manual: run `y-agent status` (non-TUI), verify log goes to both stderr and file
5. Verify no stderr output corrupts TUI layout
