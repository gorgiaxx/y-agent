# TUI Client Development Plan

**Version**: v0.1
**Created**: 2026-03-10
**Status**: Draft
**Design References**: `client-commands-design.md` (v0.3), `client-layer-design.md` (v0.2)
**Current Module Plan**: `docs/plan/modules/y-cli.md`

---

## 1. Overview

This plan covers the implementation of the ratatui-based TUI client for y-agent, as described in `client-commands-design.md` (TUI Interaction Design section) and `client-layer-design.md` (Phase 2). The TUI extends the existing `y-cli` crate with a multi-panel terminal interface supporting streaming responses, session/agent management, and the full `/`-command vocabulary.

### Current State

- `y-cli` provides a basic stdin/stdout interactive chat (`commands/chat.rs`).
- No ratatui, crossterm, or TUI-related dependencies exist.
- Slash commands are limited to `/help`, `/status`, `/clear`, `/quit`.
- No `ClientProtocol` trait implementation; chat operates directly on stdin/stdout.
- No streaming support; responses arrive as a single block.

### Target State

- 4-panel TUI layout (Sidebar, Chat, Status Bar, Input Area) with ratatui.
- Focus-based navigation model across panels.
- 4 interaction modes (Normal, Command, Search, Select) with state machine.
- Full `/`-command vocabulary with command palette overlay.
- Token-level streaming with auto-scroll, scroll lock, and frame batching.
- Keyboard-driven workflows for session/agent management.

---

## 2. Implementation Phases

Implementation is divided into **6 phases**, each independently deliverable. Dependencies between phases are noted in the dependency graph (Section 4).

---

### Phase T1: Foundation and Dependencies (Est. 2-3 days)

> **Goal**: Add ratatui/crossterm dependencies, establish the TUI application scaffold, and define the core state model.

#### T1.1 Add TUI Dependencies

##### [MODIFY] [Cargo.toml](file:///Users/gorgias/Projects/y-agent/crates/y-cli/Cargo.toml)

- Add `ratatui` (latest stable) to dependencies
- Add `crossterm` as the backend for ratatui
- Add `tui-textarea` for multi-line input editing
- Add `unicode-width` for correct CJK/emoji rendering width
- Gate all TUI code behind a `tui` feature flag for independent rollback

##### [MODIFY] [Cargo.toml](file:///Users/gorgias/Projects/y-agent/Cargo.toml) (workspace root)

- Add `ratatui`, `crossterm`, `tui-textarea`, `unicode-width` to workspace dependencies

#### T1.2 TUI Application Shell

##### [NEW] [tui/mod.rs](file:///Users/gorgias/Projects/y-agent/crates/y-cli/src/tui/mod.rs)

- `TuiApp` struct: holds terminal handle, application state, event channels
- `run()` entry point: terminal setup (raw mode, alternate screen), main event loop, cleanup on exit
- Graceful shutdown on `Ctrl+D` / `Ctrl+Q` with terminal restore
- Signal handling: catch SIGINT/SIGTERM to ensure terminal cleanup

##### [NEW] [tui/state.rs](file:///Users/gorgias/Projects/y-agent/crates/y-cli/src/tui/state.rs)

- `AppState` struct: captures full TUI application state
  - `focus: PanelFocus` enum (`Input`, `Chat`, `Sidebar`)
  - `mode: InteractionMode` enum (`Normal`, `Command`, `Search`, `Select`)
  - `sidebar_visible: bool`
  - `sidebar_view: SidebarView` enum (`Sessions`, `Agents`)
  - `messages: Vec<ChatMessage>` (conversation transcript)
  - `input_buffer: String`
  - `scroll_offset: usize`
  - `is_streaming: bool`
- State transition methods: `set_focus()`, `set_mode()`, `toggle_sidebar()`

##### [NEW] [tui/events.rs](file:///Users/gorgias/Projects/y-agent/crates/y-cli/src/tui/events.rs)

- `EventLoop`: async event handler combining crossterm key events and server-side `ClientEvent` stream
- Uses `tokio::select!` to multiplex terminal input with server events
- Debounces rapid resize events (50ms window)

#### T1.3 Wire TUI to CLI

##### [MODIFY] [commands/mod.rs](file:///Users/gorgias/Projects/y-agent/crates/y-cli/src/commands/mod.rs)

- Add `y-agent tui` subcommand (clap)
- Feature-gated behind `tui` flag

##### [NEW] [commands/tui_cmd.rs](file:///Users/gorgias/Projects/y-agent/crates/y-cli/src/commands/tui_cmd.rs)

- `run()`: construct `TuiApp` from `AppServices`, delegate to `TuiApp::run()`

#### T1 Test Plan

| Test ID | Description | File |
|---------|-------------|------|
| T-TUI-01-01 | `AppState` initializes with `Input` focus and `Normal` mode | `tui/state.rs` |
| T-TUI-01-02 | `set_focus()` transitions update `focus` field | `tui/state.rs` |
| T-TUI-01-03 | `set_mode()` transitions update `mode` field | `tui/state.rs` |
| T-TUI-01-04 | `toggle_sidebar()` flips `sidebar_visible` | `tui/state.rs` |
| T-TUI-01-05 | `InteractionMode` state transitions follow design state machine | `tui/state.rs` |
| T-TUI-01-06 | `PanelFocus` transitions follow design focus model | `tui/state.rs` |

---

### Phase T2: Panel Rendering (Est. 3-4 days)

> **Goal**: Implement the 4-panel layout and render each panel with real data.

#### T2.1 Layout Engine

##### [NEW] [tui/layout.rs](file:///Users/gorgias/Projects/y-agent/crates/y-cli/src/tui/layout.rs)

- `compute_layout(terminal_size, sidebar_visible) -> LayoutChunks`: calculates ratatui `Rect` for each panel
- Sidebar: 28 columns (fixed) when visible; auto-hidden on terminals < 100 columns
- Chat: remaining width
- Status Bar: 1 line at bottom of main area
- Input Area: 1-6 lines auto-expanding; capped at 30% of terminal height
- Minimum terminal size check: < 60 columns or < 15 rows shows "Terminal too small" centered message

#### T2.2 Chat Panel

##### [NEW] [tui/panels/chat.rs](file:///Users/gorgias/Projects/y-agent/crates/y-cli/src/tui/panels/chat.rs)

- Renders conversation messages as styled blocks (User / Assistant / System / Tool)
- Markdown-aware rendering: bold, italic, code blocks, inline code (via basic inline parser, not full markdown AST)
- Scroll support: `scroll_offset` controls visible window
- Auto-scroll when at bottom; "New content below" indicator when scrolled up
- Selected message highlight in Select mode
- Thinking block: collapsible, dimmed, above response
- Tool status cards: inline rendering of tool name, phase, and result preview

#### T2.3 Sidebar Panel

##### [NEW] [tui/panels/sidebar.rs](file:///Users/gorgias/Projects/y-agent/crates/y-cli/src/tui/panels/sidebar.rs)

- Tab bar at top: "Sessions" / "Agents" toggle
- Session list: label/ID, message count, last activity; sorted by most recent; active session accent
- Agent list: agent name, mode badge, model name; active agent accent
- List navigation: cursor-based selection with highlight
- Inline search: `/` to filter displayed items

#### T2.4 Status Bar

##### [NEW] [tui/panels/status_bar.rs](file:///Users/gorgias/Projects/y-agent/crates/y-cli/src/tui/panels/status_bar.rs)

- Single-line bar showing: `[session label] | [agent name] | [model] | [tokens used] | [connection state]`
- Connection state indicator: `Connected` (green), `Disconnected` (red), `Reconnecting...` (yellow)
- Content truncation with ellipsis on narrow terminals

#### T2.5 Input Area

##### [NEW] [tui/panels/input.rs](file:///Users/gorgias/Projects/y-agent/crates/y-cli/src/tui/panels/input.rs)

- Multi-line text editor using `tui-textarea`
- Auto-expand height with content (1-6 lines)
- Enter sends message (single-line); Shift+Enter inserts newline
- Input history navigation with Up/Down when empty
- Focus border accent

#### T2 Test Plan

| Test ID | Description | File |
|---------|-------------|------|
| T-TUI-02-01 | Layout hides sidebar when terminal < 100 columns | `tui/layout.rs` |
| T-TUI-02-02 | Layout shows "too small" when terminal < 60x15 | `tui/layout.rs` |
| T-TUI-02-03 | Input area height scales with content (1-6 lines) | `tui/layout.rs` |
| T-TUI-02-04 | Chat scroll offset limits clamp to message count | `tui/panels/chat.rs` |
| T-TUI-02-05 | Sidebar tab switch between Sessions and Agents | `tui/panels/sidebar.rs` |
| T-TUI-02-06 | Status bar truncates with ellipsis on narrow width | `tui/panels/status_bar.rs` |

---

### Phase T3: Keyboard Navigation and Focus System (Est. 2-3 days)

> **Goal**: Implement the full keyboard shortcut system, focus model, and interaction mode state machine.

#### T3.1 Key Dispatcher

##### [NEW] [tui/keys.rs](file:///Users/gorgias/Projects/y-agent/crates/y-cli/src/tui/keys.rs)

- `handle_key(key_event, state) -> Action` enum dispatcher
- Global shortcuts handled first (Ctrl+C, Ctrl+D, Ctrl+Q)
- Mode-specific dispatch: Normal, Command, Search, Select
- Focus-specific dispatch: Input, Chat, Sidebar
- Returns `Action` enum: `SendMessage`, `ExecuteCommand`, `ToggleSidebar`, `SwitchFocus`, `Scroll`, `Quit`, `Cancel`, `Noop`, etc.

#### T3.2 Focus Transitions

- Implement focus model per design state diagram:
  - `Tab`: Input → Sidebar (if visible) → Input cycle
  - `Ctrl+Up`: Input → Chat
  - `Esc` / `i` / `Enter`: Chat or Sidebar → Input
- Visual feedback: focused panel draws border with accent color; unfocused panels use dim border

#### T3.3 Mode Transitions

- Implement mode state machine per design:
  - Normal → Command (`/` typed)
  - Normal → Search (`Ctrl+R` or `Ctrl+F`)
  - Normal → Select (`Ctrl+Up` into chat)
  - Command → Normal (`Esc` or `Enter`)
  - Search → Normal (`Esc` or `Enter`)
  - Select → Normal (`Esc`, `i`, or `Enter`)

#### T3 Test Plan

| Test ID | Description | File |
|---------|-------------|------|
| T-TUI-03-01 | Global `Ctrl+C` sends `Cancel` action in all modes | `tui/keys.rs` |
| T-TUI-03-02 | `/` in Normal mode transitions to Command mode | `tui/keys.rs` |
| T-TUI-03-03 | `Ctrl+R` in Normal mode transitions to Search mode | `tui/keys.rs` |
| T-TUI-03-04 | `Esc` in Command mode returns to Normal | `tui/keys.rs` |
| T-TUI-03-05 | `Tab` cycles focus Input → Sidebar → Input | `tui/keys.rs` |
| T-TUI-03-06 | `y` in Select mode returns `Yank` action | `tui/keys.rs` |
| T-TUI-03-07 | `b` in Select mode returns `BranchFrom` action | `tui/keys.rs` |
| T-TUI-03-08 | Sidebar focus keys (`d`, `n`, `/`) dispatch correctly | `tui/keys.rs` |

---

### Phase T4: Command System Integration (Est. 3-4 days)

> **Goal**: Integrate the `/`-command vocabulary with the TUI, implement the command palette overlay, and wire commands to server/local actions.

#### T4.1 Command Registry

##### [NEW] [tui/commands/registry.rs](file:///Users/gorgias/Projects/y-agent/crates/y-cli/src/tui/commands/registry.rs)

- `CommandRegistry` struct: stores registered `CommandHandler` trait objects
- Built-in command registration: `/new`, `/switch`, `/list`, `/reset`, `/delete`, `/help`, `/status`, `/clear`, `/quit`, `/agent`, `/model`, `/context`, `/debug`, `/export`
- Alias support: `/n` → `/new`, `/s` → `/switch`, `/l` → `/list`, etc.
- `search(prefix) -> Vec<CommandInfo>`: fuzzy-filtered command list for palette

#### T4.2 Command Palette Overlay

##### [NEW] [tui/overlays/command_palette.rs](file:///Users/gorgias/Projects/y-agent/crates/y-cli/src/tui/overlays/command_palette.rs)

- Floating popup anchored above input area
- Fuzzy-filtered command list updates on each keystroke
- Preview bar: one-line description and argument synopsis for selected command
- Argument completion mode:
  - `/switch` → lists session IDs and labels
  - `/agent select` → lists agent names
  - `/model select` → lists model names
- Enter executes; Esc dismisses

#### T4.3 Command Handlers

##### [NEW] [tui/commands/handlers.rs](file:///Users/gorgias/Projects/y-agent/crates/y-cli/src/tui/commands/handlers.rs)

- Session handlers: `/new` (create session via `SessionManager`), `/switch` (switch active session), `/list` (populate sidebar), `/delete`, `/reset`, `/branch`, `/merge`, `/fork`
- Agent handlers: `/agent list`, `/agent select`, `/agent info`
- Model handlers: `/model list`, `/model select`, `/model params`
- Debug handlers: `/debug --on/--off`, `/status`, `/logs`, `/stats`
- Config handlers: `/config show`, `/config set`
- Export/import: `/export` (session to markdown/json)
- Each handler takes `&mut AppState` + `&AppServices` and returns `CommandResult`

#### T4 Test Plan

| Test ID | Description | File |
|---------|-------------|------|
| T-TUI-04-01 | `CommandRegistry` resolves aliases (`/n` → `/new`) | `tui/commands/registry.rs` |
| T-TUI-04-02 | `search("sw")` returns `/switch`, `/status`, `/stats` | `tui/commands/registry.rs` |
| T-TUI-04-03 | Command palette fuzzy filter narrows correctly | `tui/overlays/command_palette.rs` |
| T-TUI-04-04 | `/new` handler creates session via `SessionManager` | `tui/commands/handlers.rs` |
| T-TUI-04-05 | `/switch` handler updates `AppState` active session | `tui/commands/handlers.rs` |
| T-TUI-04-06 | Unknown command displays error in chat panel | `tui/commands/handlers.rs` |

---

### Phase T5: Streaming and LLM Integration (Est. 3-4 days)

> **Goal**: Implement token-level streaming, the send-message flow, and all server event rendering.

#### T5.1 Streaming Chat Flow

##### [NEW] [tui/chat_flow.rs](file:///Users/gorgias/Projects/y-agent/crates/y-cli/src/tui/chat_flow.rs)

- `send_message(input, state, services)`: sends user message to LLM via `ProviderPool`
- Spawns async streaming task that receives `ClientEvent` deltas
- Feeds events into `AppState.messages` for real-time rendering
- Context pipeline assembly: calls `ContextPipeline::assemble()` before each turn
- System prompt prepended via existing `build_chat_messages()` logic

#### T5.2 Streaming Renderer

##### [MODIFY] [tui/panels/chat.rs](file:///Users/gorgias/Projects/y-agent/crates/y-cli/src/tui/panels/chat.rs)

- In-progress message: appends `ClientEvent::Assistant` deltas in real time
- Thinking block: `ClientEvent::Thinking` renders as dimmed collapsible block
- Tool status: `ClientEvent::Tool` renders as inline card (name, phase, result preview)
- Frame batching: accumulate events within 16ms window; render once per frame
- Cancel: `Ctrl+C` sends cancel; marks message with "cancelled" indicator

#### T5.3 Scroll Lock Behavior

- Auto-scroll when chat is at bottom and new tokens arrive
- Scroll lock when user has scrolled up (Select mode): show "New content below" indicator at bottom of chat panel
- Clicking indicator or pressing `End` returns to bottom and resumes auto-scroll

#### T5 Test Plan

| Test ID | Description | File |
|---------|-------------|------|
| T-TUI-05-01 | User message appended to history and sent to provider | `tui/chat_flow.rs` |
| T-TUI-05-02 | Streaming deltas accumulate into current message | `tui/chat_flow.rs` |
| T-TUI-05-03 | `Ctrl+C` during streaming marks message as cancelled | `tui/chat_flow.rs` |
| T-TUI-05-04 | Scroll lock prevents auto-scroll when scrolled up | `tui/panels/chat.rs` |
| T-TUI-05-05 | "New content below" indicator shown during scroll lock | `tui/panels/chat.rs` |
| T-TUI-05-06 | Context pipeline assembles system prompt before each turn | `tui/chat_flow.rs` |

---

### Phase T6: Overlays, Search, and Polish (Est. 3-4 days)

> **Goal**: Implement remaining overlays (search, help, session switcher, agent selector), edge-case handling, and final polish.

#### T6.1 Search Overlay

##### [NEW] [tui/overlays/search.rs](file:///Users/gorgias/Projects/y-agent/crates/y-cli/src/tui/overlays/search.rs)

- **History search** (`Ctrl+R`): incremental search over input history; result populates input area
- **Conversation search** (`Ctrl+F`): incremental search over chat messages; result jumps to and highlights matching message
- Up/Down or `Ctrl+N`/`Ctrl+P` to navigate results
- Enter selects; Esc dismisses

#### T6.2 Session Switcher and Agent Selector Overlays

##### [NEW] [tui/overlays/session_switcher.rs](file:///Users/gorgias/Projects/y-agent/crates/y-cli/src/tui/overlays/session_switcher.rs)

- `Ctrl+O` overlay: fuzzy search over all sessions by label/ID
- Displays: label, message count, last activity
- Enter switches; Esc dismisses

##### [NEW] [tui/overlays/agent_selector.rs](file:///Users/gorgias/Projects/y-agent/crates/y-cli/src/tui/overlays/agent_selector.rs)

- `Ctrl+A` overlay: list agents with mode badge and model info
- Enter selects; Esc dismisses

#### T6.3 Help Overlay

##### [NEW] [tui/overlays/help.rs](file:///Users/gorgias/Projects/y-agent/crates/y-cli/src/tui/overlays/help.rs)

- `Ctrl+H` overlay: categorized keyboard shortcut reference
- Scrollable; grouped by mode (Global, Normal, Command, Search, Select, Sidebar)

#### T6.4 Edge Cases and Polish

- **Terminal resize**: immediate re-layout; preserve scroll position, input buffer, streaming state
- **Minimum terminal size**: centered "Terminal too small" message when < 60x15
- **Clipboard**: yank to system clipboard via `arboard` crate; fallback to internal buffer on failure
- **Disconnection handling**: status bar "Disconnected" indicator; input queuing for messages sent while offline
- **Session data refresh**: auto-refresh sidebar session list on focus; spinner during loading
- **Timestamps**: `Ctrl+T` toggles timestamp display next to each message

#### T6 Test Plan

| Test ID | Description | File |
|---------|-------------|------|
| T-TUI-06-01 | History search matches input history entries | `tui/overlays/search.rs` |
| T-TUI-06-02 | Conversation search highlights matching message | `tui/overlays/search.rs` |
| T-TUI-06-03 | Session switcher fuzzy matches by label | `tui/overlays/session_switcher.rs` |
| T-TUI-06-04 | Terminal resize preserves state | `tui/events.rs` |
| T-TUI-06-05 | Minimum terminal size shows warning | `tui/layout.rs` |
| T-TUI-06-06 | Clipboard fallback to internal buffer on failure | edge case tests |

---

## 3. File Structure

After all phases, the `y-cli/src/` directory will have the following TUI-related additions:

```
y-cli/src/
  tui/
    mod.rs                      — TuiApp, terminal setup, main event loop
    state.rs                    — AppState, PanelFocus, InteractionMode
    events.rs                   — EventLoop (crossterm + server events)
    layout.rs                   — Layout engine (panel sizing, responsive)
    keys.rs                     — Key dispatcher (mode/focus-aware)
    chat_flow.rs                — Send message, streaming, context pipeline
    panels/
      mod.rs                    — Panel trait and reexports
      chat.rs                   — Chat panel renderer
      sidebar.rs                — Sidebar panel (sessions/agents)
      status_bar.rs             — Status bar renderer
      input.rs                  — Input area (tui-textarea)
    overlays/
      mod.rs                    — Overlay trait and reexports
      command_palette.rs        — Command palette (fuzzy search, preview)
      search.rs                 — History and conversation search
      session_switcher.rs       — Session switcher (Ctrl+O)
      agent_selector.rs         — Agent selector (Ctrl+A)
      help.rs                   — Help overlay (keyboard reference)
    commands/
      mod.rs                    — Command system reexports
      registry.rs               — CommandRegistry, CommandHandler trait
      handlers.rs               — Built-in command handler implementations
  commands/
    tui_cmd.rs                  — `y-agent tui` subcommand entry point
```

---

## 4. Dependency Graph

```
Phase T1 (Foundation)
  ↓
Phase T2 (Panel Rendering)
  ↓
Phase T3 (Keyboard & Focus)
  ↓
Phase T4 (Command System) ←→ Phase T5 (Streaming & LLM)  [parallelizable]
  ↓
Phase T6 (Overlays, Search, Polish)  ← depends on T3 + T4 + T5
```

---

## 5. New Dependencies

| Crate | Version | Purpose | Feature Flag |
|-------|---------|---------|-------------|
| `ratatui` | latest | TUI framework, immediate-mode rendering | `tui` |
| `crossterm` | latest | Terminal backend (raw mode, events, alternate screen) | `tui` |
| `tui-textarea` | latest | Multi-line input editor widget | `tui` |
| `unicode-width` | latest | Correct width calculation for CJK and emoji | `tui` |
| `arboard` | latest | System clipboard access (yank/paste) | `tui` |

All gated behind the `tui` feature flag in `y-cli/Cargo.toml`.

---

## 6. Verification

### After Each Phase

```bash
# Unit tests
cargo test -p y-cli

# Clippy
cargo clippy -p y-cli -- -D warnings

# Build (TUI feature enabled)
cargo build -p y-cli --features tui

# Full workspace
cargo build --workspace
```

### Manual Verification (After Phase T5+)

| Scenario | Steps | Expected |
|----------|-------|----------|
| Basic chat | Launch `y-agent tui`, type message, observe response | Streaming tokens appear in chat panel |
| Session switch | `Ctrl+O`, select session, verify context changes | Chat panel shows selected session history |
| Command palette | Type `/`, select `/status`, observe result | Status displayed in chat panel |
| Sidebar toggle | Press `Tab`, navigate sessions, press `Enter` | Focus moves to sidebar, session switches |
| Scroll lock | Scroll up during streaming, check indicator | "New content below" shown; no auto-scroll |
| Terminal resize | Resize window during chat | Layout re-renders; no crash, state preserved |

---

## 7. Risks and Mitigations

| Risk | Mitigation |
|------|------------|
| ratatui immediate-mode rendering performance with large histories | Implement virtual scrolling; render only visible messages |
| crossterm key event inconsistency across platforms | Test on macOS, Linux; document known terminal emulator limitations |
| `tui-textarea` limited feature set | Fall back to custom line editor if needed; keep input logic behind abstraction |
| TUI feature flag increases CI build matrix | Test TUI feature only on one target per platform; non-TUI builds unaffected |
| Streaming frame batching timing | Use `tokio::time::interval(16ms)` for frame tick; buffer events between ticks |
| Existing `chat.rs` regression during refactor | Keep `chat.rs` (CLI mode) unchanged; TUI is a separate code path with shared `AppServices` |

---

## 8. Acceptance Criteria

- [ ] `y-agent tui` launches a 4-panel terminal interface
- [ ] Focus model: `Tab` cycles sidebar, `Ctrl+Up` enters chat, `Esc` returns to input
- [ ] All 4 interaction modes (Normal/Command/Search/Select) function per design
- [ ] Command palette with fuzzy search, preview, and argument completion works
- [ ] Token-level streaming renders in real time with auto-scroll and scroll lock
- [ ] Sidebar displays sessions and agents; switching works
- [ ] Status bar shows session, agent, model, tokens, connection state
- [ ] Terminal resize preserves state; minimum size displays warning
- [ ] All registered `/`-commands execute correctly from TUI
- [ ] `Ctrl+C` cancels in-flight LLM requests
- [ ] Feature-gated: `cargo build -p y-cli` (without `--features tui`) still compiles
- [ ] Coverage >= 70% for `tui/` module
- [ ] `cargo clippy -p y-cli --features tui -- -D warnings` zero warnings

---

## 9. Estimated Timeline

| Phase | Duration | Cumulative |
|-------|----------|------------|
| T1: Foundation | 2-3 days | 2-3 days |
| T2: Panel Rendering | 3-4 days | 5-7 days |
| T3: Keyboard & Focus | 2-3 days | 7-10 days |
| T4: Command System | 3-4 days | 10-14 days |
| T5: Streaming & LLM | 3-4 days | 10-14 days (parallel with T4) |
| T6: Overlays & Polish | 3-4 days | 13-18 days |

**Total estimated duration**: 3-4 weeks.
