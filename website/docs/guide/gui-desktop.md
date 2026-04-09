# GUI Desktop App

y-agent ships with a **Tauri v2 desktop GUI** built with React 19 and TypeScript. The frontend uses Radix UI primitives, Lucide icons, react-virtuoso for virtualized lists, and Mermaid for diagram rendering.

## Layout

The GUI follows a **VSCode-style layout** with a sidebar and main content area:

| Sidebar Panel | Description |
|---------------|-------------|
| **Sessions** | Chat history, organized by workspaces |
| **Automation** | Workflow automation (DAG editor) |
| **Skills** | Installed skills -- search, import, enable/disable |
| **Knowledge** | Knowledge base collections -- create, import, search |
| **Agents** | Registered agents -- built-in, user-defined, dynamic |

## Chat Interface

- **New Session** -- Click the `+` button in the sidebar to start a new chat
- **Send** -- Press `Enter` to send, `Shift+Enter` for a newline
- **Slash Commands** -- Type `/` to open the command menu (`/new`, `/clear`, `/settings`, `/model <name>`, `/status`, `/diagnostics`, `/export`)
- **Skill Mention** -- Type `/` and select a skill to attach as `@skill-name`
- **Knowledge RAG** -- Click the knowledge button in the toolbar to attach collections for retrieval-augmented generation
- **Model Selector** -- Click the `@` button to switch between configured providers
- **Context Reset** -- Click the eraser button to insert a context reset divider; messages before it are excluded from future context
- **Stop Generation** -- Click the stop button during streaming to cancel

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
| **Tools** | Max active tools, MCP server configuration |
| **Guardrails** | Permission model, loop detection, risk scoring |
| **Knowledge** | Embedding model, chunking, retrieval strategy |
| **Prompts** | View and edit system prompt templates |

## Status Bar

The bottom status bar displays:

- Current session ID and turn count
- Token usage progress relative to the context window
- Active provider and model name
