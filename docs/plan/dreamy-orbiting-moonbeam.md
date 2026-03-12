# Plan: Prompt-Based Tool Calling Protocol

## Context

y-agent currently sends tool definitions via the OpenAI-compatible `tools` field in HTTP request bodies to LLM providers. This causes three problems:

1. **Provider incompatibility** — Some LLMs (e.g., DeepSeek) don't support or mishandle the `tools` field. The field is an OpenAI convention, not a universal standard.
2. **Token waste** — `ChatService::build_tool_definitions()` serializes ALL registered tool definitions into every request, consuming thousands of tokens per turn.
3. **Accidental complexity** — Each provider (OpenAI, Anthropic, Gemini, Ollama) transforms tool JSON to its own native format, creating N provider-specific translation layers.

The intended outcome: y-agent communicates tool usage to ANY LLM entirely through the system prompt, using a standardized text protocol that works regardless of provider API. Tool definitions are lazy-loaded via a hierarchical taxonomy search, not eagerly included.

The existing codebase already has significant infrastructure for this:
- `ToolIndex` + `tool_search` meta-tool + `ToolActivationSet` — lazy loading primitives
- `InjectTools` context pipeline stage — injects tool list into prompt
- `PromptSection` system with conditional activation — can add protocol sections

What's **missing** is:
- A text-based tool call protocol (prompt template + output format + parser)
- Hierarchical taxonomy for tool search (currently flat keyword match)
- Removing the `tools` field from HTTP requests (or making it configurable)
- A standard document codifying the protocol

---

## Deliverables

### D1: Standard — `docs/standards/TOOL_CALL_PROTOCOL.md`
Universal tool calling protocol that works with any LLM via prompt engineering.

### D2: Design — `docs/design/tool-search-design.md`
Hierarchical tool taxonomy and search mechanism design.

### D3: Implementation — Prompt-based tool calling mode in Rust

---

## Phase 1: Standards & Design Documents

### Step 1.1: Create `docs/standards/TOOL_CALL_PROTOCOL.md`

Defines the provider-agnostic tool calling protocol:

**Tool Call Output Format** — XML-like tags (most reliably parsed, least ambiguous):
```
<tool_call>
{"name": "file_read", "arguments": {"path": "/src/main.rs"}}
</tool_call>
```

Why XML tags over alternatives:
- JSON code blocks (` ```json `) — ambiguous, LLMs produce these for non-tool purposes
- Custom delimiters — fragile, model-dependent
- XML tags — distinct from normal text, easy to regex, well-understood by all LLMs

**Protocol sections to document:**
1. **Tool Call Format** — How LLM expresses intent to call a tool
2. **Tool Result Format** — How tool results are fed back (as `<tool_result>` blocks in user/tool messages)
3. **Tool Search Protocol** — How LLM discovers tools (call `tool_search` with category path or keyword)
4. **Multi-call** — LLM can emit multiple `<tool_call>` blocks in one response
5. **Error Handling** — Standard error result format
6. **Parsing Rules** — Regex patterns, edge cases, validation

### Step 1.2: Create `docs/design/tool-search-design.md`

Hierarchical tool taxonomy design:

**Tree structure (TOML-based):**
```
[categories.file]
label = "File Management"
description = "Read, write, search, and manage files"

[categories.file.subcategories.read]
label = "File Reading"
tools = ["file_read", "file_list"]

[categories.file.subcategories.write]
label = "File Writing"
tools = ["file_write"]

[categories.file.subcategories.search]
label = "File Search"
tools = ["file_search"]

[categories.network]
label = "Network"
description = "HTTP requests, DNS, connectivity"

[categories.shell]
label = "Shell"
description = "Execute shell commands"
tools = ["shell_exec"]

[categories.memory]
label = "Memory & Knowledge"
description = "Store, recall, and search knowledge"

[categories.agent]
label = "Agent & Workflow"
description = "Delegate to sub-agents, manage workflows"

[categories.meta]
label = "Meta Tools"
description = "Tool management tools"
tools = ["tool_search", "tool_create"]
```

**Search flow:**
1. LLM sees taxonomy root in prompt (just category names + descriptions, ~100 tokens)
2. LLM calls `tool_search(category: "file")` → gets subcategories + tool summaries
3. LLM calls `tool_search(tool: "file_read")` → gets full tool schema
4. Full schema injected into ToolActivationSet, available for subsequent calls

**Sub-agent for tool search** — Uses existing AgentDelegator pattern:
- Built-in system sub-agent: `tool-searcher`
- Mode: `explore` (read-only, search tools only)
- Input: search query / category path
- Output: structured JSON with matching tools

### Step 1.3: Update `docs/design/tools-design.md`

Move "LLM tool call parsing and function calling format" from Out of Scope to In Scope. Add cross-reference to TOOL_CALL_PROTOCOL.md.

---

## Phase 2: Core Types & Parser (TDD)

### Step 2.1: Add `ToolCallingMode` to `crates/y-core/src/provider.rs`

```rust
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ToolCallingMode {
    /// Tool calling via system prompt text protocol (universal, works with any LLM)
    #[default]
    PromptBased,
    /// Tool calling via provider-native API fields (OpenAI tools, Anthropic tools)
    Native,
}
```

Add `tool_calling_mode: ToolCallingMode` to `ChatRequest`.

**Key file:** [crates/y-core/src/provider.rs](crates/y-core/src/provider.rs)

### Step 2.2: Create `crates/y-tools/src/parser.rs` — Tool call text parser

Parse `<tool_call>...</tool_call>` blocks from LLM text output.

```rust
pub struct ParsedToolCall {
    pub name: String,
    pub arguments: serde_json::Value,
}

pub struct ParseResult {
    /// Text content with tool_call blocks removed
    pub text: String,
    /// Extracted tool calls in order
    pub tool_calls: Vec<ParsedToolCall>,
}

pub trait ToolCallParser: Send + Sync {
    fn parse(&self, raw_text: &str) -> ParseResult;
}

pub struct XmlTagParser; // Default implementation
```

Edge cases to handle:
- Multiple tool calls in one response
- Tool call mixed with regular text
- Malformed/incomplete tags (fail gracefully, treat as text)
- Nested angle brackets in JSON arguments (parser must handle this)
- Empty tool call blocks

**Key file:** [crates/y-tools/src/parser.rs](crates/y-tools/src/parser.rs) (NEW)

### Step 2.3: Create `crates/y-tools/src/taxonomy.rs` — Hierarchical tool taxonomy

```rust
pub struct ToolTaxonomy {
    categories: HashMap<String, TaxonomyCategory>,
}

pub struct TaxonomyCategory {
    pub label: String,
    pub description: String,
    pub subcategories: HashMap<String, TaxonomySubcategory>,
    pub tools: Vec<ToolName>,  // Tools directly in this category
}

pub struct TaxonomySubcategory {
    pub label: String,
    pub description: String,
    pub tools: Vec<ToolName>,
}

impl ToolTaxonomy {
    pub fn from_toml(config: &str) -> Result<Self, TaxonomyError>;
    pub fn root_summary(&self) -> String;  // For prompt injection (~100 tokens)
    pub fn category_detail(&self, category: &str) -> Option<String>;
    pub fn search(&self, query: &str) -> Vec<ToolName>;
}
```

**Key file:** [crates/y-tools/src/taxonomy.rs](crates/y-tools/src/taxonomy.rs) (NEW)

---

## Phase 3: Prompt Integration

### Step 3.1: Add `core.tool_protocol` prompt section in `crates/y-prompt/src/builtins.rs`

New section with condition `SectionCondition::Always` (tool protocol is always needed):

Content teaches the LLM:
1. How to call tools (XML tag format)
2. How to search for tools (tool_search with taxonomy)
3. The taxonomy root (category list)
4. How tool results will appear

**Key file:** [crates/y-prompt/src/builtins.rs](crates/y-prompt/src/builtins.rs)

### Step 3.2: Enhance `crates/y-context/src/inject_tools.rs`

In `PromptBased` mode:
- Inject taxonomy root summary (not flat tool list)
- Inject tool_search usage instructions
- Inject any currently-activated tool schemas (from ToolActivationSet)

In `Native` mode (backward compat):
- Keep current behavior (flat tool list, tools field in request)

**Key file:** [crates/y-context/src/inject_tools.rs](crates/y-context/src/inject_tools.rs)

---

## Phase 4: Service Layer Changes

### Step 4.1: Modify `crates/y-service/src/chat.rs`

`ChatService::execute_turn()` changes:

```
PromptBased mode:
  1. Don't call build_tool_definitions() → tools field stays empty
  2. After LLM response, run ToolCallParser on response.content
  3. If parser finds tool_calls → execute them, format results as <tool_result> blocks
  4. Append tool results to history and loop

Native mode:
  1. Keep current behavior (build_tool_definitions, vendor tool_calls parsing)
```

**Key file:** [crates/y-service/src/chat.rs](crates/y-service/src/chat.rs)

### Step 4.2: Enhance `tool_search` meta-tool

Update `crates/y-tools/src/builtin/tool_search.rs`:
- Accept `category` parameter for taxonomy navigation
- Accept `tool` parameter for specific tool schema retrieval
- Return structured results with taxonomy context

**Key file:** [crates/y-tools/src/builtin/tool_search.rs](crates/y-tools/src/builtin/tool_search.rs)

### Step 4.3: Provider adjustments

In `crates/y-provider/src/providers/openai.rs` (and other providers):
- When `tool_calling_mode == PromptBased`, set `tools: None` in the request body
- When `Native`, keep current behavior

**Key files:**
- [crates/y-provider/src/providers/openai.rs](crates/y-provider/src/providers/openai.rs)
- [crates/y-provider/src/providers/anthropic.rs](crates/y-provider/src/providers/anthropic.rs)
- [crates/y-provider/src/providers/gemini.rs](crates/y-provider/src/providers/gemini.rs)
- [crates/y-provider/src/providers/ollama.rs](crates/y-provider/src/providers/ollama.rs)

---

## Phase 5: Configuration

### Step 5.1: Create `config/tool_taxonomy.toml`

Default hierarchical tool taxonomy with categories:
- `file` — File Management (read, write, list, search)
- `shell` — Shell Execution
- `network` — Network Operations
- `memory` — Memory & Knowledge
- `search` — Search & Retrieval
- `agent` — Agent & Workflow
- `meta` — Meta Tools (tool_search, tool_create)

**Key file:** [config/tool_taxonomy.toml](config/tool_taxonomy.toml) (NEW)

### Step 5.2: Add `tool_calling_mode` to provider config

In provider TOML config, allow per-provider or global setting:
```toml
[tool_calling]
mode = "prompt_based"  # or "native"
```

**Key file:** [crates/y-provider/src/config.rs](crates/y-provider/src/config.rs)

---

## Implementation Order

1. **Docs first**: TOOL_CALL_PROTOCOL.md → tool-search-design.md → update tools-design.md
2. **Core types**: ToolCallingMode in y-core
3. **Parser (TDD)**: parser.rs with comprehensive tests (this is the most critical new code)
4. **Taxonomy (TDD)**: taxonomy.rs with TOML loading + tree navigation
5. **Prompt section**: core.tool_protocol in builtins.rs
6. **InjectTools**: Enhance for PromptBased mode
7. **ChatService**: Dual-mode tool call handling
8. **tool_search**: Taxonomy-aware search
9. **Providers**: Conditional tools field
10. **Config**: tool_taxonomy.toml + tool_calling_mode setting

---

## Verification

1. **Unit tests**: Parser handles all edge cases (malformed tags, multiple calls, nested JSON)
2. **Unit tests**: Taxonomy loads from TOML, root summary fits in ~100 tokens
3. **Unit tests**: ChatService in PromptBased mode doesn't send tools field
4. **Integration test**: Full turn cycle — LLM text → parse tool call → execute → format result → feed back
5. **Manual test**: Connect to DeepSeek (or any OpenAI-compatible API without native tools support) and verify tool calling works via prompt protocol
6. **Token measurement**: Compare token usage before/after (expect 60-90% reduction on initial turn)
