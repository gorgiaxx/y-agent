---
layout: home
title: y-agent -- Modular AI Agent Framework in Rust
titleTemplate: false

head:
  - - meta
    - name: description
      content: y-agent is a modular, extensible AI agent framework written in Rust. Async-first, model-agnostic, full observability, WAL-based recoverability, and self-evolving skills.

hero:
  name: y-agent
  text: AI Agent Framework
  tagline: Async-first, model-agnostic, fully observable -- built in Rust for production workloads
  image:
    src: /logo.svg
    alt: y-agent
  actions:
    - theme: brand
      text: Get Started
      link: /guide/getting-started
    - theme: alt
      text: Download
      link: /download
    - theme: alt
      text: GitHub
      link: https://github.com/gorgias/y-agent

features:
  - title: Multi-Provider LLM Pool
    details: Tag-based routing, automatic failover, provider freeze/thaw, enable/disable toggle. Supports OpenAI, Anthropic, Gemini, DeepSeek, Ollama, Azure, and any OpenAI-compatible API.
  - title: DAG Workflow Engine
    details: Typed channels, checkpointing at every step, interrupt/resume protocol. Build complex multi-step workflows with conditional branching and parallel execution.
  - title: Tool System
    details: JSON Schema validation, LRU activation, dynamic tool creation. Multi-format parser supporting OpenAI, DeepSeek DSML, MiniMax, GLM4, Longcat, and Qwen3Coder formats.
  - title: Three-Tier Memory
    details: Short-term, long-term (Qdrant vector store), and working memory with semantic search. Context-aware memory retrieval for rich agent conversations.
  - title: Multi-Agent Collaboration
    details: Session tree with parent/child delegation. TOML-defined agents with template expansion. Built-in agents for skill ingestion, security, architecture, and more.
  - title: Guardrails & Safety
    details: Content filtering, PII detection, loop detection, risk scoring middleware. Three-layer defense -- sandbox, middleware interception, human-in-the-loop approval.
  - title: Context Pipeline
    details: 8-stage middleware chain for token-budget-aware prompt assembly. System prompt, bootstrap, memory, knowledge, skills, tools, history loading, and context status injection.
  - title: Knowledge Base & RAG
    details: Multi-level chunking (L0/L1/L2), hybrid retrieval (BM25 + vector). Import Markdown, code, PDF, and more into searchable collections.
  - title: Self-Evolving Skills
    details: Git-like versioning, experience capture, self-improvement with HITL approval. Skills grow smarter over time through usage patterns.
  - title: Browser Tool
    details: Web browsing via Chrome DevTools Protocol. Headless or visible mode with full page interaction capabilities.
  - title: Bot Adapters
    details: Expose y-agent as a Discord, Feishu (Lark), or Telegram bot. Platform adapters share the same service container with zero duplication.
  - title: Full Observability
    details: Span-based tracing, cost intelligence, trace replay. Know exactly what your agent did, why, and how much it cost.
---
