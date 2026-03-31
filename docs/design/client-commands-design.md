# Client Commands Design

> Command system for CLI/TUI interaction in y-agent

**Version**: v0.3
**Created**: 2026-03-04
**Updated**: 2026-03-10
**Status**: Draft

---

## TL;DR

y-agent's command system provides a unified, `/`-prefixed command vocabulary shared by CLI and TUI clients, covering session control, file and context references, agent/model management, tool invocation, debugging, and configuration. It also defines `@file` and `#context` reference syntaxes for injecting external content into conversations. Commands are designed to be intuitive for daily use, context-aware for smart completion, and extensible for future plugin commands. Batch and pipe modes enable scripting and automation.

---

## Background and Goals

### Background

Users interact with y-agent through conversational messages and structural commands. Messages are forwarded to the agent for processing; commands control the client-side environment (sessions, agents, models, tools, configuration). A well-designed command system reduces friction in common workflows like switching sessions, referencing files, and debugging agent behavior.

The command system must work identically in CLI (readline-based) and TUI (ratatui-based) clients to avoid user confusion when switching between interfaces.

### Goals

| Goal | Measurable Criteria |
|------|-------------------|
| **Intuitive syntax** | New users can discover commands via `/help` within 30 seconds |
| **Full coverage** | All session, agent, model, tool, and config operations accessible via commands |
| **Context-aware completion** | Tab completion for commands, file paths, session IDs, and agent names |
| **Cross-client consistency** | CLI and TUI share 100% of the command vocabulary |
| **Extensibility** | New command addable with a single handler registration |
| **Scriptability** | All commands executable in batch mode via `.yagent` script files |

### Assumptions

1. Commands are prefixed with `/` to distinguish them from conversational messages.
2. File references use `@` prefix; context references use `#` prefix.
3. Command parsing happens client-side before any server communication.
4. All commands are synchronous from the user's perspective (they block until complete or return immediately with status).

---

## Scope

### In Scope

- Session control commands (`/new`, `/switch`, `/list`, `/reset`, `/delete`, `/branch`, `/merge`, `/fork`, `/tree`)
- File reference system (`@file`, `@dir/`, line ranges, symbol lookup)
- Context reference system (`#git-diff`, `#clipboard`, `#recent`, etc.)
- Agent and model management commands (`/agent`, `/model`)
- Context management commands (`/context`, `/add`, `/remove`)
- Memory management commands (`/memory`)
- Tool invocation commands (`/tool`, `/exec`)
- Debug and status commands (`/debug`, `/status`, `/logs`, `/stats`)
- Configuration commands (`/config`)
- Export/import commands (`/export`, `/import`)
- Built-in aliases and custom alias support
- TUI keyboard shortcuts
- Batch mode and pipe mode
- Smart completion engine

### Out of Scope

- Plugin command framework (deferred)
- Natural language command interpretation (e.g., "switch to my research session")
- Voice command input
- Server-side command execution

---

## High-Level Design

### Command Categories

```mermaid
flowchart TB
    subgraph Input["User Input"]
        Msg["Conversational Message"]
        Cmd["/ Command"]
        FileRef["@ File Reference"]
        CtxRef["# Context Reference"]
    end

    subgraph Parser["Command Parser"]
        Detect["Input Classifier"]
        CmdParser["Command Parser"]
        RefResolver["Reference Resolver"]
    end

    subgraph Handlers["Command Handlers"]
        Session["Session Commands"]
        Agent["Agent/Model Commands"]
        Context["Context Commands"]
        Tool["Tool Commands"]
        Debug["Debug Commands"]
        Config["Config Commands"]
        IO["Export/Import Commands"]
    end

    subgraph Targets["Execution Targets"]
        Server["Server via ClientProtocol"]
        LocalState["Local Client State"]
        FileSystem["File System"]
    end

    Msg --> Detect
    Cmd --> Detect
    FileRef --> Detect
    CtxRef --> Detect

    Detect -->|"starts with /"| CmdParser
    Detect -->|"contains @"| RefResolver
    Detect -->|"contains #"| RefResolver
    Detect -->|"plain text"| Server

    CmdParser --> Session
    CmdParser --> Agent
    CmdParser --> Context
    CmdParser --> Tool
    CmdParser --> Debug
    CmdParser --> Config
    CmdParser --> IO

    RefResolver --> FileSystem
    RefResolver --> Server

    Session --> Server
    Session --> LocalState
    Agent --> Server
    Tool --> Server
    Debug --> LocalState
    Config --> LocalState
```

**Diagram rationale**: Flowchart chosen to show how user input is classified, routed to the appropriate handler, and ultimately targets either the server, local state, or file system.

**Legend**:
- **Input**: Four types of user input, distinguished by prefix (`/`, `@`, `#`, or plain text).
- **Handlers**: Grouped by domain; each handler processes one category of commands.
- **Targets**: Commands may affect the server (via ClientProtocol), local client state, or the file system.

### Command Reference

#### Session Commands

| Command | Description | Key Options |
|---------|-------------|-------------|
| `/new` | Create a new session | `--label`, `--agent`, `--model`, `--parent`, `--inherit` |
| `/switch <target>` | Switch to a session (ID, label, `-` for previous, `..` for parent, `/` for root) | Fuzzy matching supported |
| `/list` | List sessions | `--all`, `--tree`, `--agent`, `--sort`, `--limit` |
| `/reset` | Reset current session | `--keep-config`, `--confirm` |
| `/delete <id>` | Delete a session | `--recursive`, `--archive`, `--confirm` |
| `/branch` | Create a branch from current session | `--from <msg_id>`, `--label`, `--message` |
| `/merge <source>` | Merge another session into current | `--strategy (append/interleave/summarize)`, `--delete-source` |
| `/fork` | Create a working copy of current session | `--label`, `--shallow`, `--keep-link` |
| `/tree` | Display session tree structure | Optional root session ID |

#### File and Context References

| Syntax | Description | Example |
|--------|-------------|---------|
| `@path/to/file` | Reference entire file | `Explain @src/main.rs` |
| `@file:10` | Reference line 10 | `Bug at @src/main.rs:42` |
| `@file:10-20` | Reference line range | `Review @src/lib.rs:10-20` |
| `@file:symbol` | Reference function/struct by name | `Improve @src/parser.rs:parse_expression` |
| `@dir/` | Reference directory tree | `What's in @src/?` |
| `@dir/**/*.rs` | Glob pattern match | `Find TODOs in @src/**/*.rs` |
| `#git-diff` | Staged git diff | `Explain #git-diff` |
| `#git-status` | Git status | `What changed? #git-status` |
| `#clipboard` | Clipboard contents | `Analyze: #clipboard` |
| `#recent` | Recently modified files | `Review #recent` |
| `#cwd` | Current working directory | `Where am I? #cwd` |

#### Agent and Model Commands

| Command | Description |
|---------|-------------|
| `/agent list` | List available agents with descriptions and capabilities |
| `/agent select <id>` | Switch to a different agent |
| `/agent info <id>` | Show agent details (model, tools, skills) |
| `/model list` | List available models with context window and cost info |
| `/model select <name>` | Switch model |
| `/model params` | View/set model parameters (temperature, max_tokens) |

#### Context, Tool, Debug, Config Commands

| Command | Description |
|---------|-------------|
| `/context` | Show current context usage (files, tokens, memory) |
| `/add file <path>` | Manually add file to context |
| `/remove file <path>` | Remove file from context |
| `/memory search <query>` | Search long-term memory |
| `/memory add <key> <value>` | Store a memory entry |
| `/tool <name> [args]` | Invoke a tool directly (bypassing LLM) |
| `/exec <command>` | Execute a shell command |
| `/debug --on/--off` | Toggle debug mode |
| `/status` | Show system status (connection, session, usage, cost) |
| `/logs --tail N` | View recent logs |
| `/stats` | Show token usage and cost statistics |
| `/config show` | Display current configuration |
| `/config set <key> <val>` | Set a configuration value |
| `/export [file]` | Export session (markdown/json/html) |
| `/import <file>` | Import session from file |

### Alias System

Built-in single-character aliases for common commands:

| Alias | Expands To |
|-------|-----------|
| `/n` | `/new` |
| `/s` | `/switch` |
| `/l` | `/list` |
| `/r` | `/reset` |
| `/a` | `/agent` |
| `/m` | `/model` |
| `/h` | `/help` |
| `/q` | `/exit` |

Users can define custom aliases via `/alias <name> <command>`.

### TUI Interaction Design

The TUI client provides a ratatui-based terminal interface for extended interactive sessions. It shares the full `/`-command vocabulary with the CLI, but adds spatial navigation, multi-panel layout, and keyboard-driven workflows optimized for long-running agent interactions.

#### Panel Layout

```mermaid
flowchart LR
    subgraph TUI["TUI Layout"]
        direction TB
        subgraph TopRow["Main Area"]
            direction LR
            Sidebar["Sidebar Panel\n(sessions / agents)"]
            Chat["Chat Panel\n(conversation + streaming)"]
        end
        StatusBar["Status Bar\n(session, agent, model, tokens, connection)"]
        Input["Input Area\n(multi-line editor + completion overlay)"]
    end
```

**Diagram rationale**: Flowchart chosen to show the spatial arrangement and nesting of TUI panels.

**Legend**:
- **Sidebar Panel**: Toggleable left panel displaying session list or agent list; collapsed by default on narrow terminals (< 100 columns).
- **Chat Panel**: Primary content area showing the conversation transcript with streaming token rendering.
- **Status Bar**: Single-line bar at the bottom of the main area showing current session, active agent, model, token usage, and connection state.
- **Input Area**: Multi-line text editor at the bottom with auto-expanding height (1-6 lines) and completion overlay.

#### Panel Sizing

| Panel | Default Size | Resize Behavior |
|-------|-------------|-----------------|
| **Sidebar** | 28 columns (fixed) | Toggleable via `Tab`; hidden on terminals < 100 columns wide |
| **Chat** | Remaining width | Fills all horizontal space not used by sidebar |
| **Status Bar** | 1 line (fixed) | Always visible; content truncated with ellipsis on narrow terminals |
| **Input Area** | 1-6 lines (auto) | Grows with input content; maximum height capped at 30% of terminal height |

#### Focus Model and Navigation

The TUI uses a focus-based navigation model. Exactly one panel holds focus at any time; the focused panel receives keyboard input and is visually highlighted with a border accent.

```mermaid
stateDiagram-v2
    [*] --> InputFocus: TUI launch
    InputFocus --> SidebarFocus: Tab (sidebar visible)
    SidebarFocus --> InputFocus: Tab / Esc / Enter
    InputFocus --> ChatFocus: Ctrl+Up
    ChatFocus --> InputFocus: Esc / i / Enter
    ChatFocus --> SidebarFocus: Tab (sidebar visible)
    SidebarFocus --> ChatFocus: Ctrl+Up
```

**Diagram rationale**: State diagram chosen to show focus transitions between panels triggered by keyboard shortcuts.

**Legend**:
- **InputFocus**: Default state; user types messages or commands. Most time is spent here.
- **ChatFocus**: User scrolls through conversation history, selects messages for copying or branching.
- **SidebarFocus**: User navigates session or agent list for switching.

#### TUI Interaction Modes

The TUI operates in four mutually exclusive modes that determine how keyboard input is interpreted:

```mermaid
stateDiagram-v2
    [*] --> Normal: TUI launch
    Normal --> Command: type "/"
    Command --> Normal: Esc / Enter (execute)
    Normal --> Search: Ctrl+R or Ctrl+F
    Search --> Normal: Esc / Enter (select)
    Normal --> Select: Ctrl+Up (enter chat scroll)
    Select --> Normal: Esc / i (return to input)
    Command --> Search: Ctrl+R (search within commands)
    Search --> Command: Esc (back to command input)
```

**Diagram rationale**: State diagram chosen to show the mutually exclusive interaction modes and their transitions.

**Legend**:
- **Normal**: Default mode. Typing goes to the input editor; keyboard shortcuts are active.
- **Command**: Activated by typing `/` in the input area. A command palette overlay appears with filtered command list and argument hints.
- **Search**: Activated by `Ctrl+R` (history search) or `Ctrl+F` (conversation search). An incremental search overlay filters results as the user types.
- **Select**: Activated by scrolling into the chat panel. Enables message selection for copy, branch, or context addition.

#### Keyboard Shortcuts

Shortcuts are grouped by the context in which they are active.

**Global (active in all modes):**

| Shortcut | Action |
|----------|--------|
| Ctrl+C | Cancel current LLM operation (sends cancel to server) |
| Ctrl+D | Exit TUI (prompts confirmation if operation in progress) |
| Ctrl+Q | Quit immediately without confirmation |

**Normal Mode (Input Focus):**

| Shortcut | Action |
|----------|--------|
| Enter | Send message (single-line); insert newline in multi-line mode |
| Shift+Enter | Force newline (multi-line input toggle) |
| Tab | Toggle sidebar visibility; if completion overlay is open, accept completion |
| Ctrl+Up | Move focus to chat panel (enter Select mode) |
| Ctrl+N | Create new session (equivalent to `/new`) |
| Ctrl+O | Open session switcher overlay (fuzzy search over session list) |
| Ctrl+A | Open agent selector overlay |
| Ctrl+R | Open command history search |
| Ctrl+F | Open conversation search |
| Ctrl+H | Toggle help overlay |
| Ctrl+L | Clear chat viewport (scroll to bottom; history preserved) |
| Ctrl+P | Open command palette (equivalent to typing `/`) |
| Ctrl+T | Toggle timestamps in chat messages |
| Ctrl+E | Export current session (equivalent to `/export`) |
| Up/Down | Navigate input history (when input is single-line and empty) |

**Command Mode (Command Palette Open):**

| Shortcut | Action |
|----------|--------|
| Up/Down | Navigate filtered command list |
| Tab | Accept selected command; move to argument input |
| Enter | Execute selected command with current arguments |
| Esc | Close command palette; return to Normal mode |
| Ctrl+R | Switch to history search within command palette |

**Search Mode:**

| Shortcut | Action |
|----------|--------|
| Up/Down | Navigate search results |
| Enter | Select result (insert into input or jump to message) |
| Esc | Close search; return to previous mode |
| Ctrl+N / Ctrl+P | Next / previous result (alternative to Up/Down) |

**Select Mode (Chat Panel Focus):**

| Shortcut | Action |
|----------|--------|
| Up/Down | Scroll message by message |
| PgUp/PgDn | Scroll by page |
| Home/End | Jump to first / last message |
| y | Yank (copy) selected message content to clipboard |
| b | Branch from selected message (equivalent to `/branch --from`) |
| a | Add selected message content to input area |
| Enter / Esc / i | Return to input (Normal mode) |

**Sidebar Focus (Session/Agent List):**

| Shortcut | Action |
|----------|--------|
| Up/Down | Navigate list items |
| Enter | Switch to selected session or agent |
| d | Delete selected session (with confirmation) |
| n | Create new session |
| / | Filter list (inline search) |
| Tab / Esc | Return focus to input area |

#### Command Palette

The command palette is a floating overlay that appears when the user types `/` in Normal mode or presses `Ctrl+P`. It provides filtered command discovery with inline documentation.

**Behavior**:

1. **Activation**: Typing `/` opens the palette anchored above the input area.
2. **Filtering**: Each subsequent keystroke narrows the command list using fuzzy matching. For example, `/sw` matches `/switch`, `/stats`, `/status`.
3. **Preview**: The selected command shows a one-line description and argument synopsis in a preview bar below the list.
4. **Argument completion**: After selecting a command, the palette transitions to argument mode. For `/switch`, it lists sessions; for `/agent select`, it lists agents; for `/model select`, it lists models. Each with fuzzy search.
5. **Execution**: Pressing `Enter` executes the command. The palette closes and the result appears in the chat panel or status bar.
6. **Dismissal**: `Esc` closes the palette without executing.

#### Streaming Render Behavior

During token-level streaming from the server, the chat panel renders incoming tokens with the following rules:

| Aspect | Behavior |
|--------|----------|
| **Token append** | Each `ClientEvent::Assistant` delta is appended to the current message bubble in real time |
| **Scroll lock** | If the user has scrolled up (Select mode), new tokens do NOT auto-scroll; a "New content below" indicator appears |
| **Auto-scroll** | If the chat panel is at the bottom, it auto-scrolls to show new tokens |
| **Frame batching** | Rapid token events are batched into single frame renders (target: 60fps / 16ms per frame) to avoid flickering |
| **Thinking display** | `ClientEvent::Thinking` tokens render in a collapsible, dimmed block above the response |
| **Tool execution** | `ClientEvent::Tool` events render as inline status cards showing tool name, phase (running/complete/error), and result preview |
| **Cancel feedback** | On `Ctrl+C`, the current streaming message is marked with a "cancelled" indicator and the partial content is preserved |

#### Sidebar Panel Interactions

The sidebar has two views, toggled by a tab bar at the top of the sidebar:

**Session List View:**
- Displays sessions sorted by last activity (most recent first).
- Each entry shows: truncated label (or session ID), message count, last activity timestamp.
- Active session is highlighted with a distinct accent.
- Inline search (`/` in sidebar focus) filters sessions by label or ID.

**Agent List View:**
- Displays available agents with their mode (build/plan/explore/general).
- Each entry shows: agent name, mode badge, model name.
- Active agent is highlighted.
- Selecting an agent switches the current session to use that agent.

#### TUI-Specific Failure Handling

| Scenario | Handling |
|----------|--------|
| Terminal resize during streaming | Re-render layout immediately; preserve scroll position, input buffer, and streaming state |
| Terminal too small (< 60 columns or < 15 rows) | Display a centered "Terminal too small" message; suppress all panel rendering |
| Clipboard access failure | Display warning in status bar; fall back to internal yank buffer |
| Sidebar data stale (session list) | Auto-refresh on sidebar focus; show spinner during refresh |
| Connection lost during TUI session | Status bar shows "Disconnected" with reconnect countdown; input area remains active for queuing messages |

### Batch and Pipe Modes

**Batch mode**: Execute a script file containing commands and messages:

```bash
y-agent batch script.yagent
```

Script format (`.yagent`):
```
# Comments start with #
/new --label "Code Review"
@src/main.rs
Please review this code for security issues
/export review.md
/exit
```

**Pipe mode**: Accept input from stdin:

```bash
echo "Explain this code" | y-agent run --agent coder < code.rs
```

---

## Data and State Model

### Command Registry

```rust
struct CommandRegistry {
    commands: HashMap<String, Box<dyn CommandHandler>>,
    aliases: HashMap<String, String>,
}

trait CommandHandler: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn usage(&self) -> &str;
    async fn execute(&self, args: &[&str], ctx: &mut ClientContext) -> Result<()>;
    fn completions(&self, partial: &str, ctx: &ClientContext) -> Vec<String>;
}
```

### File Reference Model

| Field | Type | Description |
|-------|------|-------------|
| `path` | PathBuf | File or directory path |
| `range` | Option<LineRange> | Single line, line range, or none (entire file) |
| `symbol` | Option<String> | Function/struct name for symbol-based lookup |
| `Glob` | Option<String> | Glob pattern for directory references |

### Completion Context

The completion engine uses the current client state to provide context-aware suggestions:
- **Command position**: Suggest command names from the registry
- **Argument position**: Suggest values based on command type (session IDs, agent names, file paths)
- **File reference**: Suggest file paths from the workspace
- **Session ID**: Suggest from cached session list with fuzzy matching on IDs and labels

---

## Failure Handling and Edge Cases

| Scenario | Handling |
|----------|---------|
| Unknown command | Display "Unknown command: X. Type /help for available commands." |
| File reference to non-existent file | Display warning, continue processing message without the reference |
| File reference exceeds context budget | Truncate with warning: "File truncated to fit context window (showing first N lines)" |
| Symbol not found in file | Fall back to full file reference with warning |
| Glob pattern matches too many files | Cap at configurable limit (default 20); display count of skipped files |
| `/exec` command returns non-zero exit code | Display stderr output; do not abort session |
| `/merge` with conflicting message timestamps | Use source session timestamps; warn if clock skew detected |
| `/branch --from` with invalid message ID | Display error with list of valid recent message IDs |
| Batch script syntax error | Report line number and error; abort script |
| Alias creates circular reference | Detect at alias creation time; reject with error |

---

## Security and Permissions

| Concern | Approach |
|---------|----------|
| **Shell execution** (`/exec`) | Commands run with user's permissions; optional sandboxing via config flag `cli.sandbox_exec` |
| **File access** (`@file`) | Restricted to workspace root by default; `..` traversal blocked unless explicitly allowed |
| **Context injection** (`#context`) | Built-in contexts are read-only; no write operations possible via context references |
| **Sensitive file detection** | Warn when referencing files matching `.env`, `credentials.*`, `*secret*` patterns |
| **Batch mode** | Batch scripts cannot execute `/config set` commands that modify security settings |
| **Alias safety** | Aliases cannot override built-in security commands (`/exit`, `/config`) |

---

## Performance and Scalability

| Metric | Target |
|--------|--------|
| Command parse time | < 1ms for any command |
| File reference resolution | < 50ms for single file; < 200ms for Glob with 20 files |
| Tab completion response | < 100ms |
| Session list retrieval | < 200ms for up to 1000 sessions |
| Batch script throughput | > 10 commands/second |
| Alias resolution | < 0.1ms (single HashMap lookup) |

### Optimization Strategies

- File path completion uses an in-memory directory tree cache, refreshed on workspace change.
- Session list is cached client-side with a 5-second TTL to avoid repeated server calls during rapid `/list` commands.
- Symbol lookup uses a lightweight AST parser for Rust/Python/TypeScript; falls back to regex for other languages.

---

## Observability

- All commands are logged with timestamp, command name, arguments (with sensitive values redacted), and execution duration.
- `/stats` command provides per-session metrics: message count, token usage, tool call count, cost, and session duration.
- `/debug --show-tokens` overlays token count on each message in the chat display.
- `/debug --show-latency` displays round-trip time for each server request.
- Failed commands increment a `client.command_errors` counter by command name.

---

## Rollout and Rollback

### Phased Implementation

| Phase | Scope | Duration |
|-------|-------|----------|
| **Phase 1** | Core commands: `/new`, `/switch`, `/list`, `/reset`, `@file`, `@dir/`, `/agent`, `/model`, `/help`, `/status` | 1-2 weeks |
| **Phase 2** | Advanced session: `/branch`, `/merge`, `/fork`, `/tree`; context: `/context`, `/add`, `/remove`; memory: `/memory` | 1-2 weeks |
| **Phase 3** | Tools and debug: `/tool`, `/exec`, `/debug`, `/logs`, `/stats`, `/config` | 1 week |
| **Phase 4** | Polish: `/export`, `/import`, `/alias`, batch mode, smart completion | 1-2 weeks |

### Rollback Plan

Commands are independently registered in the `CommandRegistry`. Any command can be disabled by removing its handler registration without affecting other commands. Batch mode and alias features are additive and can be feature-flagged independently.

---

## Alternatives and Trade-offs

### Command Prefix

| | `/` prefix (chosen) | `:` prefix | `!` prefix |
|-|---------------------|-----------|-----------|
| **Familiarity** | Slack, Discord, many chat apps | Vim-like | Jupyter notebooks |
| **Conflict with messages** | Rare in natural language | Rare | Common in exclamations |
| **Visual distinction** | Good | Good | Moderate |

**Decision**: `/` prefix chosen for familiarity with chat application conventions.

### File Reference Syntax

| | `@path` (chosen) | `{path}` | `[[path]]` |
|-|------------------|---------|-----------|
| **Readability** | High | Moderate | High |
| **Typing speed** | Fast (1 char) | Slow (2 chars) | Slow (4 chars) |
| **Conflict with text** | Email addresses (handled by path validation) | Code blocks | Wiki syntax |

**Decision**: `@path` chosen for minimal typing overhead and familiarity from IDE conventions.

### Merge Strategy for `/merge`

| Strategy | Behavior | Best For |
|----------|----------|----------|
| **Append** | Concatenate source messages after target | Simple sequential merges |
| **Interleave** | Sort all messages by timestamp | Reconstructing parallel work |
| **Summarize** | LLM-summarize source, append summary | Reducing context size |

**Decision**: Support all three strategies with append as default; let user choose via `--strategy` flag.

---

## Open Questions

| # | Question | Owner | Due Date | Status |
|---|----------|-------|----------|--------|
| 1 | Should `/exec` support background execution with status tracking? | Client team | 2026-03-20 | Open |
| 2 | Should file references support URL syntax (`@https://...`) for remote files? | Client team | 2026-03-27 | Open |
| 3 | Should custom commands be registerable via a plugin API or only via aliases? | Client team | 2026-04-03 | Open |
| 4 | Should `/branch --from` support branching from a specific tool call result, not just messages? | Client team | 2026-03-27 | Open |

