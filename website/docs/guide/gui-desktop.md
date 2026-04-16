# GUI Desktop App

y-agent ships with a **Tauri v2 desktop GUI** built with React 19 and TypeScript. The frontend uses Radix UI primitives, Lucide icons, react-virtuoso for virtualized lists, and Mermaid for diagram rendering.

## Layout

The GUI follows a **VSCode-style layout** with a sidebar and main content area:

| Sidebar Panel | Description |
|---------------|-------------|
| **Chat** | Chat sessions, organized by workspaces |
| **Automation** | Workflow and schedule management (DAG editor, execution history) |
| **Skills** | Installed skills -- search, import, enable/disable |
| **Knowledge** | Knowledge base collections -- create, import, search |
| **Agents** | Registered agents -- built-in, user-defined, dynamic. Agent studio for editing. |
| **Observation** | Diagnostics and observability -- traces, costs, system health |

## Chat Interface

- **New Session** -- Click the `+` button in the sidebar to start a new chat
- **Send** -- Press `Enter` to send, `Shift+Enter` for a newline
- **Slash Commands** -- Type `/` to open the command menu (`/new`, `/clear`, `/settings`, `/model <name>`, `/status`, `/diagnostics`, `/export`)
- **Skill Mention** -- Type `/` and select a skill to attach as `@skill-name`
- **Knowledge RAG** -- Click the knowledge button in the toolbar to attach collections for retrieval-augmented generation
- **Model Selector** -- Click the `@` button to switch between configured providers
- **Context Reset** -- Click the eraser button to insert a context reset divider; messages before it are excluded from future context
- **Stop Generation** -- Click the stop button during streaming to cancel

## Agents View

- Browse all registered agents (built-in and user-defined)
- Agent overview with glyph display
- **Agent Studio** -- visual editor for creating and modifying agent definitions
- Per-agent session rail showing recent conversations

## Automation View

- **Workflows** -- Create, edit, and visualize DAG-based workflows with the built-in graph editor
- **Schedules** -- Cron/interval/one-time schedule management with pause/resume
- **Execution History** -- View past workflow and schedule execution results
- **DAG Graph** -- Interactive visualization of workflow step dependencies

## Workspaces

1. Click the folder icon in the sidebar header to create a new workspace
2. Give it a name and a filesystem path
3. Create sessions within a workspace
4. Move sessions between workspaces via the context menu

## Settings

Open via `/settings` or the gear icon:

| Tab | Configures |
|-----|------------|
| **General** | Theme (dark / light), log level, output format |
| **Providers** | Add / edit / delete / test / toggle LLM providers |
| **Session** | Max tree depth, compaction threshold, auto-archive |
| **Runtime** | Execution backend (Native / Docker / SSH), Python / Bun venvs |
| **Browser** | Browser tool toggle, headless mode, Chrome path |
| **Storage** | SQLite path, WAL mode, transcript directory |
| **Tools** | Max active tools, tool configuration |
| **MCP** | MCP server connections (stdio / HTTP transports) |
| **Guardrails** | Permission model, loop detection, risk scoring |
| **Knowledge** | Embedding model, chunking, retrieval strategy |
| **Hooks** | Middleware timeouts, event bus capacity, hook handler configuration |
| **Prompts** | View and edit system prompt templates |
| **TOML Editor** | Direct TOML config file editing with syntax highlighting |
| **About** | Version info and system details |

## Status Bar

The bottom status bar displays:

- Current session ID and turn count
- Token usage progress relative to the context window
- Active provider and model name
