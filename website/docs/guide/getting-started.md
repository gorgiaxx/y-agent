# Getting Started

This guide walks you through building, configuring, and running y-agent for the first time.

## Prerequisites

| Dependency | Required? | Notes |
|------------|-----------|-------|
| **Rust 1.94+** | Yes | Pinned in `rust-toolchain.toml` |
| **Node.js 18+** | GUI only | [nodejs.org](https://nodejs.org) |
| **SQLite 3.35+** | Embedded | Bundled, no action needed |
| **Chrome / Chromium** | Optional | For the browser tool (auto-detected) |
| PostgreSQL 14+ | Optional | For diagnostics / analytics |
| Qdrant | Optional | For semantic vector search |

## Build

```bash
# Clone
git clone https://github.com/gorgias/y-agent.git
cd y-agent

# Build CLI + Web server
cargo build --release
# Binary: target/release/y-agent

# Or build the GUI desktop app (Tauri v2)
cd crates/y-gui && npm install && cd ../..
./scripts/build-release.sh gui
```

## Initialize Configuration

```bash
# Interactive (recommended for first setup)
y-agent init

# Non-interactive
y-agent init --non-interactive --provider openai
```

This generates the configuration tree:

```
./
  .env                         # API key placeholders
  config/
    y-agent.example.toml       # Global settings (log level, output)
    providers.example.toml     # LLM provider pool  ** MUST configure **
    knowledge.example.toml     # Knowledge base & embedding
    storage.example.toml       # Database & transcript
    session.example.toml       # Session tree, compaction, auto-archive
    runtime.example.toml       # Docker / Native sandbox, resource limits
    browser.example.toml       # Browser tool
    hooks.example.toml         # Middleware timeouts, event bus capacity
    tools.example.toml         # Tool registry limits, MCP servers
    guardrails.example.toml    # Permission model, loop detection, risk scoring
    agents/                    # TOML-based agent definitions
    prompts/                   # System prompt templates
  data/
    transcripts/               # Session transcript storage
```

## Configure a Provider

This is the **most critical step**. Without at least one LLM provider, y-agent cannot function.

Copy `config/providers.example.toml` to `config/providers.toml` and edit it (or use the GUI Settings > Providers tab):

```toml
[[providers]]
id = "openai-main"
provider_type = "openai"
model = "gpt-4o"
tags = ["reasoning", "general"]
max_concurrency = 3
context_window = 128000
api_key = "sk-your-openai-key-here"
enabled = true
# Or use an environment variable:
# api_key_env = "OPENAI_API_KEY"
```

### Provider Presets

| Provider | `provider_type` | Model Example | API Key Env Var | Base URL |
|----------|----------------|---------------|-----------------|----------|
| OpenAI | `openai` | `gpt-4o` | `OPENAI_API_KEY` | *(default)* |
| Anthropic | `anthropic` | `claude-sonnet-4-20250514` | `ANTHROPIC_API_KEY` | *(default)* |
| Google Gemini | `gemini` | `gemini-2.5-flash` | `GEMINI_API_KEY` | *(default)* |
| DeepSeek | `openai` | `deepseek-chat` | `DEEPSEEK_API_KEY` | `https://api.deepseek.com/v1` |
| Groq | `openai` | `llama-3.3-70b-versatile` | `GROQ_API_KEY` | `https://api.groq.com/openai/v1` |
| Together AI | `openai` | `meta-llama/Llama-3.3-70B` | `TOGETHER_API_KEY` | `https://api.together.xyz/v1` |
| Ollama (local) | `ollama` | `llama3.1:8b` | *(none)* | `http://localhost:11434` |
| Azure OpenAI | `azure` | `gpt-4o` | *(your key)* | `https://your-resource.openai.azure.com/openai/deployments/gpt-4o` |
| Any OpenAI-compat | `openai` | *(user-specified)* | *(user-specified)* | *(your endpoint /v1)* |

Multiple providers can coexist. y-agent routes requests by tags and automatically fails over when a provider is unavailable. Providers can be toggled on/off with the `enabled` field.

## Start

```bash
# CLI interactive chat
y-agent chat

# TUI mode (ratatui terminal UI)
y-agent tui

# Start the Web API server (axum, port 8080)
y-agent serve

# Or launch the GUI desktop app
# (built via build-release.sh -- .app / .dmg / .AppImage in dist/)
```

## Next Steps

- [Configuration Reference](/guide/configuration) -- Full config file reference
- [GUI Desktop App](/guide/gui-desktop) -- Using the Tauri desktop GUI
- [Knowledge Base](/guide/knowledge-base) -- Setting up RAG and semantic search
- [Architecture](/architecture/) -- Understanding the system architecture
