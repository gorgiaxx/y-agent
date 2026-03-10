# Memory & Context Feature Analysis: OpenViking / Claude-Mem / EdgeQuake

> Comparative analysis of memory and context management features from three open-source projects, evaluated against y-agent's current design. Focus: identifying features worth adopting.

**Version**: v0.1
**Date**: 2026-03-08
**Status**: Research

---

## 1. Project Overview

| Dimension | OpenViking | Claude-Mem | EdgeQuake |
|-----------|-----------|------------|-----------|
| **Language** | Python + Go (AGFS) | TypeScript (Bun/Node) | Rust + TypeScript (MCP) |
| **Positioning** | AI Agent context database with file-system paradigm | Cross-session persistent memory plugin for Claude Code | Graph-RAG framework (LightRAG) with MCP integration |
| **Memory model** | 6-category LLM-extracted memory + L0/L1/L2 hierarchical context | Observation-based structured memory (observer Agent extracts from tool use) | Knowledge graph (entity-relationship) + vector; no conversation memory |
| **Context management** | memory_window message count limit + session archival with L0/L1 summaries | Condition-count based injection (totalObservationCount, fullObservationCount) | Token-budget truncation (TruncationConfig with fixed max_tokens) |
| **Retrieval** | Hierarchical directory retrieval + intent analysis + rerank | SQLite (primary) + Chroma vector (semantic); strategy fallback | Multi-mode (local/global/hybrid/mix/naive) graph + vector |
| **Maturity** | Production (deployed product) | Production (Claude Code plugin) | Production (multi-tenant RAG service) |

---

## 2. y-agent Current Design Summary (Baseline)

y-agent's memory and context design is comprehensive and well-structured:

- **3-tier memory**: Long-Term Memory (persistent, 4 categories: Personal/Task/Tool/Experience), Short-Term Memory (session-scoped, Compact/Compress/IndexedExperience), Working Memory (pipeline-scoped)
- **Context Assembly Pipeline**: 7-stage ordered ContextMiddleware chain (SystemPrompt -> Bootstrap -> Memory -> Skills -> Tools -> History -> ContextStatus)
- **Context Window Guard**: 3 trigger modes (auto/soft/hybrid), 5-category token budget (128K total)
- **Compression strategies**: Compact (lossless disk offload), Compress (LLM summary), IndexedExperience (agent-controlled archival, Memex-inspired)
- **Retrieval**: Hybrid (text 0.4 + vector 0.6), multi-dimensional index (semantic + time + importance + scope)
- **Session tree**: Branch/merge/fork, cross-channel canonical sessions
- **Knowledge Base**: External knowledge ingestion with domain classification, agent-driven pipelines

---

## 3. Feature-by-Feature Comparison

### 3.1 Long-Term Memory Extraction

| Aspect | y-agent (Current) | OpenViking | Claude-Mem | EdgeQuake |
|--------|-------------------|-----------|------------|-----------|
| **Extraction trigger** | Planned (design phase) | Session commit (explicit/auto) | PostToolUse Hook (every tool call) | N/A (no conversation memory) |
| **Extraction method** | LLM-driven classification into 4 types | LLM-driven extraction of 6 categories | Observer Agent generates structured observations | LLM entity/relationship extraction from documents |
| **Categories** | Personal, Task, Tool, Experience | profile, preferences, entities, events, cases, patterns | bugfix, feature, refactor, change, discovery, decision | Entities + Relationships (graph schema) |
| **Deduplication** | Planned (importance decay + pruning) | LLM-based dedup (vector similarity + LLM decide: create/merge/skip/delete) | Content hash dedup (30s window) + content_hash | Graph merge (entity normalization + LLM summary) |
| **Storage** | Vector + KV + Relational | AGFS (file) + Vector Index | SQLite (primary) + Chroma (secondary) | PostgreSQL AGE (graph) + pgvector |

### 3.2 Short-Term / Session Context Management

| Aspect | y-agent (Current) | OpenViking | Claude-Mem | EdgeQuake |
|--------|-------------------|-----------|------------|-----------|
| **Token awareness** | Yes (tiktoken incremental, 5-category budget) | No (message count only via memory_window) | No (condition-count only, no token estimation) | Partial (TruncationConfig with fixed token budgets) |
| **Compression** | 3 strategies (Compact/Compress/IndexedExperience) | Session archival with L0/L1 summary generation | Transcript-based session summary at Stop | No session compression |
| **Agent-controlled compression** | Yes (IndexedExperience, compress_experience tool) | No (system-only) | No (system-only) | No |
| **Context status exposure** | Yes (InjectContextStatus, utilization %) | No | No | No |
| **Recovery/Reload** | grep + read + read_experience | L0/L1/L2 hierarchical on-demand loading | mem-search Skill (SQLite + Chroma) | No offloaded content recovery |

### 3.3 Context Assembly & Injection

| Aspect | y-agent (Current) | OpenViking | Claude-Mem | EdgeQuake |
|--------|-------------------|-----------|------------|-----------|
| **Pipeline architecture** | 7-stage ordered ContextMiddleware | ContextBuilder with fixed segments (identity, workspace, profile, memory, bootstrap, skills) | ContextBuilder with configurable count limits | Fixed prompt template (Role/Goal/Instructions/Context/Query) |
| **Memory auto-injection** | InjectMemory stage (vector recall) | get_viking_memory_context at prompt build time | SessionStart Hook injects timeline + summaries | No memory injection (single-query RAG only) |
| **Tool injection** | Lazy loading (ToolIndex + tool_search) | Full tool schema via get_definitions() | MCP tools registered statically | MCP tools external, not injected into prompt |
| **Extensibility** | Plugin-based (y-hooks ContextMiddleware) | Code-level (modify ContextBuilder) | Config-driven (settings.json counts) | Code-level (modify build_prompt) |

### 3.4 Retrieval & RAG

| Aspect | y-agent (Current) | OpenViking | Claude-Mem | EdgeQuake |
|--------|-------------------|-----------|------------|-----------|
| **Search modes** | Hybrid (text + vector, configurable weights) | Hierarchical (directory-recursive + intent analysis + rerank) | Multi-strategy (Chroma / SQLite / Hybrid with fallback) | Multi-mode (naive/local/global/hybrid/mix) |
| **Intent analysis** | No explicit intent analysis | Yes (LLM-driven, produces TypedQuery with context_type, intent, priority) | No | Optional keyword extraction (high/low level + query_intent) |
| **Hierarchical retrieval** | Scope tree (ancestor/descendant) | Directory-recursive with L0/L1 progressive loading | No hierarchy | Graph neighborhood traversal |
| **Reranking** | No explicit rerank stage | Yes (optional rerank post-retrieval) | No | No |
| **Knowledge graph** | Phase 2 (planned) | No (file-system paradigm) | No | Yes (PostgreSQL AGE, entity-relationship, LightRAG) |

---

## 4. Features Worth Borrowing

The following table evaluates features from the three projects against y-agent's existing design. Only features that represent genuine improvements are included.

### Priority Legend
- **P1 (High)**: Directly addresses a gap in y-agent's current design; significant impact on quality or capability.
- **P2 (Medium)**: Valuable enhancement; y-agent has a partial solution but the source project's approach is notably better in specific dimensions.
- **P3 (Low)**: Nice-to-have; marginal improvement over y-agent's current design.

### 4.1 Recommended Features

| # | Feature | Source | Priority | y-agent Current State | Source Project Implementation | Recommendation & Rationale |
|---|---------|--------|----------|----------------------|------------------------------|---------------------------|
| 1 | **LLM-driven memory deduplication** | OpenViking | **P1** | Planned importance decay + periodic pruning. No LLM-based merge/dedup decision. | MemoryDeduplicator: vector-finds similar memories, then LLM decides create/merge/skip/delete per candidate. 4 actions ensure precise dedup. | **Adopt**: y-agent's pruning is coarse (time-decay). OpenViking's LLM-judged 4-action model (create/merge/skip/delete) is more precise, prevents semantic duplicates while merging evolving knowledge. Fits naturally into y-agent's background write pipeline. Cost is acceptable since dedup runs at commit/session-end, not per-turn. |
| 2 | **L0/L1/L2 hierarchical context loading** | OpenViking | **P1** | No multi-resolution content representation. Memory recall returns full content or nothing. | L0 (~100 tokens abstract), L1 (~2K overview), L2 (full content). Search returns L0 first; agent/system decides whether to load L1/L2. Token-efficient by design. | **Adopt (for Knowledge Base)**: y-agent's Knowledge Base design already plans chunking and retrieval but lacks a multi-resolution abstraction. Applying L0/L1/L2 to Knowledge Base entries (and optionally to LTM content) would dramatically reduce token waste during recall. Agent sees summaries first, loads full content on demand -- directly aligned with y-agent's token efficiency principle. |
| 3 | **Intent-aware retrieval with TypedQuery** | OpenViking | **P2** | Hybrid search (text + vector) with configurable weights. No intent decomposition. | IntentAnalyzer decomposes a query into 0-5 TypedQuery objects, each with context_type, intent, and priority. Enables targeted multi-query retrieval. | **Adopt (for Memory recall)**: y-agent's single-query hybrid search may miss relevant memories when user intent is compound (e.g., "how did we handle auth errors last time and what tools did we use?"). Intent decomposition into typed sub-queries improves recall precision. Can be implemented as an optional pre-processing step in the InjectMemory pipeline stage. |
| 4 | **Structured observation extraction (observer pattern)** | Claude-Mem | **P2** | LTM extraction planned as direct LLM analysis of conversation. No independent "observer" agent. | Separate observer Agent processes tool-use events asynchronously; produces structured observations (type, title, facts, narrative, concepts, files). Non-blocking, fire-and-forget. | **Consider selectively**: The observer-agent pattern is elegant for non-blocking extraction but introduces a separate LLM call per tool use, which is expensive. y-agent's planned batch extraction at session-end/commit is more cost-efficient. However, the **structured observation schema** (type/title/facts/narrative/concepts) is worth adopting for y-agent's Experience memory type -- richer structure improves retrieval quality. |
| 5 | **Multi-strategy search with graceful fallback** | Claude-Mem | **P2** | Hybrid search only. No explicit fallback chain if vector store is unavailable. | SearchOrchestrator: tries Chroma first, falls back to SQLite full-text, or uses Hybrid. executeWithFallback pattern ensures retrieval always works. | **Adopt**: y-agent's Memory Architecture mentions "retry 3 times + keyword fallback" for embedding failures but lacks a formal SearchOrchestrator with strategy selection and fallback. A pluggable strategy pattern (Vector -> Hybrid -> FullText -> Keyword) with automatic fallback improves resilience. Fits into the MemoryClient trait design. |
| 6 | **Content-hash deduplication (fast path)** | Claude-Mem | **P2** | No fast-path dedup before vector comparison. | content_hash (SHA256 of session+title+narrative) with 30s time window. Instant dedup for retries and duplicate events. Zero LLM cost. | **Adopt as fast-path complement**: Use content-hash as a cheap first-pass dedup before y-agent's planned LLM-based dedup. Catches exact/near-exact duplicates at write time with zero LLM cost. The two mechanisms are complementary: hash catches identical writes, LLM catches semantic overlaps. |
| 7 | **Graph-enhanced retrieval (knowledge graph)** | EdgeQuake | **P2** | Knowledge Graph listed as Phase 2 in Memory Architecture. No detailed design yet. | Full LightRAG implementation: LLM entity/relationship extraction, graph merge, multi-mode retrieval (local=entity neighborhood, global=relationship clusters). Token-budget truncation. | **Use as reference for Phase 2**: EdgeQuake provides a production-quality Rust reference for graph-RAG. Key design decisions worth studying: entity normalization + optional LLM summarization during merge, TruncationConfig for graph context, multi-mode retrieval (local vs global). y-agent should adapt rather than copy -- the entity extraction pipeline can reuse y-agent's existing tool infrastructure. |
| 8 | **Reranking stage in retrieval pipeline** | OpenViking | **P3** | No reranking. Results sorted by hybrid score only. | Optional rerank post-retrieval. Improves precision when initial recall is broad. | **Add as optional pipeline stage**: Reranking is a well-known technique to improve precision. Can be added as an optional step in y-agent's recall pipeline, using a lightweight cross-encoder model. Low priority because y-agent's multi-dimensional scoring (semantic + time + importance) already provides good ranking. |
| 9 | **Token-budget context truncation** | EdgeQuake | **P3** | Context Window Guard with 5-category budget. But retrieval results not pre-truncated before injection. | TruncationConfig: max_entity_tokens, max_relation_tokens, max_total_tokens. Results truncated by relevance before prompt assembly. | **Already mostly covered**: y-agent's Context Window Guard handles this at the pipeline level. However, adding a pre-injection token budget specifically for memory recall results (analogous to EdgeQuake's per-category limits) would make the InjectMemory stage more predictable. Minor enhancement. |

### 4.2 Features Evaluated but NOT Recommended

| Feature | Source | Why Not Adopt |
|---------|--------|---------------|
| **6-category memory classification** | OpenViking | y-agent's 4-type model (Personal/Task/Tool/Experience) is already well-designed and more aligned with agent workflows. OpenViking's categories (profile/preferences/entities/events/cases/patterns) overlap and are tuned for consumer chatbot use cases. |
| **Viking URI file-system paradigm** | OpenViking | Imposes a specific storage model. y-agent's scope-tree + pluggable storage backends (Vector/KV/Relational) is more flexible and better suited for a framework. |
| **Per-tool-use observer Agent** | Claude-Mem | Too expensive for y-agent's cost-efficiency goals. One LLM call per tool use at runtime is not viable for the micro-agent pipeline pattern (many small steps). Batch extraction at session-end is more appropriate. |
| **Fire-and-forget Chroma sync** | Claude-Mem | y-agent's Read Barrier design is strictly better: guarantees write-then-read consistency. Fire-and-forget leads to stale reads and requires backfill mechanisms. |
| **SQLite as primary + vector as secondary** | Claude-Mem | y-agent's vector-first design is more suitable for semantic retrieval. SQLite-first is a pragmatic choice for Claude-Mem's simpler search needs. |
| **Conversation_history in query request** | EdgeQuake | EdgeQuake has this field but doesn't use it in build_prompt. Not a feature to borrow -- y-agent already handles conversation context through the Session Tree and Context Assembly Pipeline. |
| **Fixed prompt template** | EdgeQuake | y-agent's extensible ContextMiddleware pipeline is categorically superior to a hardcoded Role/Goal/Instructions/Context/Query template. |

---

## 5. Integration Roadmap Suggestion

Suggested integration sequence based on priority and dependency:

| Phase | Features | Dependency | Effort Estimate |
|-------|----------|------------|-----------------|
| **Phase 1 (with LTM implementation)** | #1 LLM-driven dedup, #6 Content-hash fast-path dedup | Memory Architecture implementation | Medium -- extends the write pipeline |
| **Phase 1 (with Knowledge Base)** | #2 L0/L1/L2 hierarchical loading | Knowledge Base implementation | Medium -- new abstraction layer over chunks |
| **Phase 2 (Memory recall enhancement)** | #3 Intent-aware TypedQuery, #5 Multi-strategy search fallback | Memory recall implementation | Medium -- adds pre-processing and fallback logic |
| **Phase 2 (Experience type enrichment)** | #4 Structured observation schema | Experience memory type implementation | Low -- schema extension only |
| **Phase 3 (Knowledge Graph)** | #7 Graph-RAG reference study | Knowledge Base Phase 2 | High -- new subsystem |
| **Optional** | #8 Reranking, #9 Pre-injection token budget | Retrieval pipeline completion | Low -- additive enhancements |

---

## 6. Summary

Of the three projects analyzed:

- **OpenViking** provides the most relevant features for y-agent, particularly in **memory deduplication** (LLM-judged 4-action model), **hierarchical context loading** (L0/L1/L2), and **intent-aware retrieval** (TypedQuery). These address real gaps in y-agent's design.

- **Claude-Mem** offers solid engineering patterns: **content-hash fast dedup**, **multi-strategy search fallback**, and a well-structured **observation schema**. The observer-agent pattern is interesting but too costly for y-agent's architecture.

- **EdgeQuake** is primarily valuable as a **reference implementation for Phase 2 knowledge graph** work. Its Rust + PostgreSQL AGE implementation is directly relevant. Its context/memory capabilities are narrower than y-agent's design (no session memory, no conversation compression).

y-agent's existing design is architecturally more advanced than all three projects in key areas: the 3-tier memory model, IndexedExperience (Memex-inspired agent-controlled compression), Context Window Guard with soft/hybrid modes, Session Tree with canonical sessions, and the extensible ContextMiddleware pipeline. The recommended borrowings are tactical enhancements to specific subsystems, not architectural changes.

---

*This analysis is based on code-level review of the three projects' source code and y-agent's design documents as of 2026-03-08.*
