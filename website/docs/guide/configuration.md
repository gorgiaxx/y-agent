# Configuration Reference

y-agent uses a layered TOML-based configuration system with multiple precedence levels.

## Precedence (Highest to Lowest)

1. **CLI arguments** -- `--log-level debug`
2. **Environment variables** -- `Y_AGENT_LOG_LEVEL=debug`
3. **User config directory** -- `~/.config/y-agent/`
4. **Project config directory** -- `./config/`
5. **Built-in defaults**

## Config Files

| File | Description | Must Configure? |
|------|-------------|-----------------|
| `providers.toml` | LLM provider pool (API keys, models, routing tags, enable toggle) | **Yes** |
| `y-agent.toml` | Global settings (log level, output format) | No |
| `knowledge.toml` | Knowledge base embedding & retrieval | Only if using embedding |
| `storage.toml` | SQLite database path, WAL mode, transcripts | No |
| `session.toml` | Session tree depth, compaction, auto-archive | No |
| `runtime.toml` | Execution backend (Docker / Native / SSH), sandboxing | No |
| `browser.toml` | Browser tool (Chrome, headless mode, CDP) | Only if using browser |
| `hooks.toml` | Middleware timeouts, event bus capacity | No |
| `tools.toml` | Tool registry limits, MCP server connections | Only if using MCP |
| `guardrails.toml` | Permission model, loop detection, risk scoring | No |
| `bots.toml` | Bot adapter configuration (Discord, Feishu, Telegram) | Only if using bots |

## Agent Definitions

Agent definitions live in `config/agents/` as TOML files. They support **template expansion** -- placeholders like `{{YAGENT_CONFIG_PATH}}` are resolved to system-specific paths at load time.

### Built-in Agents

| Agent | Purpose |
|-------|---------|
| `agent-architect` | Agent design and configuration |
| `capability-assessor` | Capability assessment |
| `compaction-summarizer` | Context compaction |
| `complexity-classifier` | Task complexity classification |
| `context-summarizer` | Context summarization |
| `knowledge-metadata` | Knowledge entry metadata extraction |
| `knowledge-summarizer` | Knowledge base document summarization |
| `pattern-extractor` | Pattern extraction from conversations |
| `plan-phase-executor` | Plan phase execution |
| `plan-writer` | Plan generation and writing |
| `pruning-summarizer` | Context pruning optimization |
| `skill-ingestion` | Skill import and validation |
| `skill-security-check` | Security audit for skill packages |
| `task-decomposer` | Task decomposition into sub-tasks |
| `task-intent-analyzer` | Intent classification for delegation |
| `title-generator` | Session title auto-generation |
| `tool-engineer` | Dynamic tool creation |
| `translator` | Content translation |

## Proxy Configuration

```toml
# providers.toml -- multi-level proxy (global -> tag-based -> per-provider)
[proxy]
default_scheme = "socks5"

[proxy.global]
url = "socks5://proxy.company.com:1080"

[proxy.providers.ollama-local]
enabled = false   # Local provider, no proxy
```

## Browser Tool Configuration

```toml
# config/browser.toml
enabled = true
auto_launch = true
headless = true
# chrome_path = ""       # Leave empty for auto-detection
local_cdp_port = 9222
```

## MCP Server Configuration

```toml
# config/tools.toml
[[mcp_servers]]
name = "filesystem"
transport = "stdio"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/workspace"]
enabled = true
```

## Environment Variables

```bash
# LLM Provider API keys
OPENAI_API_KEY=sk-...
ANTHROPIC_API_KEY=sk-ant-...
DEEPSEEK_API_KEY=sk-...
GEMINI_API_KEY=AIza...

# Infrastructure
Y_AGENT_PORT=8080
Y_QDRANT_URL=http://localhost:6334
RUST_LOG=info
```
