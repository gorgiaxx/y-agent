# y-agent Agent Autonomy Standard

**Version**: v0.2
**Created**: 2026-03-11
**Status**: Draft

---

## 1. Purpose

This document defines the engineering standard for how agents are defined, invoked, and self-managed throughout the y-agent framework. The core principle: **every LLM reasoning operation in y-agent is an agent delegation**. No module may bypass the multi-agent framework to make direct LLM calls with ad-hoc abstractions.

This standard enables y-agent's vision of agent autonomy — agents that can independently create, configure, evaluate, and retire other agents at runtime — while ensuring safety through permission inheritance, trust hierarchies, and validation pipelines.

---

## 2. Foundational Principle: Unified Agent Invocation

### 2.1 The Rule

All LLM reasoning operations within y-agent MUST be expressed as agent delegations through the `AgentPool` / `DelegationProtocol`. This applies to:

- **Internal system tasks**: compaction summarization, context summarization, input enrichment analysis, capability-gap assessment, pattern extraction, skill usage audit
- **User-facing tasks**: chat, code generation, research, planning
- **Autonomy tasks**: agent design (`agent-architect`), tool creation (`tool-engineer`)

### 2.2 Prohibited Patterns

| Anti-Pattern                  | Example                                                 | Why It's Wrong                                                                                                              |
| ----------------------------- | ------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------- |
| **Module-scoped LLM trait**   | `CompactionLlm` in `y-context`                          | Creates a parallel LLM abstraction outside the agent framework; bypasses observability, guardrails, and resource governance |
| **Hardcoded prompt strings**  | `build_summarize_prompt()` in Rust source               | Prompts not managed through the Prompt System; cannot be customized, versioned, or audited                                  |
| **Inline agent loop**         | Custom `loop { llm.call(); }` in a module               | Duplicates retry, timeout, checkpointing, and guardrail logic already in the Orchestrator                                   |
| **Direct ProviderPool calls** | `provider_pool.chat_completion()` outside agent context | Bypasses middleware chains (Context, Tool, LLM), guardrails, and diagnostics                                                |

### 2.3 Correct Pattern

When a module needs LLM reasoning, it MUST:

1. **Define a built-in `AgentDefinition`** (TOML) for the purpose
2. **Register it** in the `AgentRegistry` at startup
3. **Invoke it** via `DelegationProtocol` with the appropriate `ContextStrategy`
4. **Receive structured output** via the delegation result

```rust
// CORRECT: Module passes input data to agent; agent controls its own prompt
let result = agent_pool.delegate(DelegationRequest {
    agent_name: "compaction-summarizer",
    context_strategy: ContextStrategy::None,
    input: serde_json::json!({
        "messages": messages,
        "message_count": count,
    }),
}).await?;
let summary = result.output;
```

```rust
// WRONG: Module defines its own LLM abstraction
trait CompactionLlm: Send + Sync {
    async fn summarize(&self, prompt: &str) -> Result<String, String>;
}
```

### 2.4 Cross-Module Agent Invocation Trait

To avoid crate dependency violations (peer crates cannot depend on each other), `y-core` defines a minimal invocation trait:

```rust
/// Trait for modules to request agent delegation without depending on y-agent.
///
/// Modules pass structured input data — the agent controls its own prompt.
/// The agent's system prompt and reasoning strategy are defined in its AgentDefinition;
/// the caller only provides the data to be processed.
#[async_trait]
pub trait AgentDelegator: Send + Sync {
    /// Delegate a task to a named agent with structured input data.
    ///
    /// `input` is the raw data the agent needs to process (e.g., messages to summarize,
    /// experience records to analyze). The agent's own prompt template determines how
    /// this data is presented to the LLM.
    async fn delegate(
        &self,
        agent_name: &str,
        input: serde_json::Value,
        context_strategy: ContextStrategyHint,
    ) -> Result<DelegationOutput, DelegationError>;
}
```

At runtime, `y-agent`'s `AgentPool` implements this trait and is injected into modules that need it. The `AgentPool` resolves the agent's `AgentDefinition`, constructs the full prompt by combining the agent's system instructions with the structured input data (formatted via the agent's prompt template), and executes the delegation.

---

## 3. Agent Definition Standard

### 3.1 Definition Sources (Trust Hierarchy)

| Source           | Trust Tier | Origin                                                      | Modifiable At Runtime             |
| ---------------- | ---------- | ----------------------------------------------------------- | --------------------------------- |
| **Built-in**     | Highest    | Compiled into binary or shipped as TOML in `config/agents/` | No (read-only)                    |
| **User-defined** | Medium     | User-created TOML in `~/.config/y-agent/agents/`            | Yes (by user)                     |
| **Dynamic**      | Lowest     | Created at runtime via `agent_create` meta-tool             | Yes (by agents, with constraints) |

### 3.2 Agent Definition Schema

Every agent, regardless of source, uses the same `AgentDefinition` TOML schema:

```toml
[agent]
name = "compaction-summarizer"       # unique identifier
role = "Summarize conversation history for context compaction"
mode = "explore"                      # build | plan | explore | general

[agent.model]
preferred = ["gpt-4o-mini"]
fallback = []
temperature = 0.2

[agent.tools]
allowed = []                          # no tools needed for pure reasoning
denied = ["ShellExec", "FileWrite"]

[agent.context]
sharing = "none"                      # none | summary | filtered | full
max_tokens = 4096

[agent.limits]
max_iterations = 3                    # most system agents need few iterations
max_tool_calls = 0
timeout = "30s"

[agent.instructions]
system = """
You are a summarization specialist. Given a list of conversation messages,
produce a concise summary that preserves:
- All identifiers (file paths, URLs, email addresses)
- Key decisions and their rationale
- Code references and function names
- Action items and their owners
Output the summary as plain text, not markdown.
"""
```

### 3.3 Naming Convention

| Category                               | Pattern                                             | Examples                           |
| -------------------------------------- | --------------------------------------------------- | ---------------------------------- |
| System agents (compaction, enrichment) | `{function}-{role}`                                 | `compaction-summarizer`,           |
| Autonomy agents                        | `{domain}-{role}`                                   | `tool-engineer`, `agent-architect` |
| User-facing agents                     | User-chosen                                         | `researcher`, `code-reviewer`      |
| Dynamic agents                         | User/agent-chosen (prefixed internally with `dyn:`) | `dyn:pr-reviewer`                  |

### 3.4 Mode Selection Guidelines

| Mode        | Use When                                                 | Tool Access         | Model Tier      |
| ----------- | -------------------------------------------------------- | ------------------- | --------------- |
| **build**   | Agent produces artifacts (code, tool definitions, files) | Full (allowed list) | High-capability |
| **plan**    | Agent analyzes and proposes without side effects         | Read-only           | High-capability |
| **explore** | Agent gathers or synthesizes information quickly         | Search + read       | Fast/cheap      |
| **general** | Default balanced operation                               | Full (allowed list) | Default         |

System sub-agents that only do text transformation (summarization, analysis) SHOULD use `explore` mode with no tool access and a fast/cheap model.

---

## 4. Agent Autonomy Rules

### 4.1 Agent Creation Rights

| Creator          | Can Create          | Delegation Depth | Constraints                   |
| ---------------- | ------------------- | ---------------- | ----------------------------- |
| System (startup) | Built-in agents     | N/A              | Hardcoded, immutable          |
| User             | User-defined agents | Initial: 2       | None (full authority)         |
| Agent (runtime)  | Dynamic agents      | Parent depth - 1 | Permission inheritance (§4.2) |

### 4.2 Permission Inheritance (Snapshot Model)

When an agent creates a child agent via `agent_create`:

```
child.effective_permissions = intersection(
    child.declared_permissions,
    creator.permissions_at_creation_time   // snapshot, not live reference
)
```

Dimension-by-dimension:

| Dimension               | Rule                                                |
| ----------------------- | --------------------------------------------------- |
| `tools.allowed`         | Child's must be a **subset** of creator's           |
| `tools.denied`          | Child's must be a **superset** of creator's         |
| `limits.max_iterations` | `min(child.declared, creator.actual)`               |
| `limits.max_tool_calls` | `min(child.declared, creator.actual)`               |
| `limits.timeout`        | `min(child.declared, creator.actual)`               |
| `delegation_depth`      | `creator.depth - 1`; at 0, `agent_create` is denied |

### 4.3 Lifecycle Operations

| Operation  | Tool                           | Constraints                                          |
| ---------- | ------------------------------ | ---------------------------------------------------- |
| Create     | `agent_create`                 | Validation pipeline (§4.4); permission inheritance   |
| Update     | `agent_update`                 | Only dynamic agents; re-validates permissions        |
| Deactivate | `agent_deactivate`             | Soft-delete; preserves experience records            |
| Search     | `agent_search`                 | All trust tiers searchable; results tagged by source |
| Reactivate | `agent_update` (status=active) | Requires original creator or higher trust tier       |

### 4.4 Validation Pipeline

All agent definitions created at runtime go through three stages:

| Stage          | Checks                                                                   | Failure Action                |
| -------------- | ------------------------------------------------------------------------ | ----------------------------- |
| **Schema**     | Valid TOML, required fields, mode enum, model names in provider pool     | Reject with parse errors      |
| **Permission** | Permission inheritance rule, delegation depth > 0, tool allowlist subset | Reject with violation details |
| **Safety**     | System prompt injection patterns, dangerous tool combinations            | Reject or escalate to HITL    |

### 4.5 Resource Governance

- **Global agent concurrency**: Max 10 concurrent agent instances (configurable)
- **Per-delegation concurrency**: Max 5 agents per delegation (configurable)
- **Dynamic agent workspace limit**: Max 50 dynamic agents (configurable)
- **Delegation depth default**: 2 (root agent → sub-agent → sub-sub-agent)

---

## 5. Prompt Management

### 5.1 Rule

All agent prompts MUST be externalized. No prompt text in Rust source files.

### 5.2 Prompt Locations

| Prompt Type             | Location                                               | Format                            |
| ----------------------- | ------------------------------------------------------ | --------------------------------- |
| System instructions     | `AgentDefinition.instructions.system` (TOML)           | Plain text                        |
| Task-specific templates | `PromptSection` definitions in `y-prompt` SectionStore | TOML with `{{slot}}` placeholders |
| Mode-specific overlays  | `PromptTemplate.mode_overlays`                         | TOML                              |
| Input data              | Passed by caller via `DelegationRequest.input`         | `serde_json::Value`               |

The agent's prompt template is responsible for formatting the input data into the final user message sent to the LLM. Callers never construct prompts — they only provide structured data.

### 5.3 Prompt Design Rules for System Agents

1. System prompt MUST define the agent's single responsibility clearly
2. System prompt MUST specify output format expectations (plain text, JSON, structured)
3. System prompt MUST NOT contain y-agent internals (crate names, struct names)
4. Agent's prompt template formats the structured input data into the final user message; callers MUST NOT construct prompts
5. System prompts SHOULD stay under 500 tokens for system agents (token efficiency)

---

## 6. Observability

All agent delegations — including system sub-agents — pass through the standard middleware chains, providing:

| Signal             | Automatic                                                    |
| ------------------ | ------------------------------------------------------------ |
| Trace spans        | Each delegation creates a child span linked to parent        |
| Token usage        | Tracked per agent instance (`agents.instance.tokens_used`)   |
| Delegation metrics | `agents.delegations.total`, `agents.delegations.duration_ms` |
| Cost tracking      | Routed through Diagnostics (PostgreSQL)                      |
| Guardrail checks   | LLM middleware chain applies to all agents                   |

This is a key advantage of the unified model: internal system agent calls (e.g., compaction summarization) get the same observability as user-facing agent calls at zero additional implementation cost.

---

## 7. Migration Guide (Existing Anti-Patterns)

The following existing implementations violate this standard and MUST be refactored:

| Module                            | Current Pattern                                                                                   | Target State                                                      |
| --------------------------------- | ------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------- |
| `y-context/compaction.rs`         | `CompactionLlm` trait with `build_summarize_prompt()`                                             | Replace with delegation to `compaction-summarizer` built-in agent |
| `y-agent/context.rs`              | `apply_summary()` with inline summary logic                                                       | Replace with delegation to `compaction-summarizer` built-in agent |
| `y-session/manager.rs`            | `generate_title()` with hardcoded system prompt and direct `ProviderPool::chat_completion()` call | Replace with delegation to `title-generator` built-in agent       |
| `y-context` (planned)             | `TaskIntentAnalyzer` as inline sub-agent                                                          | Define as `task-intent-analyzer` built-in agent                   |
| `y-skills/evolution.rs` (planned) | Pattern extraction LLM calls                                                                      | Define as `pattern-extractor` built-in agent                      |
| `y-hooks` (planned)               | Capability mismatch assessment                                                                    | Define as `capability-assessor` built-in agent                    |

Refactoring priority: new implementations MUST follow this standard; existing implementations are migrated during their next modification cycle (boy scout rule).

---

## 8. Checklist (for Code Review)

When reviewing code that involves LLM reasoning:

- [ ] Is the LLM call expressed as an agent delegation (not a direct `ProviderPool` call)?
- [ ] Is the agent defined as an `AgentDefinition` (TOML, not ad-hoc Rust trait)?
- [ ] Are prompts externalized (not hardcoded in `.rs` files)?
- [ ] Does the agent follow the naming convention (§3.3)?
- [ ] Is the mode appropriate for the task (§3.4)?
- [ ] If creating dynamic agents: does permission inheritance hold (§4.2)?
- [ ] If creating dynamic agents: is the validation pipeline invoked (§4.4)?
- [ ] Are resource limits set conservatively for system agents?
