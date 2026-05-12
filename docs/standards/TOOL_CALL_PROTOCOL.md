# Tool Call Protocol Standard

> Two-layer tool calling: native API for first-party providers, XML fallback for compatibility

**Version**: v0.3
**Created**: 2026-03-12
**Updated**: 2026-03-27
**Status**: Draft

---

## TL;DR

y-agent uses a **two-layer tool calling system**:

- **Layer 1 (Native)**: First-party providers (OpenAI, Anthropic, Azure, Gemini, DeepSeek)
  send tool definitions via their native API fields and extract structured tool calls
  from response objects. This is the **default** for these providers.

- **Layer 2 (PromptBased)**: Compatibility providers (openai-compat, custom, ollama)
  inject tool definitions into the system prompt and parse tool calls from XML tags
  (`<tool_call>`) in the LLM's text output. A lenient, multi-format parser handles
  variations across models.

The layer is auto-detected from `provider_type` in the configuration. Users can
override with `tool_calling_mode = "native"` or `tool_calling_mode = "prompt_based"`
per provider. A fallback path in the execution loop also attempts XML parsing even
in Native mode, ensuring tool calls are never missed.

---

## 1. Design Principles

| Principle                | Rationale                                                                                                                                                                                 |
| ------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Two-layer system**     | Native API tool calling for providers that support it; XML prompt-based fallback for the rest. Auto-detected from `provider_type`.                                                        |
| **Two-tier visibility**  | Core tools (Tier 1) are always available with schemas in the prompt. Extended tools (Tier 2) are loaded on demand via `ToolSearch`.                                                      |
| **Explicit format**      | XML tags are unambiguous, easy to parse, and well-understood by all major LLMs.                                                                                                           |
| **Native-first default** | `Native` is the default for first-party providers (openai, anthropic, azure, gemini, deepseek). `PromptBased` is the default for compatibility providers (openai-compat, custom, ollama). |
| **Fail gracefully**      | Malformed tool call tags are treated as regular text. Even in Native mode, a fallback XML parser catches tool calls emitted as text.                                                      |

---

## 2. Tool Call Format

When an LLM wants to invoke a tool, it outputs a `<tool_call>` block containing a JSON object with `name` and `arguments` fields.

### Single Tool Call

```
I need to read that file to understand the code structure.

<tool_call>
<name>FileRead</name>
<arguments>{"path": "/src/main.rs"}</arguments>
</tool_call>
```

### Multiple Tool Calls

Multiple `<tool_call>` blocks can appear in a single response. They are executed sequentially in order of appearance.

```
Let me check both files.

<tool_call>
<name>FileRead</name>
<arguments>{"path": "/src/lib.rs"}</arguments>
</tool_call>

<tool_call>
<name>FileRead</name>
<arguments>{"path": "/src/main.rs"}</arguments>
</tool_call>
```

The content inside `<tool_call>` uses XML tags for structure:

- `<name>` — Tool name (must match a registered or activated tool)
- `<arguments>` — JSON object with tool-specific parameters

The parser also accepts JSON format as a legacy fallback:

```json
{ "name": "tool_name", "arguments": { "param1": "value1" } }
```

---

## 3. Tool Result Format

After executing a tool call, the result is fed back to the LLM as a `<tool_result>` block in the next message.

### Success

```
<tool_result name="FileRead" success="true">
{"content": "fn main() {\n    println!(\"Hello, world!\");\n}"}
</tool_result>
```

### Error

```
<tool_result name="FileRead" success="false">
{"error": "file not found: /src/missing.rs"}
</tool_result>
```

### Attributes

| Attribute | Required | Description               |
| --------- | -------- | ------------------------- |
| `name`    | Yes      | Tool name that was called |
| `success` | Yes      | `"true"` or `"false"`     |

The body is always a JSON object. On success, the structure depends on the tool. On error, it contains an `error` field with a human-readable message.

---

## 4. Tool Search Protocol

Tools use a **two-tier visibility model**:

1. **Tier 1 (Core Tools)** -- Always available in the prompt with compact schemas. The LLM can call these directly without searching.
2. **Tier 2 (Extended Tools)** -- Discovered via `ToolSearch`. The LLM sees a taxonomy root (~100 tokens) and must search before calling.

### Search by Category

```
<tool_call>
<name>ToolSearch</name>
<arguments>{"category": "file"}</arguments>
</tool_call>
```

Returns subcategories and tool summaries within that category.

### Search by Keyword

```
<tool_call>
<name>ToolSearch</name>
<arguments>{"query": "read file contents"}</arguments>
</tool_call>
```

Returns matching tools across all categories.

### Get Full Tool Schema

```
<tool_call>
<name>ToolSearch</name>
<arguments>{"tool": "FileRead"}</arguments>
</tool_call>
```

Returns the full tool definition including parameter schema, description, and examples.

### After Search

Once a tool is retrieved via `ToolSearch`, its full schema is added to the **ToolActivationSet** (session-scoped LRU cache, ceiling 20). The LLM can then call the tool directly in subsequent turns.

---

## 5. Two-Tier Prompt Injection

### 5.1 Tier 1: Core Tool Schemas

The following core tools are always listed in the system prompt with compact schemas. This eliminates the common failure mode where LLMs guess familiar Unix command names (e.g., `ls`, `cat`, `grep`) instead of using registered tool names.

| Tool          | Description                            | Required Args                                 |
| ------------- | -------------------------------------- | --------------------------------------------- |
| `FileRead`   | Read file contents                     | `{"path": "<filepath>", "line_offset?": 0, "limit?": 2000, "include_line_numbers?": false}` |
| `FileWrite`  | Write content to a file (creates dirs) | `{"path": "<filepath>", "content": "<text>"}` |
| `FileList`   | List directory contents                | `{"path": "<dirpath>"}`                       |
| `FileSearch` | Search for text pattern in files       | `{"pattern": "<text>", "path": "<dirpath>"}`  |
| `ShellExec`  | Execute a shell command                | `{"command": "<cmd>"}`                        |

The prompt also includes an explicit instruction:

> IMPORTANT: Use ONLY these exact tool names. Do NOT invent tool names like 'ls', 'cat', 'grep', or 'mkdir'. For shell operations not covered above, use shell_exec.

### 5.2 Tier 2: Taxonomy Root

For extended tools, the taxonomy root is injected to inform the LLM of available categories:

```
## Tool Categories

You have access to tools organized in the following categories. Use `ToolSearch` to discover and load specific tools before using them.

| Category | Description |
|----------|-------------|
| file | File management -- read, write, list, search files |
| shell | Shell command execution |
| network | HTTP requests, DNS, connectivity |
| memory | Store and recall knowledge |
| search | Search and retrieval |
| agent | Sub-agent delegation, workflow management |
| meta | Tool management (search, create) |
```

---

## 6. Parsing Rules

### Extraction

1. Find all `<tool_call>...</tool_call>` blocks in the LLM response text
2. For each block, try to parse as XML-nested format (`<name>` + `<arguments>` tags)
3. If XML parsing fails, try to parse as JSON (legacy fallback)
4. Validate: must have non-empty name and arguments must be an object
5. Collect all valid tool calls in order of appearance
6. Separate text content: everything outside `<tool_call>` blocks is regular text

### Regex Pattern

```
<tool_call>\s*(.*?)\s*</tool_call>
```

Flags: dotall (`.` matches newlines), non-greedy

### Edge Cases

| Case                                 | Behavior                                                                  |
| ------------------------------------ | ------------------------------------------------------------------------- |
| Malformed JSON inside tags           | Treat entire `<tool_call>` block as regular text; emit warning            |
| Missing `name` field                 | Skip this tool call; emit warning                                         |
| Missing `arguments` field            | Default to empty object `{}`                                              |
| Unclosed `<tool_call>` tag           | Treat as regular text (no match)                                          |
| Nested angle brackets in JSON values | Handled by non-greedy regex matching `</tool_call>` end tag               |
| Empty `<tool_call></tool_call>`      | Skip; no tool call emitted                                                |
| `<tool_call>` in code blocks         | Parsed as tool call (LLMs should avoid this, but protocol is unambiguous) |

### Validation

After JSON parsing, validate:

1. `name` is a non-empty string
2. `arguments` is a JSON object (not array, not primitive)
3. Tool name exists in registry or activation set (lookup, not schema validation at parse time)

---

## 7. Prompt Template

The system prompt includes a dedicated section teaching the LLM the protocol. This section is injected by the `core.tool_protocol` prompt section (priority 450, condition: Always).

```
## Tool Usage Protocol

When you need to use a tool, output a <tool_call> block with <name> and <arguments> tags:

<tool_call>
<name>tool_name</name>
<arguments>{"param1": "value1"}</arguments>
</tool_call>

You may include multiple <tool_call> blocks in a single response. Each will be executed in order.

After each tool call, you will receive the result in a <tool_result> block:

<tool_result name="tool_name" success="true">
{"result_key": "result_value"}
</tool_result>

Important:
- Always use tool_search to discover available tools before calling them
- Do not guess tool names or parameters — search first, then call
- You may include regular text before and after tool calls
```

---

## 8. Mode Configuration

### PromptBased (default)

- Tool protocol taught via system prompt
- `ChatRequest.tools` is empty — no tools field in HTTP request body
- Tool calls parsed from LLM text output via `ToolCallParser`
- Tool results formatted as `<tool_result>` blocks
- Works with any LLM

### Native

- Tool definitions sent via provider-specific API fields (OpenAI `tools`, Anthropic `tools`)
- Tool calls extracted from provider-specific response fields (`tool_calls`)
- Tool results sent as provider-specific tool result messages
- Only works with providers that support native tool calling

### Configuration

```toml
[tool_calling]
mode = "prompt_based"  # "prompt_based" (default) | "native"
```

Per-provider override is possible:

```toml
[[providers]]
id = "openai-gpt4"
tool_calling_mode = "native"  # Override for this provider only
```

---

## 9. Interaction with Existing Systems

| System                             | Interaction                                                                                                 |
| ---------------------------------- | ----------------------------------------------------------------------------------------------------------- |
| **ToolIndex**                      | Still used internally for registry lookups; not directly exposed to LLM in PromptBased mode                 |
| **ToolActivationSet**              | Tools loaded via `ToolSearch` are activated here; full schemas cached for the session                      |
| **InjectTools** (context pipeline) | In PromptBased mode: injects taxonomy root + activated tool schemas. In Native mode: injects flat tool list |
| **ToolExecutor**                   | Unchanged — executes tools regardless of how they were called                                               |
| **ParameterValidator**             | Unchanged — validates parameters before execution                                                           |
| **Guardrails**                     | Unchanged — permission checks still apply                                                                   |

---

## 10. Token Budget Analysis

| Component                          | PromptBased     | Native                   |
| ---------------------------------- | --------------- | ------------------------ |
| Tool protocol section              | ~200 tokens     | 0                        |
| Tier 1 core tool schemas           | ~300 tokens     | 0                        |
| Taxonomy root (Tier 2)             | ~100 tokens     | 0                        |
| Flat tool index (root agent)       | 0               | ~50 tokens               |
| Sub-agent tools summary            | ~200 tokens     | 0 (via API field)        |
| Tool definitions in API            | 0               | 5,000-25,000 tokens      |
| Activated tool schemas (per tool)  | ~100-300 tokens | 0 (in API field)         |
| **Total (initial turn, 50 tools)** | **~600 tokens** | **~5,000-25,000 tokens** |

Savings: **60-95%** on initial turns. The Tier 1 core tools add ~300 tokens vs. the original lazy-only approach, but eliminate the most common failure mode (LLMs guessing non-existent tool names). After tool activation, costs converge but prompt-based remains more efficient because only used tools are loaded.

In Native mode, sub-agent system prompts are returned unchanged -- tool definitions are sent exclusively via the API `tools` field, avoiding redundant prompt injection.
