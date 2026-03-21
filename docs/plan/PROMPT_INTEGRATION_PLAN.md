# Prompt System Integration Plan

**Version**: v0.1
**Created**: 2026-03-10
**Status**: Approved

## Context

Running `y-agent chat` sends bare user messages to the LLM with no system prompt. The design docs (`prompt-design.md`, `context-session-design.md`) specify a full Context Assembly Pipeline with `BuildSystemPrompt` at priority 100, lazy-loaded sections, mode overlays, and token budgets. The infrastructure (`y-prompt` crate, `ContextPipeline`, `ContextProvider` trait) is fully implemented and tested, but three things are missing:

1. **No `BuildSystemPromptProvider`** — no concrete `ContextProvider` that uses `y-prompt` to assemble a system prompt
2. **No wiring** — `wire.rs` creates an empty `ContextPipeline::new()` with zero providers
3. **No chat integration** — `chat.rs` never calls `context_pipeline.assemble()` and never injects a `Role::System` message

This plan bridges those gaps as a Phase 1 (no template inheritance, no TOML config file loading, no caching).

---

## Architecture Decision

**`BuildSystemPromptProvider` lives in `y-context`** (with `y-prompt` as new dependency).

- `ContextProvider` trait is in `y-context`; all concrete providers (`InjectContextStatus`, `InputEnrichmentProvider`) live alongside it.
- Keeps y-cli as pure wiring — no business logic.
- No circular dependency: `y-context → y-prompt → y-core`.

**Per-turn state via `Arc<RwLock<PromptContext>>`** shared between `AppServices` and the provider.

- `ContextProvider::provide()` only takes `&mut AssembledContext` — no way to pass per-turn params through the trait.
- The chat loop writes the `PromptContext` before each `assemble()` call; the provider reads it inside `provide()`.
- Same pattern as `ContextMiddlewareAdapter`'s `Arc<Mutex<AssembledContext>>`.

**Dynamic sections (datetime, environment) handled inside the provider** — not via template variable interpolation (Open Question #1 in design doc).

---

## Implementation Steps (TDD Order)

### Step 1: Dependency Wiring (Cargo.toml)

Add `y-prompt` as dependency to `y-context` and `y-cli`.

**Files:**
- `Cargo.toml` (workspace root) — add `y-prompt = { path = "crates/y-prompt" }` to `[workspace.dependencies]`
- `crates/y-context/Cargo.toml` — add `y-prompt = { workspace = true }` + `chrono = { workspace = true }`
- `crates/y-cli/Cargo.toml` — add `y-prompt = { workspace = true }`

**Verify:** `cargo check -p y-context -p y-cli`

---

### Step 2: `HasTool("*")` Wildcard Support

The design doc specifies `core.tool_protocol` has condition `Always` (protocol and behavior rules are merged into one section). Current `SectionCondition::HasTool` only matches a specific tool name.

**File:** `crates/y-prompt/src/section.rs`

- In `SectionCondition::evaluate()`, add: `HasTool(name) if name == "*"` → `!ctx.available_tools.is_empty()`

**Tests (RED first):**
- `test_condition_has_tool_wildcard_true` — wildcard with tools present
- `test_condition_has_tool_wildcard_false` — wildcard with empty tools

---

### Step 3: Built-in Sections & Default Template

Factory functions that create a pre-populated `SectionStore` and the default `PromptTemplate`.

**New file:** `crates/y-prompt/src/builtins.rs`

```rust
pub fn builtin_section_store() -> SectionStore  // 9 built-in sections
pub fn default_template() -> PromptTemplate     // references the built-ins
```

9 built-in sections (all `ContentSource::Inline`):

| ID | Category | Priority | Budget | Condition |
|----|----------|----------|--------|-----------|
| `core.identity` | Identity | 100 | 200 | Always |
| `core.datetime` | Context | 150 | 50 | Always |
| `core.environment` | Context | 200 | 300 | Always |
| `core.guidelines` | Behavioral | 300 | 500 | Always |
| `core.safety` | Behavioral | 400 | 300 | Always |
| `core.tool_protocol` | Behavioral | 450 | 800 | Always |
| `core.persona` | Domain | 250 | 500 | ConfigFlag("persona.enabled") |
| `core.planning` | Behavioral | 350 | 300 | ModeIs("plan") |
| `core.exploration` | Behavioral | 350 | 200 | ModeIs("explore") |

`core.datetime` and `core.environment` use **placeholder inline content** ("{{datetime}}", "{{environment}}") — the provider replaces them dynamically at assembly time.

Default template: mode overlays for `plan` (include `core.planning`) and `explore` (exclude `core.safety`, include `core.exploration`, budget override 2000).

**Modify:** `crates/y-prompt/src/lib.rs` — add `pub mod builtins;` and re-exports.

**Tests:**
- `test_builtin_store_has_9_sections`
- `test_builtin_sections_have_inline_content`
- `test_default_template_general_mode` — 9 sections active (identity, datetime, environment, guidelines, safety, tool_protocol, persona, planning, exploration)
- `test_default_template_plan_mode` — includes planning
- `test_default_template_explore_mode` — excludes safety, includes exploration, budget=2000

---

### Step 4: `BuildSystemPromptProvider`

The core integration — a `ContextProvider` that bridges `y-prompt` into the pipeline.

**New file:** `crates/y-context/src/system_prompt.rs`

```rust
pub struct SystemPromptConfig {
    pub prompt_templates_enabled: bool,   // feature flag
    pub fallback_prompt: String,          // used when disabled or all sections excluded
}

pub struct BuildSystemPromptProvider {
    template: PromptTemplate,
    store: SectionStore,
    prompt_context: Arc<RwLock<PromptContext>>,
    config: SystemPromptConfig,
}
```

`provide()` algorithm:
1. If `!config.prompt_templates_enabled` → emit `fallback_prompt`, return.
2. Read `PromptContext` from `Arc<RwLock<>>`.
3. `template.effective_sections(mode)` → active section list.
4. For each section (sorted by priority):
   - Look up in `store.get(id)` → skip if not found.
   - Evaluate condition against `PromptContext` → skip if false.
   - Load content via `store.load_content(id)` → skip if error.
   - **Dynamic replacement:** if id is `core.datetime` → replace with `chrono::Utc::now()` formatted string. If `core.environment` → replace with `std::env::consts::OS`, cwd, etc.
   - Token budget: `estimate_tokens()`, `truncate_to_budget()` if over section budget.
   - Total budget check: stop if cumulative tokens exceed `template.effective_budget(mode)`.
   - Append to accumulated prompt string.
5. If accumulated is empty → use `fallback_prompt`.
6. Emit single `ContextItem { category: SystemPrompt, content, token_estimate, priority: 100 }`.

**Modify:** `crates/y-context/src/lib.rs` — add `pub mod system_prompt;` and re-exports.

**Tests:**
- `test_provider_name_and_priority` — name="build_system_prompt", priority=100
- `test_provide_emits_system_prompt` — with built-in store, emits content containing identity/guidelines/safety
- `test_conditions_exclude_sections` — plan mode includes planning
- `test_per_section_budget_truncates` — oversized section gets `[truncated]` marker
- `test_total_budget_drops_low_priority` — sections beyond total budget are dropped
- `test_all_excluded_uses_fallback` — empty PromptContext with all conditions false → fallback
- `test_feature_flag_disabled_uses_fallback` — `prompt_templates_enabled=false` → fallback only
- `test_mode_overlay_applied` — explore mode has different sections than general
- `test_dynamic_datetime_replaced` — output contains current date, not placeholder
- `test_missing_section_skipped` — template references nonexistent section → skip, no error

---

### Step 5: Wiring in `wire.rs`

Register providers into the pipeline and expose shared `PromptContext`.

**File:** `crates/y-cli/src/wire.rs`

Changes:
1. Import `y_prompt::{builtin_section_store, default_template, PromptContext}` and `y_context::{BuildSystemPromptProvider, InjectContextStatus, SystemPromptConfig}`.
2. Add `prompt_context: Arc<tokio::sync::RwLock<PromptContext>>` to `AppServices`.
3. In `wire()`, replace the empty pipeline construction:
   ```rust
   let prompt_context = Arc::new(RwLock::new(PromptContext::default()));
   let mut context_pipeline = ContextPipeline::new();
   context_pipeline.register(Box::new(BuildSystemPromptProvider::new(
       default_template(),
       builtin_section_store(),
       Arc::clone(&prompt_context),
       SystemPromptConfig::default(),
   )));
   context_pipeline.register(Box::new(InjectContextStatus::new(4096)));
   ```

**Tests:**
- `test_wire_registers_context_providers` — `pipeline.provider_count() == 2`

---

### Step 6: Chat Command Integration

Call the pipeline and prepend `Role::System` message.

**File:** `crates/y-cli/src/commands/chat.rs`

Changes:

1. **Before the chat loop** — initialize `PromptContext`:
   ```rust
   let initial_ctx = PromptContext {
       agent_mode: "general".into(),
       active_skills: vec![],
       available_tools: services.tool_registry.tool_names().await,
       config_flags: HashMap::new(),
   };
   *services.prompt_context.write().await = initial_ctx;
   ```

2. **Extract helper function** (testable without stdin):
   ```rust
   fn build_chat_messages(assembled: &AssembledContext, history: &[Message]) -> Vec<Message>
   ```
   Filters `ContextCategory::SystemPrompt` items → joins as one `Role::System` message → prepends before history.

3. **In the chat loop** — before building `ChatRequest`:
   ```rust
   let assembled = services.context_pipeline.assemble().await
       .unwrap_or_else(|e| { warn!(...); AssembledContext::default() });
   let messages = build_chat_messages(&assembled, &history);
   let request = ChatRequest { messages, ... };
   ```

Note: System message is assembled fresh each turn, NOT persisted to `history`. This is correct — the system prompt may change between turns.

**Tests:**
- `test_build_chat_messages_prepends_system` — system prompt from assembled context appears first
- `test_build_chat_messages_no_system_when_empty` — empty assembled context → no system message, just history
- `test_build_chat_messages_preserves_history_order` — history messages follow system message in order

---

## File Summary

| Action | File |
|--------|------|
| Modify | `Cargo.toml` (workspace) |
| Modify | `crates/y-context/Cargo.toml` |
| Modify | `crates/y-cli/Cargo.toml` |
| Modify | `crates/y-prompt/src/section.rs` — wildcard HasTool |
| Create | `crates/y-prompt/src/builtins.rs` — factory functions |
| Modify | `crates/y-prompt/src/lib.rs` — re-exports |
| Create | `crates/y-context/src/system_prompt.rs` — BuildSystemPromptProvider |
| Modify | `crates/y-context/src/lib.rs` — re-exports |
| Modify | `crates/y-cli/src/wire.rs` — register providers, add prompt_context |
| Modify | `crates/y-cli/src/commands/chat.rs` — pipeline call, system message |

## Verification

1. `cargo check --workspace` — compilation passes
2. `cargo test -p y-prompt` — all new + existing tests pass
3. `cargo test -p y-context` — all new + existing tests pass
4. `cargo test -p y-cli` — all new + existing tests pass
5. Manual: `cargo run -- chat` → `/status` shows providers → send a message → LLM response reflects system prompt persona (e.g., "You are y-agent")
6. Manual: set `prompt_templates_enabled: false` → verify fallback prompt is used
