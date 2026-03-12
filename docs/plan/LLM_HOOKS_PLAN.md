# LLM Hook Handlers Implementation Plan

> Complete prompt and agent hook handlers by defining DI traits in y-core, implementing adapters in y-provider and y-agent-core, and wiring them into y-hooks at startup.

**Design reference**: [hooks-plugin-design.md §LLM Hook Integration Architecture](file:///Users/gorgias/Projects/y-agent/docs/design/hooks-plugin-design.md)  
**Depends on**: y-hooks refactoring (H1-H5 complete), y-provider ProviderPool, y-agent-core agent loop  
**Estimated effort**: 2-3 weeks  

---

## Context

Prompt and agent hook handlers are currently **stubbed** in `y-hooks/src/hook_handler.rs` — they validate config and return default-allow. Full implementation requires:

1. **Prompt hooks** need LLM access (`ProviderPool::chat_completion`) to send single-turn evaluation requests.
2. **Agent hooks** need agent loop access (spawn subagent with restricted read-only tools, max 50 turns).

Direct dependencies would create circular crate graphs. The solution is trait-based dependency injection defined in `y-core`, with implementations in downstream crates.

---

## Phases

### L1: Define DI Traits in y-core (1d)

#### [MODIFY] [hook.rs](file:///Users/gorgias/Projects/y-agent/crates/y-core/src/hook.rs)

Add two async traits at the end of the file:

```rust
/// Single-turn LLM evaluation for prompt hooks.
#[async_trait]
pub trait HookLlmRunner: Send + Sync {
    async fn evaluate(
        &self,
        system_prompt: &str,
        user_message: &str,
        model: Option<&str>,
        timeout: Duration,
    ) -> Result<String, String>;
}

/// Multi-turn subagent execution for agent hooks.
#[async_trait]
pub trait HookAgentRunner: Send + Sync {
    async fn run_agent(
        &self,
        task_prompt: &str,
        model: Option<&str>,
        max_turns: u32,
        timeout: Duration,
    ) -> Result<String, String>;
}
```

**Tests**: Trait compilation test only (no impl yet).

---

### L2: Accept Runners in HookHandlerExecutor (1d)

#### [MODIFY] [hook_handler.rs](file:///Users/gorgias/Projects/y-agent/crates/y-hooks/src/hook_handler.rs)

1. Add `llm_runner: Option<Arc<dyn HookLlmRunner>>` and `agent_runner: Option<Arc<dyn HookAgentRunner>>` fields.
2. Update `from_config()` to accept optional runners (or add separate `set_llm_runner()`/`set_agent_runner()` methods).
3. Replace `execute_prompt()` stub with real implementation:
   - Build system prompt: "Respond with JSON: `{\"ok\": true/false, \"reason\": \"...\"}`"
   - Replace `$ARGUMENTS` in user prompt template
   - Call `llm_runner.evaluate()` with timeout
   - Parse response as `PromptAgentDecision`
4. Replace `execute_agent()` stub with real implementation:
   - Replace `$ARGUMENTS` in task prompt
   - Call `agent_runner.run_agent()` with max_turns=50 and timeout
   - Parse response as `PromptAgentDecision`

#### [MODIFY] [hook_system.rs](file:///Users/gorgias/Projects/y-agent/crates/y-hooks/src/hook_system.rs)

Add `set_llm_runner()` and `set_agent_runner()` methods that forward to the executor.

**Tests**: 
- Unit test: prompt hook with mock runner → parse allow/block
- Unit test: agent hook with mock runner → parse allow/block
- Unit test: missing runner → log warning, return default allow

---

### L3: Implement HookLlmRunner in y-provider (2d)

#### [NEW] [hook_llm_runner.rs](file:///Users/gorgias/Projects/y-agent/crates/y-provider/src/hook_llm_runner.rs)

```rust
pub struct ProviderPoolHookLlmRunner {
    pool: Arc<dyn ProviderPool>,
}

impl HookLlmRunner for ProviderPoolHookLlmRunner {
    async fn evaluate(&self, system, user, model, timeout) -> Result<String, String> {
        // Build ChatRequest with system + user messages
        // Route with preferred_model = model, required_tags = ["fast"]
        // tokio::time::timeout(timeout, pool.chat_completion(...))
        // Return response.content
    }
}
```

**Tests**:
- Mock ProviderPool → verify request format
- Timeout handling
- Error propagation

---

### L4: Implement HookAgentRunner in y-agent-core (3d)

#### [NEW] [hook_agent_runner.rs](file:///Users/gorgias/Projects/y-agent/crates/y-agent-core/src/hook_agent_runner.rs)

```rust
pub struct SubagentHookRunner {
    pool: Arc<dyn ProviderPool>,
    // Read-only tool registry (Read, Grep, Glob only)
}

impl HookAgentRunner for SubagentHookRunner {
    async fn run_agent(&self, prompt, model, max_turns, timeout) -> Result<String, String> {
        // Create restricted tool set (Read, Grep, Glob)
        // Run agent loop with max_turns cap
        // Extract final structured response
        // Return JSON string
    }
}
```

**Depends on**: y-agent-core agent loop being functional.

**Tests**:
- Mock agent loop → verify restricted tools
- Max turns cap
- Timeout handling

---

### L5: Wire at Startup (1d)

#### [MODIFY] startup / y-cli initialization

After `HookSystem::new()` and `ProviderPoolImpl::from_config()`:

```rust
let llm_runner = Arc::new(ProviderPoolHookLlmRunner::new(pool.clone()));
let agent_runner = Arc::new(SubagentHookRunner::new(pool.clone()));
hook_system.set_llm_runner(llm_runner);
hook_system.set_agent_runner(agent_runner);
```

**Tests**: Integration test with real prompt hook config → verify LLM called.

---

### L6: End-to-End Verification (1d)

- `cargo check --workspace`
- `cargo test -p y-core -p y-hooks -p y-provider -p y-agent-core`
- `cargo clippy --workspace -- -W warnings`
- Feature flag isolation: `--features hook_handlers` without `llm_hooks` → prompt/agent skipped gracefully
- Feature flag: `--features llm_hooks` → prompt/agent compile and execute

---

## Dependency Graph

```
L1 (y-core traits)
 ├── L2 (y-hooks accepts runners)
 │    ├── L3 (y-provider impl)
 │    └── L4 (y-agent-core impl)
 └── L5 (startup wiring) ← depends on L2, L3, L4
      └── L6 (verification) ← depends on all
```

L3 and L4 can be developed in parallel after L1+L2 are complete.

---

## Risk and Mitigation

| Risk | Impact | Mitigation |
|------|--------|-----------|
| y-agent-core agent loop not ready for L4 | Agent hooks remain stubbed | L4 can be deferred; prompt hooks (L3) work independently |
| Circular dependency despite DI | Build failure | Traits in y-core guarantee unidirectional dependencies |
| LLM response not valid JSON | Prompt/agent hooks default-allow | Parse error → log + default allow per design §Failure Handling |
