# Crate Reference

Complete reference for all 24 workspace crates. Organized by architectural layer.

## Core

### y-core

**Path:** `crates/y-core/` | **Purpose:** Boundary-defining crate -- trait definitions and shared types only.

**Key Traits:**

| Trait | Methods | Used By |
|-------|---------|---------|
| `LlmProvider` | `chat_completion()`, `chat_completion_stream()`, `metadata()` | y-provider |
| `ProviderPool` | `chat_completion()`, `chat_completion_stream()`, `report_error()`, `freeze()`, `thaw()` | y-service |
| `Tool` | `execute()`, `definition()`, `check_permissions()`, `is_read_only()`, `is_destructive()` | y-tools |
| `ToolRegistry` | `tool_index()`, `search()`, `get()`, `register()`, `unregister()` | y-tools |
| `RuntimeAdapter` | `execute()`, `spawn()`, `kill()`, `health_check()`, `cleanup()` | y-runtime |
| `Middleware` | `execute()`, `chain_type()`, `priority()`, `name()` | y-hooks, y-guardrails |
| `AgentRunner` | `run(AgentRunConfig) -> AgentRunOutput` | y-service |
| `AgentDelegator` | `delegate(agent_name, input, context_strategy, session_id)` | y-service |

**Key Types:**

| Type | Description |
|------|-------------|
| `Message` | Chat message with role, content, tool_calls, metadata |
| `Role` | System, User, Assistant, Tool |
| `ChatRequest` | LLM request with messages, model, tools, thinking config |
| `ChatResponse` | LLM response with content, tool_calls, usage, raw payloads |
| `ToolDefinition` | Tool schema with name, description, JSON Schema parameters, category |
| `ToolCategory` | FileSystem, Network, Shell, Search, Memory, Knowledge, Agent, Workflow, Schedule, Interaction, Custom |
| `ToolType` | BuiltIn, Mcp, Custom, Dynamic |
| `ToolCallingMode` | PromptBased (XML tags) vs Native (API-level) |
| `TokenUsage` | input_tokens, output_tokens, cache_read_tokens, cache_write_tokens |
| `ProviderType` | OpenAi, Anthropic, Gemini, Ollama, Azure, OpenRouter, Custom |
| `SessionNode` | Session tree node with parent/root/depth/path/state/agent info |
| `SessionType` | Main, Child, Branch, Ephemeral, SubAgent, Canonical |
| `SessionState` | Active, Paused, Archived, Merged, Tombstone |
| `Memory` | Memory entry with type, scopes, importance, access_count |
| `MemoryType` | Personal, Task, Tool, Experience |
| `HookPoint` | 24 lifecycle hook points (PreLlmCall, PostToolExecute, etc.) |
| `ChainType` | Context, Tool, Llm, Compaction, Memory |
| `ThinkingConfig` | Extended thinking with effort levels (Low/Medium/High/Max) |
| `ResponseFormat` | Text, JsonObject, JsonSchema |
| `ChatCheckpoint` | Turn-level checkpoint for rollback |
| `AgentRunConfig` | Agent execution configuration (name, prompt, models, tools, timeout) |

**ID Types (newtype wrappers over String):**
`SessionId`, `WorkflowId`, `TaskId`, `ProviderId`, `AgentId`, `ToolName`, `SkillId`, `MemoryId`

---

## Infrastructure

### y-provider

**Path:** `crates/y-provider/` | **Purpose:** LLM provider pool with routing, failover, and metrics.

| Component | Responsibility |
|-----------|---------------|
| `ProviderPoolImpl` | Multi-provider pool with semaphore-based concurrency |
| `TagBasedRouter` | 4-step route selection: freeze -> tag -> priority -> strategy |
| `FreezeManager` | Adaptive freeze durations based on error classification |
| `HealthChecker` | Background provider health monitoring |
| `ProviderMetrics` | Success/error/latency/cost tracking |
| `PriorityScheduler` | Critical/Normal/Idle tier scheduling with reserved slots |
| `OpenAiProvider` | OpenAI + compatible APIs (DeepSeek, etc.) |
| `AnthropicProvider` | Anthropic Claude API |
| `GeminiProvider` | Google Gemini API |
| `OllamaProvider` | Local Ollama inference |
| `AzureOpenAiProvider` | Azure OpenAI deployment |
| `OpenAiEmbeddingProvider` | Embedding generation |

### y-session

**Path:** `crates/y-session/` | **Purpose:** Session lifecycle, state machine, tree traversal.

| Component | Responsibility |
|-----------|---------------|
| `SessionManager` | CRUD operations, dual-transcript writes, fork/branch |
| `StateMachine` | Session state transitions (Active/Paused/Archived/Merged/Tombstone) |
| `ChatCheckpointManager` | Turn-level checkpointing for rollback |
| `CanonicalSessionManager` | Canonical session resolution |
| `TreeUtils` | Session tree traversal and query |

### y-context

**Path:** `crates/y-context/` | **Purpose:** Context assembly, compaction, memory recall.

| Component | Responsibility |
|-----------|---------------|
| `ContextPipeline` | Priority-ordered provider chain (100-700) |
| `ContextWindowGuard` | 3 trigger modes for context overflow |
| `CompactionEngine` | LLM-based conversation summarization |
| `PruningEngine` | RetryPruning (zero LLM cost) + ProgressivePruning (rolling summary) |
| `RecallStore` | Hybrid text/vector memory recall |
| `KnowledgeContextProvider` | Knowledge base injection at priority 350 |
| `WorkingMemory` | Pipeline-scoped blackboard for inter-stage data |

### y-storage

**Path:** `crates/y-storage/` | **Purpose:** SQLite backends and JSONL transcript writers.

| Store | Backend | Data |
|-------|---------|------|
| `SqliteSessionStore` | SQLite | Session metadata |
| `SqliteChatMessageStore` | SQLite | Chat messages with status flags |
| `SqliteChatCheckpointStore` | SQLite | Turn-level checkpoints |
| `SqliteCheckpointStorage` | SQLite | Workflow checkpoints |
| `SqliteScheduleStore` | SQLite | Schedule definitions and execution records |
| `SqliteWorkflowStore` | SQLite | Workflow templates |
| `SqliteProviderMetricsStore` | SQLite | Provider performance data |
| `JsonlTranscriptStore` | JSONL files | Context transcripts (LLM-facing) |
| `JsonlDisplayTranscriptStore` | JSONL files | Display transcripts (UI-facing) |

**Utilities:** `create_pool()` (sqlx SqlitePool with WAL mode), `run_embedded_migrations()`.

### y-knowledge

**Path:** `crates/y-knowledge/` | **Purpose:** External knowledge ingestion and hybrid retrieval.

| Component | Responsibility |
|-----------|---------------|
| `IngestionPipeline` | Document ingestion with format detection and chunking |
| `ChunkingStrategy` | L0/L1/L2 multi-resolution chunking |
| `RuleBasedClassifier` | Domain classification for knowledge entries |
| `Bm25Index` | BM25 full-text search index |
| `HybridRetriever` | BM25 + vector combined retrieval |
| `ProgressiveLoader` | On-demand sub-document loading |
| `QualityFilter` | Relevance and quality scoring |
| `AutoTokenizer` | Language-aware tokenization (Chinese, simple) |
| `VectorIndexer` | Qdrant vector indexing (feature-gated) |

### y-diagnostics

**Path:** `crates/y-diagnostics/` | **Purpose:** Trace storage, cost intelligence, replay.

| Component | Responsibility |
|-----------|---------------|
| `DiagnosticsContext` | Global static context (`DIAGNOSTICS_CTX`) for trace scoping |
| `CostIntelligence` | Token cost computation and aggregation |
| `TraceReplay` | Replay recorded traces for debugging |
| `TraceSearch` | Query traces by session, time, agent |
| `SqliteTraceStore` | Production trace storage |
| `InMemoryTraceStore` | Test-only trace storage |
| `DiagnosticsSubscriber` | Event bus listener for automatic trace capture |

---

## Middleware

### y-hooks

**Path:** `crates/y-hooks/` | **Purpose:** Middleware chain execution and event bus.

| Component | Responsibility |
|-----------|---------------|
| `HookSystem` | Unified facade for chains + event bus |
| `MiddlewareChain` | Priority-sorted pipeline per `ChainType` |
| `ChainRunner` | Timeout-guarded chain execution |
| `HookRegistry` | Hook registration and lifecycle |
| `EventBus` | `tokio::broadcast` async event distribution |
| `HookHandlerExecutor` | Command/HTTP/prompt-agent decision handlers (feature-gated) |

### y-guardrails

**Path:** `crates/y-guardrails/` | **Purpose:** Safety guardrails implemented as middleware.

| Component | Priority | Responsibility |
|-----------|----------|---------------|
| `ToolGuardMiddleware` | 10 | Permission enforcement for tool calls |
| `LoopDetectorMiddleware` | 20 | 4-pattern loop detection (repetition, oscillation, drift, redundant) |
| `LlmGuardMiddleware` | 900 | LLM output validation |
| `PermissionModel` | -- | allow/notify/ask/deny evaluation |
| `TaintTracker` | -- | Data flow taint propagation |
| `RiskScorer` | -- | Composite risk assessment |
| `HitlProtocol` | -- | Human-in-the-loop with configurable timeout |
| `CapabilityGapMiddleware` | -- | Detects missing capabilities |
| `GuardrailManager` | -- | Hot-reloadable config via `RwLock<GuardrailConfig>` |

### y-prompt

**Path:** `crates/y-prompt/` | **Purpose:** Prompt template engine with section management.

| Component | Responsibility |
|-----------|---------------|
| `PromptSection` | Typed, prioritized, token-budgeted prompt fragment |
| `PromptTemplate` | Declarative section composition with mode overlays |
| `SectionStore` | Section registry and lookup |
| `ModeOverlay` | build/plan/explore/general mode toggles |
| `estimate_tokens()` | Fast token estimation |
| `truncate_to_budget()` | Budget-aware content truncation |
| `truncate_tool_result()` | Hard 10K character cap for tool results |

### y-mcp

**Path:** `crates/y-mcp/` | **Purpose:** Model Context Protocol client for third-party tools.

| Component | Responsibility |
|-----------|---------------|
| `McpClient` | Stdio + HTTP transport MCP client |
| `McpToolAdapter` | Wraps MCP tools as y-core `Tool` trait objects |
| `McpConnectionManager` | Connection lifecycle with `ReconnectPolicy` |
| `McpAuthStore` | Credential storage for MCP servers |
| `discovery` module | MCP server auto-discovery |

---

## Capabilities

### y-tools

**Path:** `crates/y-tools/` | **Purpose:** Tool registry with 4 types, lazy loading, validation.

| Component | Responsibility |
|-----------|---------------|
| `ToolRegistryImpl` | Central registry for all tool types |
| `ToolActivationSet` | LRU-based active tool set (ceiling 20) |
| `ToolIndex` | Lightweight name+description index (always loaded) |
| `ToolExecutor` | Schema validation + middleware + execution |
| `JsonSchemaValidator` | JSON Schema Draft 7 parameter validation |
| `DynamicToolManager` | Runtime tool create/update/delete |
| `ResultFormatter` | Tool output formatting |
| `RateLimiter` | Per-tool rate limiting |
| `ToolTaxonomy` | Hierarchical tool categorization |
| Parser functions | Multi-format tool call parsing (OpenAI, DeepSeek, MiniMax, GLM4, Qwen3) |

### y-skills

**Path:** `crates/y-skills/` | **Purpose:** Skill ingestion, versioning, evolution.

| Component | Feature Gate | Responsibility |
|-----------|-------------|---------------|
| `SkillRegistry` | always-on | Skill registration and lookup |
| `PersistentVersionStore` | always-on | Content-addressable versioning with JSONL reflog |
| `SkillGarbageCollector` | always-on | Unused skill cleanup |
| `IngestionPipeline` | `skill_ingestion` | Format detection, classification, decomposition |
| `SecurityScreener` | `skill_security_screening` | Security verdict before activation |
| `ResourceLinker` | `skill_linkage` | Cross-skill dependency resolution |
| `ExperienceStore` | `evolution_capture` | Usage experience recording |
| `PatternExtractor` | `evolution_extraction` | Pattern mining from usage data |
| `SkillRefiner` | `evolution_refinement` | Automated skill improvement with regression detection |
| `FastPathExtractor` | `evolution_fast_path` | Quick pattern extraction for common cases |
| `SkillUsageAudit` | `skill_usage_audit` | Usage tracking and analytics |

### y-runtime

**Path:** `crates/y-runtime/` | **Purpose:** Sandboxed code execution.

| Backend | Isolation | Use Case |
|---------|-----------|----------|
| `DockerRuntime` | Container | Strongest isolation, network/filesystem control |
| `NativeRuntime` | bubblewrap | Linux namespace isolation, lower overhead |
| `SshRuntime` | Remote | Execution on remote machines |

Supporting components: `RuntimeManager`, `CapabilityChecker`, `ImageWhitelist`, `SecurityPolicy`, `SecurityProfile`, `AuditTrail`, `ResourceMonitor` (CPU/memory/disk), `VenvManager`.

### y-scheduler

**Path:** `crates/y-scheduler/` | **Purpose:** Task scheduling with multiple trigger types.

| Schedule Type | Description |
|--------------|-------------|
| `CronSchedule` | 5-field cron expressions (via croner) |
| `IntervalSchedule` | Fixed interval repetition |
| `OneTimeSchedule` | Single future execution |
| `EventSchedule` | Event-driven triggers |

Supporting: `SchedulerManager`, `ScheduleExecutor`, `WorkflowDispatcher`, `ConcurrencyPolicy`, `MissedPolicy`, `ParameterSchema` (JSON Schema for parameterized runs).

### y-browser

**Path:** `crates/y-browser/` | **Purpose:** Browser automation via Chrome DevTools Protocol.

| Component | Responsibility |
|-----------|---------------|
| `CdpClient` | WebSocket JSON-RPC CDP client |
| `BrowserSession` | Connection lifecycle with caching and dedup |
| `BrowserTool` | Implements y-core `Tool` trait |
| `ChromeLauncher` | Local headless Chrome process management |
| `SecurityPolicy` | Domain allowlist + SSRF protection |

Supports local Chrome and remote CDP providers (Browserless, Browserbase).

### y-journal

**Path:** `crates/y-journal/` | **Purpose:** File mutation journaling and rollback.

| Component | Responsibility |
|-----------|---------------|
| `FileJournalMiddleware` | Intercepts file-mutating tool calls |
| `JournalStore` | Three-tier storage (inline SQLite < 256KB, blob 256KB-10MB, git-ref) |
| `FileHistoryManager` | Per-session file backups at user message boundaries |
| `ConflictDetector` | External modification detection |
| `rollback_scope()` | Scope-based rollback with conflict reporting |

---

## Orchestration

### y-agent

**Path:** `crates/y-agent/` | **Purpose:** DAG workflow engine and agent framework.

**Orchestrator module:**

| Component | Responsibility |
|-----------|---------------|
| DAG Engine | Serial, parallel (All/Any/AtLeast), conditional, loop task patterns |
| State Channels | Typed channels with reducers (LastValue, Append, Merge, Custom) |
| Checkpoint Manager | Task-level checkpointing for crash recovery |
| Interrupt Protocol | Pause/resume for HITL integration |
| Stream Modes | None, Values, Updates, Messages, Debug |
| Workflow Definitions | Dual format: TOML templates + expression DSL |

**Agent module:**

| Component | Responsibility |
|-----------|---------------|
| `AgentDefinition` | TOML-declared agent (role, models, tools, skills, mode) |
| `AgentRegistry` | Agent lookup and lifecycle |
| `AgentPool` | Concurrent agent pool (default max 5) |
| `DelegationProtocol` | Cross-agent delegation with context strategy |
| `DelegationTracker` | Delegation chain tracking |
| `TrustTier` | BuiltIn > UserDefined > Dynamic |

### y-bot

**Path:** `crates/y-bot/` | **Purpose:** Chat platform adapters.

| Platform | Status | Transport |
|----------|--------|-----------|
| Feishu | Active | Webhook (HTTP) |
| Discord | Active | Gateway (WebSocket) + REST |
| Telegram | Interface defined | Pending implementation |

**Trait:** `BotPlatform` -- `parse_event()`, `send_message()`, `verify_signature()`, `handle_challenge()`.

---

## Service

### y-service

**Path:** `crates/y-service/` | **Purpose:** Business logic orchestration hub.

| Service | Responsibility |
|---------|---------------|
| `ServiceContainer` | DI root -- wires all domain services from config |
| `ChatService` | LLM turn lifecycle (context -> LLM -> tools -> diagnostics) |
| `AgentService` | Agent execution loop, delegation, tool dispatch |
| `BotService` | Bot persona management and message routing |
| `CostService` | Token cost computation and daily summaries |
| `DiagnosticsService` | Trace management and replay |
| `ObservabilityService` | System snapshots (agent pool, providers, scheduler) |
| `RewindService` | File-level rewind with conflict detection |
| `SchedulerService` | Schedule CRUD and execution monitoring |
| `SkillIngestionService` | Skill import with format detection and security screening |
| `SkillService` | Skill registry queries and detail views |
| `SystemService` | Health checks and status reports |
| `WorkflowService` | Workflow CRUD, DAG visualization, validation |
| `WorkspaceService` | Workspace management |

---

## Presentation

### y-cli

**Path:** `crates/y-cli/` | **Binary:** `y-agent`

Subcommands: `chat`, `status`, `config`, `session`, `tool`, `agent`, `workflow`, `diag`, `skill`, `kb`, `mcp`, `tui`, `resume`, `fork`, `serve`, `init`, `completion`.

Boot sequence: init/completion (pre-config) -> `ConfigLoader` -> tracing subscriber -> `wire::wire(&config)` creates `ServiceContainer` -> dispatch subcommand.

### y-web

**Path:** `crates/y-web/` | **Framework:** axum

REST route groups: health, sessions, chat, agents, tools, diagnostics, bots, workflows, schedules, events, config, workspaces, skills, knowledge, observability, rewind, attachments.

### y-gui

**Path:** `crates/y-gui/` | **Framework:** Tauri v2 + React 19 + TypeScript

In-process `ServiceContainer` (zero HTTP overhead). Streaming via Tauri `app.emit`/`listen` events.

---

## Testing

### y-test-utils

**Path:** `crates/y-test-utils/` | **Purpose:** Shared test mocks and fixtures.

Exports: `MockProvider`, `MockBehaviour`, `MockRuntime`, `MockCheckpointStorage`, `MockSessionStore`, `MockTranscriptStore`, and all fixture helpers via `fixtures::*`.
