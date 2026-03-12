# Tool Search Design

> Hierarchical tool discovery via taxonomy-based search

**Version**: v0.1
**Created**: 2026-03-12
**Status**: Draft

---

## TL;DR

Tool search replaces flat keyword matching with a **hierarchical taxonomy tree**. Tools are organized into multi-level categories (e.g., `file → read → file_read`), allowing LLMs to navigate from broad domains to specific tools incrementally. A `tool-searcher` system sub-agent handles search queries using the existing AgentDelegator pattern. The taxonomy is declared in TOML configuration and loaded at startup. Only the taxonomy root (~100 tokens) is injected into the LLM prompt; full tool schemas are loaded on demand via `tool_search` calls.

---

## Background and Goals

### Background

The current `tool_search` meta-tool performs flat keyword matching against tool names and descriptions. With a growing tool catalog (built-in, MCP, custom, dynamic), flat search becomes unreliable: an LLM searching for "read a file" may not match `file_read`, and searching for "network" returns no useful results because no tool has "network" in its name.

A hierarchical taxonomy solves this by providing structured navigation — the LLM first identifies the right category, then drills down to the specific tool.

### Goals

| Goal | Measurable Criteria |
|------|-------------------|
| **Structured discovery** | LLM can find any registered tool in ≤ 2 search calls (category → tool) |
| **Token efficiency** | Taxonomy root ≤ 100 tokens; full category detail ≤ 200 tokens |
| **Extensibility** | New categories/tools added via TOML without code changes |
| **Backward compatibility** | Keyword search still works alongside taxonomy navigation |

---

## Taxonomy Structure

### Three Levels

```
Level 0: Taxonomy Root
  ├── Level 1: Category (file, shell, network, memory, agent, meta)
  │     ├── Level 2: Subcategory (file.read, file.write, file.search)
  │     │     └── Tools: [file_read, file_list]
  │     └── Direct tools: [shell_exec]
  └── ...
```

### TOML Schema

```toml
# config/tool_taxonomy.toml

[categories.file]
label = "File Management"
description = "Read, write, list, search, and manage files and directories"

[categories.file.subcategories.read]
label = "File Reading"
description = "Read file contents and metadata"
tools = ["file_read"]

[categories.file.subcategories.write]
label = "File Writing"
description = "Create, write, and modify files"
tools = ["file_write"]

[categories.file.subcategories.list]
label = "Directory Listing"
description = "List directory contents and file trees"
tools = ["file_list"]

[categories.file.subcategories.search]
label = "File Search"
description = "Search file contents by pattern or keyword"
tools = ["file_search"]

[categories.shell]
label = "Shell Execution"
description = "Execute shell commands and scripts"
tools = ["shell_exec"]

[categories.network]
label = "Network"
description = "HTTP requests, API calls, DNS, and connectivity"

[categories.memory]
label = "Memory & Knowledge"
description = "Store, recall, and search knowledge and facts"

[categories.search]
label = "Search & Retrieval"
description = "Web search, document search, and information retrieval"

[categories.agent]
label = "Agent & Workflow"
description = "Delegate tasks to sub-agents, manage workflows and schedules"

[categories.meta]
label = "Meta Tools"
description = "Tool management — search, create, and modify tools"
tools = ["tool_search"]
```

### Rust Types

```rust
/// Root taxonomy containing all categories.
pub struct ToolTaxonomy {
    categories: IndexMap<String, TaxonomyCategory>,
}

/// A top-level category.
pub struct TaxonomyCategory {
    pub label: String,
    pub description: String,
    pub subcategories: IndexMap<String, TaxonomySubcategory>,
    /// Tools directly under this category (no subcategory).
    pub tools: Vec<ToolName>,
}

/// A second-level subcategory.
pub struct TaxonomySubcategory {
    pub label: String,
    pub description: String,
    pub tools: Vec<ToolName>,
}
```

---

## Search Operations

### 1. Browse Category

**Input:** `tool_search(category: "file")`

**Output:**
```json
{
  "category": "file",
  "label": "File Management",
  "description": "Read, write, list, search, and manage files and directories",
  "subcategories": [
    {"id": "read", "label": "File Reading", "tools": ["file_read"]},
    {"id": "write", "label": "File Writing", "tools": ["file_write"]},
    {"id": "list", "label": "Directory Listing", "tools": ["file_list"]},
    {"id": "search", "label": "File Search", "tools": ["file_search"]}
  ],
  "tools": []
}
```

### 2. Browse Subcategory

**Input:** `tool_search(category: "file.read")`

**Output:**
```json
{
  "category": "file.read",
  "label": "File Reading",
  "tools": [
    {"name": "file_read", "description": "Read file contents by path"}
  ]
}
```

### 3. Get Tool Schema

**Input:** `tool_search(tool: "file_read")`

**Output:**
```json
{
  "name": "file_read",
  "description": "Read file contents by path",
  "parameters": {
    "type": "object",
    "properties": {
      "path": {"type": "string", "description": "Absolute or relative file path"}
    },
    "required": ["path"]
  },
  "category": "file.read"
}
```

### 4. Keyword Search (fallback)

**Input:** `tool_search(query: "read contents of a file")`

Searches across all tool names, descriptions, category labels, and subcategory labels. Returns ranked matches.

**Output:**
```json
{
  "query": "read contents of a file",
  "results": [
    {"name": "file_read", "description": "Read file contents by path", "category": "file.read", "relevance": "high"},
    {"name": "file_search", "description": "Search file contents by pattern", "category": "file.search", "relevance": "medium"}
  ]
}
```

---

## tool_search Meta-Tool Schema

```json
{
  "name": "tool_search",
  "description": "Search for tools by category, keyword, or specific name. Use category browsing for structured discovery, or keyword search for fuzzy matching.",
  "parameters": {
    "type": "object",
    "properties": {
      "category": {
        "type": "string",
        "description": "Browse a category or subcategory. Use dot notation for subcategories (e.g., 'file', 'file.read')"
      },
      "tool": {
        "type": "string",
        "description": "Get the full schema of a specific tool by name"
      },
      "query": {
        "type": "string",
        "description": "Keyword search across all tools and categories"
      }
    }
  }
}
```

Exactly one of `category`, `tool`, or `query` should be provided. If multiple are given, precedence: `tool` > `category` > `query`.

---

## Taxonomy Auto-Registration

When a tool is registered in `ToolRegistry`, it can be automatically placed in the taxonomy:

1. **Built-in tools**: Category assignment via `ToolDefinition.category` field (already exists)
2. **MCP tools**: Category inferred from MCP server metadata, or placed in `uncategorized`
3. **Dynamic tools**: Category specified at creation time, or `uncategorized`
4. **TOML override**: `tool_taxonomy.toml` takes precedence over auto-assignment

Mapping from `ToolCategory` enum to taxonomy path:

| ToolCategory | Taxonomy Path |
|-------------|---------------|
| FileSystem | file |
| Shell | shell |
| Network | network |
| Memory | memory |
| Knowledge | memory |
| Search | search |
| Agent | agent |
| Workflow | agent |
| Schedule | agent |
| Custom | uncategorized |

---

## Integration with ToolActivationSet

After `tool_search(tool: "file_read")` returns the full schema:

1. The full `ToolDefinition` is added to `ToolActivationSet` (LRU, ceiling 20)
2. The tool becomes available for direct invocation
3. If the activation set is full, the least-recently-used tool is evicted
4. `tool_search` itself is marked `always_active` and never evicted

---

## tool-searcher Sub-Agent

For complex search queries, a `tool-searcher` system sub-agent can be used:

```toml
# Built-in agent definition
[agent.tool-searcher]
role = "tool-discovery"
mode = "explore"
model_tags = ["fast"]
tools.allowed = ["tool_search"]
context_strategy = "none"
limits.max_iterations = 3
limits.max_tool_calls = 5
system_instructions = """
You are a tool search assistant. Given a user's task description,
find the most relevant tools from the taxonomy.
Return a structured list of tool names with their schemas.
"""
```

The main agent delegates to `tool-searcher` when:
- The keyword search returns too many results
- The task description is ambiguous
- Multiple related tools need to be discovered at once

---

## Performance Targets

| Operation | Target |
|-----------|--------|
| Taxonomy TOML parse | < 5ms |
| Category browse | < 1ms |
| Keyword search (100 tools) | < 5ms |
| Full tool schema retrieval | < 1ms |
| Taxonomy root prompt injection | ≤ 100 tokens |
| Category detail | ≤ 200 tokens |
