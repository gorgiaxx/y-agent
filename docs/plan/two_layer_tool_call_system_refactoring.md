# Two-Layer Tool Call System Refactoring

## Context

LLMs frequently produce malformed XML `<tool_call>` tags. The current system defaults to `PromptBased` tool calling for all providers, relying on XML parsing even when providers have robust native tool calling APIs (OpenAI, Anthropic, etc.). This wastes tokens on XML protocol injection and produces unreliable tool calls.

**Goal**: Switch to a two-layer approach:

- Layer 1 (API type): Providers with native tool calling APIs (OpenAI, Anthropic) send/receive tool calls via their API format
- Layer 2 (XML tags): Providers without native support fall back to the current XML-based prompt injection + lenient parser

**Key discovery**: The codebase already has ~90% of the infrastructure. Both provider implementations (openai.rs, anthropic.rs) already handle Native mode. The agent_service.rs execution loop already has a fallback path that parses XML even in Native mode. The main work is: changing defaults, adding auto-detection, gating prompt injection, and updating the GUI.

---

## Phase 1: Core -- Change Default and Add Auto-Detection

### 1.1 Change `ToolCallingMode` default to `Native`

**File**: `crates/y-core/src/provider.rs:26-38`

Move `#[default]` from `PromptBased` to `Native`. Update doc comments.

### 1.2 Add `tool_calling_mode` to `ProviderMetadata`

**File**: `crates/y-core/src/provider.rs:147-156`

Add `pub tool_calling_mode: ToolCallingMode` to `ProviderMetadata`. Update all provider constructors to pass the resolved mode.

### 1.3 Add `resolve_tool_calling_mode()` to `ProviderConfig`

**File**: `crates/y-provider/src/config.rs`

```rust
pub fn resolve_tool_calling_mode(&self) -> ToolCallingMode {
    if let Some(mode) = self.tool_calling_mode {
        return mode;
    }
    match self.provider_type.as_str() {
        "openai" | "anthropic" | "azure" | "gemini" | "deepseek" => ToolCallingMode::Native,
        // openai-compat, custom, ollama default to PromptBased
        _ => ToolCallingMode::PromptBased,
    }
}
```

Update doc comment on `tool_calling_mode` field to describe auto-detection.

### 1.4 Wire resolved mode through provider construction

**File**: `crates/y-service/src/container.rs` -- `build_providers_from_config()`

Pass `cfg.resolve_tool_calling_mode()` to each provider constructor so it gets stored in `ProviderMetadata.tool_calling_mode`.

**Files**: All provider constructors (`openai.rs`, `anthropic.rs`, `gemini.rs`, `ollama.rs`, `azure.rs`) -- add `tool_calling_mode: ToolCallingMode` parameter, store in `ProviderMetadata`.

### 1.5 Expose mode on ProviderPool

**File**: `crates/y-provider/src/pool.rs`

Add method to `ProviderPoolImpl`:

```rust
pub fn provider_tool_calling_mode(&self, provider_id: &ProviderId) -> ToolCallingMode
```

Also add to the `ProviderPool` trait if needed. This lets the service layer query the selected provider's mode.

---

## Phase 2: Service Layer -- Mode-Aware Execution

### 2.1 Update root chat turn in `chat.rs`

**File**: `crates/y-service/src/chat.rs:643-647`

Currently:

```rust
let tool_calling_mode = ToolCallingMode::default();
let tool_defs = match tool_calling_mode {
    ToolCallingMode::Native => Self::build_tool_definitions(container).await,
    ToolCallingMode::PromptBased => vec![],
};
```

Change to always build tool definitions. The tool_calling_mode default is now Native. For PromptBased providers, the provider's `build_request_body()` already ignores the tools field. The fallback parser catches any XML tool calls.

```rust
let tool_calling_mode = ToolCallingMode::default(); // now Native
let tool_defs = Self::build_tool_definitions(container).await;
```

### 2.2 Conditionally inject tool protocol prompt

**File**: `crates/y-service/src/container.rs:331-341`

Currently always registers `InjectTools::with_taxonomy_and_core_tools()` (PromptBased mode). Need to make this mode-aware.

Determine the predominant tool calling mode from configured providers. If all are Native, use the lightweight Native injection. If any are PromptBased, keep the taxonomy + core tools injection.

Also need to conditionally exclude `core.tool_protocol` (~800 tokens) from the system prompt when mode is Native. This is registered in `builtins.rs` as a built-in prompt section.

Approach: Add a flag/method to `BuildSystemPromptProvider` to skip the `core.tool_protocol` section, and set it based on the resolved mode.

**File**: `crates/y-prompt/src/builtins.rs` -- Make `core.tool_protocol` section conditional. Add a method like `exclude_section(name)` or a condition flag.

**File**: `crates/y-prompt/src/lib.rs` -- If needed, expose the ability to conditionally exclude sections.

### 2.3 Skip tool protocol in sub-agent prompt (Native mode)

**File**: `crates/y-service/src/agent_service.rs:1551-1563`

`build_subagent_system_prompt()` currently always injects `PROMPT_TOOL_PROTOCOL`. Add a `tool_calling_mode` parameter:

```rust
fn build_subagent_system_prompt(
    base_prompt: &str,
    filtered_defs: &[ToolDefinition],
    tool_calling_mode: ToolCallingMode,
) -> String {
    if filtered_defs.is_empty() {
        return base_prompt.to_string();
    }
    let tools_summary = build_agent_tools_summary(filtered_defs);
    match tool_calling_mode {
        ToolCallingMode::Native => format!("{base_prompt}\n\n{tools_summary}"),
        ToolCallingMode::PromptBased => {
            let tool_protocol = y_prompt::PROMPT_TOOL_PROTOCOL;
            format!("{base_prompt}\n\n{tool_protocol}\n\n{tools_summary}")
        }
    }
}
```

Update call site at line 1601 and the `ServiceAgentRunner::run()` at line 1652-1657.

---

## Phase 3: GUI Changes -- Native Tool Call Rendering

### 3.1 Distinguish native vs XML tool calls in streaming

**File**: `crates/y-gui/src/hooks/useStreamContent.ts`

The existing XML parser (`processStreamContent`) still runs on all content. In Native mode it will find zero XML blocks -- this is fine. No change strictly needed for correctness.

However, the `hasPendingToolCall` flag (which buffers content when a partial `<tool_call` prefix is detected) should be suppressed when the provider uses Native mode. This prevents unnecessary buffering delays.

**Approach**: Pass a `toolCallingMode` flag from the backend to the frontend. Add it to the `chat:progress` stream_start event or to the session/provider metadata. When `mode === 'native'`, skip the XML pending-tag detection in `processStreamContent`.

### 3.2 Backend: Emit tool_calling_mode in progress events

**File**: `crates/y-service/src/chat.rs` -- `TurnEvent::StreamStart` or similar

Add `tool_calling_mode: String` ("native" | "prompt_based") to the stream start event so the GUI knows which rendering path to use.

**File**: `crates/y-gui/src-tauri/src/commands/agents.rs` -- Pass through in the Tauri event emission.

### 3.3 Frontend: Mode-aware rendering

**File**: `crates/y-gui/src/hooks/useStreamContent.ts`

When `toolCallingMode === 'native'`:

- Skip the `hasPendingToolCall` detection (no XML tags expected)
- Tool calls come via `message.tool_calls` (already rendered by ToolCallCard)

When `toolCallingMode === 'prompt_based'`:

- Keep current behavior (XML parsing + delayed rendering)

### 3.4 ActionCard: Handle native tool call display

**File**: `crates/y-gui/src/components/chat-panel/chat-box/ActionCard.tsx`

For Native mode, tool calls are not embedded in the text content. They arrive as structured `ToolCallBrief[]` on the message object. The `useAssistantBubble` hook needs to be aware:

- In Native mode: build action segments from `message.tool_calls` + `toolResults` without XML extraction
- In PromptBased mode: keep current XML extraction logic

**File**: `crates/y-gui/src/components/chat-panel/chat-box/useAssistantBubble.ts`

Add a branch for Native mode that builds segments from `message.tool_calls` directly instead of parsing XML from content.

---

## Phase 4: Config and Docs

### 4.1 Update providers.example.toml

**File**: `config/providers.example.toml`

Add documented `tool_calling_mode` examples showing auto-detection behavior.

### 4.2 Update TOOL_CALL_PROTOCOL.md

**File**: `docs/standards/TOOL_CALL_PROTOCOL.md`

Document the two-layer system: Native is now default for first-party providers; PromptBased is fallback for compat/local providers.

---

## Phase 5: Tests

### 5.1 Unit tests for auto-detection

- `resolve_tool_calling_mode()` returns Native for "openai", "anthropic", "azure", "gemini", "deepseek"
- Returns PromptBased for "openai-compat", "custom", "ollama"
- Explicit override takes priority

### 5.2 Update existing tests

- Tests using `ToolCallingMode::default()` that assumed PromptBased
- `build_subagent_system_prompt` tests -- add cases for both modes
- Provider metadata tests -- verify `tool_calling_mode` field

### 5.3 Integration/behavioral tests

- Native mode + provider returns tool_calls -> handled by native path
- Native mode + model emits XML in text -> fallback parser catches it
- PromptBased mode -> XML protocol injected, parser handles response

---

## Files to Modify (in order)

| File                                                                    | Change                                                              |
| ----------------------------------------------------------------------- | ------------------------------------------------------------------- |
| `crates/y-core/src/provider.rs`                                         | Change default to Native; add tool_calling_mode to ProviderMetadata |
| `crates/y-provider/src/config.rs`                                       | Add `resolve_tool_calling_mode()`                                   |
| `crates/y-provider/src/providers/openai.rs`                             | Accept tool_calling_mode in constructor                             |
| `crates/y-provider/src/providers/anthropic.rs`                          | Accept tool_calling_mode in constructor                             |
| `crates/y-provider/src/providers/gemini.rs`                             | Accept tool_calling_mode in constructor                             |
| `crates/y-provider/src/providers/ollama.rs`                             | Accept tool_calling_mode in constructor                             |
| `crates/y-provider/src/providers/azure.rs`                              | Accept tool_calling_mode in constructor                             |
| `crates/y-provider/src/pool.rs`                                         | Add provider_tool_calling_mode() method                             |
| `crates/y-service/src/container.rs`                                     | Wire resolved mode; conditionally register InjectTools mode         |
| `crates/y-service/src/chat.rs`                                          | Always build tool_defs; emit mode in progress events                |
| `crates/y-service/src/agent_service.rs`                                 | Conditional sub-agent prompt injection                              |
| `crates/y-prompt/src/builtins.rs`                                       | Make core.tool_protocol conditional                                 |
| `crates/y-prompt/src/lib.rs`                                            | Expose conditional section control                                  |
| `crates/y-context/src/inject_tools.rs`                                  | Mode-aware injection (already supports both)                        |
| `crates/y-gui/src/hooks/useStreamContent.ts`                            | Mode-aware XML parsing                                              |
| `crates/y-gui/src/components/chat-panel/chat-box/useAssistantBubble.ts` | Native mode segment building                                        |
| `config/providers.example.toml`                                         | Document tool_calling_mode                                          |
| `docs/standards/TOOL_CALL_PROTOCOL.md`                                  | Document two-layer system                                           |

---

## Verification

1. `cargo clippy --workspace -- -D warnings`
2. `cargo check --workspace`
3. `cargo doc --workspace --no-deps`
4. `cargo fmt --all`
5. Manual testing:
   - Configure an OpenAI provider (Native default) -- verify tool calls work via API
   - Configure an openai-compat provider (PromptBased default) -- verify XML tool calls work
   - Override openai-compat with `tool_calling_mode = "native"` -- verify override works
   - Test fallback: Native mode provider where model emits XML -- verify parser catches it
6. GUI testing:
   - Native mode: tool calls render immediately without delayed XML buffering
   - PromptBased mode: existing XML parsing + delayed rendering still works
