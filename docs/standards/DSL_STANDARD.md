# y-agent Workflow DSL Standard

**Version**: v0.1
**Created**: 2026-03-10
**Status**: Draft

---

## 1. Purpose

This document defines the standard for y-agent's dual-mode workflow definition language: the **Expression DSL** (shorthand syntax for simple flows) and the **TOML Workflow Configuration** (detailed format for complex workflows). Both modes compile to the same internal `TaskDag` representation and are used by the Orchestrator to schedule and execute multi-task workflows.

**Authoritative design reference**: [orchestrator-design.md](../design/orchestrator-design.md)

### 1.1 Design Principles

| Principle | Description |
|-----------|-------------|
| **Dual mode, single representation** | Expression DSL and TOML compile to the same `Workflow` / `TaskDag` — no semantic divergence |
| **Expression DSL is syntactic sugar** | Purely additive; removal does not affect TOML workflows |
| **Composability** | Complex workflows compose from primitive operations (sequential, parallel, conditional, loop) |
| **Parse once, validate early** | All workflow definitions are parsed and validated at registration/load time, not at execution time |
| **Token efficiency** | Expression DSL minimizes definition overhead for LLM-generated workflows |

---

## 2. Expression DSL

### 2.1 Overview

The Expression DSL provides a concise, single-line syntax for defining workflow task graphs. It is designed for rapid prototyping, simple workflows, and LLM-generated workflow definitions.

```
search >> (analyze | score) >> summarize
```

This expression defines: run `search`, then run `analyze` and `score` in parallel, then run `summarize` after both complete.

### 2.2 Lexical Grammar

#### Token Types

| Token | Pattern | Description |
|-------|---------|-------------|
| `TASK_REF` | `[a-zA-Z0-9_-]+` | Task name reference |
| `SEQUENTIAL` | `>>` | Sequential composition operator |
| `PARALLEL` | `\|` | Parallel composition operator |
| `LEFT_PAREN` | `(` | Grouping open |
| `RIGHT_PAREN` | `)` | Grouping close |
| Whitespace | `[ \t\n\r]+` | Ignored between tokens |

#### Task Name Rules

- Must start with an alphanumeric character, underscore, or hyphen
- May contain: `a-z`, `A-Z`, `0-9`, `_`, `-`
- Case-sensitive: `Search` and `search` are different tasks
- No dots, slashes, or special characters beyond underscore and hyphen
- Examples: `search`, `web-search`, `data_analysis`, `step1`, `analyze-v2`

### 2.3 Operator Semantics

#### Sequential Operator (`>>`)

Establishes a dependency edge: the right operand depends on the left operand.

```
a >> b >> c
```

**Semantics**: Execute `a`, then `b` (after `a` completes), then `c` (after `b` completes).

**DAG representation**:
```
a → b → c
```

#### Parallel Operator (`|`)

Declares concurrent tasks with no dependency between them.

```
a | b | c
```

**Semantics**: Execute `a`, `b`, and `c` concurrently. All three are immediately ready.

**DAG representation**:
```
a
b   (no edges between them)
c
```

#### Operator Precedence

| Precedence | Operator | Associativity |
|-----------|----------|---------------|
| Higher | `\|` (parallel) | Left-to-right |
| Lower | `>>` (sequential) | Left-to-right |

Parentheses override default precedence.

**Consequence**: `a >> b | c >> d` parses as `a >> (b | c) >> d`, NOT as `(a >> b) | (c >> d)`.

### 2.4 Parentheses and Grouping

Parentheses control evaluation order and enable complex DAG topologies:

```
# Fan-out then fan-in
search >> (analyze | score | classify) >> summarize

# Nested grouping
(fetch | cache) >> (parse >> validate) >> store

# Deep nesting
a >> ((b | c) >> d) >> e
```

**Rules**:
- Parentheses must be balanced
- Empty parentheses `()` are not allowed
- Nesting depth is unlimited (parser is recursive descent)

### 2.5 Formal Grammar (EBNF)

```ebnf
expression  = sequential ;
sequential  = parallel , { ">>" , parallel } ;
parallel    = primary , { "|" , primary } ;
primary     = TASK_REF | "(" , expression , ")" ;

TASK_REF    = ( ALPHA | DIGIT | "_" | "-" )+ ;
ALPHA       = "a".."z" | "A".."Z" ;
DIGIT       = "0".."9" ;
```

### 2.6 Template Variables

Expression DSL supports variable substitution via `{{variable}}` syntax:

```
search_{{query}} >> analyze >> summarize
```

**Expansion rules**:
- Variables are enclosed in double curly braces: `{{name}}`
- Variables are expanded **before** tokenization
- If a variable is not found in the provided map, the placeholder remains unexpanded
- Variable names follow the same character rules as task names

**Template example**:

```
# Template definition
web_research("{{query}}")  →  search_{{query}} >> scrape >> summarize

# With variables = { "query": "rust_async" }
# Expands to: search_rust_async >> scrape >> summarize
```

### 2.7 Compilation to TaskDag

The parser produces a `DslWorkflow` AST that compiles to the internal `TaskDag`:

| AST Node | TaskDag Result |
|----------|---------------|
| `Task(name)` | Single `TaskNode` with the given name as ID; dependencies from context |
| `Sequential([a, b, c])` | Chain: `a`'s tail nodes become `b`'s dependencies, `b`'s tails become `c`'s |
| `Parallel([a, b, c])` | All branches share the same upstream dependencies; all tails are returned |

**Key invariant**: The compiled DAG is always acyclic. The Expression DSL grammar structurally prevents cycles — there is no way to express backward edges.

### 2.8 Error Handling

| Error | Cause | Example |
|-------|-------|---------|
| `UnexpectedChar` | Invalid character in expression | `search # analyze` |
| `UnexpectedEnd` | Expression ends prematurely | `search >>` |
| `UnexpectedToken` | Token in wrong position | `>> search` |
| `MismatchedParens` | Unbalanced parentheses | `(a >> b` |
| `EmptyExpression` | Empty or whitespace-only input | `""` |

All errors include position information for diagnostics.

### 2.9 Examples

```
# Simple sequential pipeline
search >> analyze >> summarize

# Parallel fan-out
web_search | database_query | cache_lookup

# Fan-out then fan-in
search >> (analyze | score) >> summarize

# Mixed complexity
fetch >> (parse | validate) >> (enrich | tag) >> store

# Micro-Agent Pipeline pattern
inspect >> locate >> analyze >> execute

# Multi-source aggregation
(google_search | bing_search | arxiv_search) >> deduplicate >> rank >> summarize
```

---

## 3. TOML Workflow Configuration

### 3.1 Overview

TOML workflow configuration is the primary format for complex workflows that require:
- Conditional branching
- Loop constructs
- Detailed I/O mappings
- Custom retry and failure strategies
- Channel type declarations
- Task-specific executor configuration

### 3.2 Document Structure

A TOML workflow file consists of the following top-level sections:

```toml
[workflow]
name = "research-pipeline"
description = "Multi-source research and summarization"
version = "1.0.0"
execution_model = "eager"           # "eager" | "superstep"
max_concurrent_tasks = 10
failure_strategy = "fail_fast"      # "fail_fast" | "continue_on_error"

[workflow.channels]
# Typed channel declarations

[workflow.inputs]
# Workflow input parameter declarations

[workflow.outputs]
# Workflow output mapping

[[workflow.tasks]]
# Task definitions (array of tables)
```

### 3.3 Channel Declarations

Channels define typed state variables with configurable merge semantics:

```toml
[workflow.channels.results]
type = "append"                     # "last_value" | "append" | "merge"

[workflow.channels.metadata]
type = "merge"
conflict = "latest"                 # "latest" | "error" | "custom"

[workflow.channels.status]
type = "last_value"                 # Default if type is omitted
```

| Channel Type | Behavior | Use Case |
|-------------|----------|----------|
| `last_value` | Last write wins (default) | Single-writer variables |
| `append` | Accumulates values into a list | Multiple search sources writing results |
| `merge` | Deep-merges maps with conflict resolution | Aggregating structured data from parallel tasks |

### 3.4 Input and Output Declarations

```toml
[workflow.inputs]
query = { type = "string", required = true, description = "Search query" }
max_results = { type = "integer", required = false, default = 10 }

[workflow.outputs]
summary = { source = "channel", channel = "final_summary" }
raw_results = { source = "task", task = "search", field = "results" }
```

### 3.5 Task Definition

Each task is defined as an entry in the `[[workflow.tasks]]` array:

```toml
[[workflow.tasks]]
id = "search"
name = "Web Search"
type = "tool_execution"             # See Task Types table
priority = "normal"                 # "critical" | "high" | "normal" | "low" | "background"
timeout_ms = 30000

  [workflow.tasks.executor]
  tool = "web_search"
  # Executor-specific configuration varies by task type

  [[workflow.tasks.inputs]]
  name = "query"
  source = "workflow_input"         # "workflow_input" | "task_output" | "context" | "constant" | "expression"
  key = "query"

  [[workflow.tasks.outputs]]
  name = "results"
  target = "context"                # "workflow_output" | "context" | "next_task_input"
  channel = "search_results"

  [workflow.tasks.condition]
  type = "always"                   # "always" | "if_channel" | "if_task_status" | "expression"

  [workflow.tasks.retry]
  max_attempts = 3
  delay_ms = 1000
  backoff = "exponential"           # "fixed" | "linear" | "exponential"

  [workflow.tasks.failure]
  strategy = "retry"                # "fail_fast" | "continue" | "retry" | "rollback" | "ignore" | "compensation"
```

### 3.6 Task Types

| Type | Value | Description | Executor Config |
|------|-------|-------------|----------------|
| LLM Call | `llm_call` | Send prompt to LLM provider | `model`, `prompt_template`, `temperature`, `max_tokens` |
| Tool Execution | `tool_execution` | Invoke a registered tool | `tool`, `parameters` |
| Sub-Agent | `sub_agent` | Delegate to an agent instance | `agent_definition`, `input_template` |
| Sub-Workflow | `sub_workflow` | Execute a nested workflow | `workflow_id` or inline definition |
| Script | `script` | Run a sandboxed script | `runtime`, `command`, `args` |
| Human Approval | `human_approval` | Request human decision | `prompt`, `options`, `timeout_ms` |
| Branch | `branch` | Conditional routing | `conditions` with target task IDs |
| Parallel Group | `parallel` | Explicit parallel group | `tasks`, `join` (`all`, `any`, `at_least(n)`) |
| Loop | `loop` | Bounded iteration | `condition`, `max_iterations`, `body_tasks` |

### 3.7 Dependencies

Task dependencies are declared explicitly:

```toml
[[workflow.tasks]]
id = "analyze"
name = "Analyze Results"
type = "llm_call"
dependencies = ["search", "enrich"]   # Runs after both search and enrich complete
```

### 3.8 Data Mapping

#### Input Sources

| Source | Description | Example |
|--------|-------------|---------|
| `workflow_input` | Value from workflow invocation parameters | `{ source = "workflow_input", key = "query" }` |
| `task_output` | Output from a predecessor task | `{ source = "task_output", task = "search", field = "results" }` |
| `context` | Value from a typed channel | `{ source = "context", channel = "accumulated_results" }` |
| `constant` | A static value | `{ source = "constant", value = "default text" }` |
| `expression` | A computed expression | `{ source = "expression", expr = "..." }` |

#### Output Targets

| Target | Description | Example |
|--------|-------------|---------|
| `workflow_output` | Becomes part of the final workflow result | `{ target = "workflow_output", key = "summary" }` |
| `context` | Write to a typed channel (reducer applies) | `{ target = "context", channel = "results" }` |
| `next_task_input` | Direct input to a specific downstream task | `{ target = "next_task_input", task = "summarize", input = "data" }` |

#### Optional Transforms

Transforms can be applied during input/output mapping:

```toml
[[workflow.tasks.inputs]]
name = "title"
source = "task_output"
task = "search"
field = "results"
transform = { type = "jsonpath", path = "$[0].title" }
```

| Transform | Description |
|-----------|-------------|
| `jsonpath` | Extract value using JSONPath expression |
| `regex` | Extract or replace using regex pattern |
| `cast` | Type conversion (string-to-int, etc.) |
| `function` | Custom transform function reference |

### 3.9 Conditional Execution

```toml
[workflow.tasks.condition]
type = "if_channel"
channel = "should_search"
operator = "equals"                 # "equals" | "not_equals" | "exists" | "gt" | "lt"
value = true
```

| Condition Type | Description |
|---------------|-------------|
| `always` | Task always executes (default) |
| `if_channel` | Execute if a channel value meets a condition |
| `if_task_status` | Execute based on a predecessor task's status |
| `expression` | Boolean expression over context values |

### 3.10 Failure and Compensation

```toml
[[workflow.tasks]]
id = "send_email"
type = "tool_execution"

  [workflow.tasks.failure]
  strategy = "compensation"
  compensation_task = "retract_email"

  [workflow.tasks.retry]
  max_attempts = 2
  delay_ms = 500
  backoff = "exponential"
  retry_on = ["network_error", "timeout"]   # Error type filter
```

### 3.11 Full Example

```toml
[workflow]
name = "research-and-summarize"
description = "Multi-source research pipeline with parallel search and summarization"
version = "1.0.0"
execution_model = "eager"
max_concurrent_tasks = 5
failure_strategy = "continue_on_error"

[workflow.channels.search_results]
type = "append"

[workflow.channels.final_summary]
type = "last_value"

[workflow.inputs]
query = { type = "string", required = true }
depth = { type = "integer", required = false, default = 3 }

[workflow.outputs]
summary = { source = "channel", channel = "final_summary" }

# Task 1: Web search
[[workflow.tasks]]
id = "web_search"
name = "Web Search"
type = "tool_execution"
priority = "high"
timeout_ms = 15000

  [workflow.tasks.executor]
  tool = "web_search"

  [[workflow.tasks.inputs]]
  name = "query"
  source = "workflow_input"
  key = "query"

  [[workflow.tasks.outputs]]
  name = "results"
  target = "context"
  channel = "search_results"

  [workflow.tasks.retry]
  max_attempts = 3
  delay_ms = 1000
  backoff = "exponential"

# Task 2: Database search (parallel with web_search)
[[workflow.tasks]]
id = "db_search"
name = "Database Search"
type = "tool_execution"
priority = "normal"
timeout_ms = 5000

  [workflow.tasks.executor]
  tool = "knowledge_search"

  [[workflow.tasks.inputs]]
  name = "query"
  source = "workflow_input"
  key = "query"

  [[workflow.tasks.outputs]]
  name = "results"
  target = "context"
  channel = "search_results"

  [workflow.tasks.failure]
  strategy = "ignore"

# Task 3: Analyze results (after both searches)
[[workflow.tasks]]
id = "analyze"
name = "Analyze Results"
type = "llm_call"
dependencies = ["web_search", "db_search"]
priority = "normal"
timeout_ms = 30000

  [workflow.tasks.executor]
  model = "balanced"
  prompt_template = "Analyze the following search results:\n\n{{search_results}}"
  max_tokens = 2000

  [[workflow.tasks.inputs]]
  name = "search_results"
  source = "context"
  channel = "search_results"

  [[workflow.tasks.outputs]]
  name = "analysis"
  target = "context"
  channel = "final_summary"

# Task 4: Summarize (after analysis)
[[workflow.tasks]]
id = "summarize"
name = "Generate Summary"
type = "llm_call"
dependencies = ["analyze"]
priority = "high"

  [workflow.tasks.executor]
  model = "capable"
  prompt_template = "Summarize concisely:\n\n{{final_summary}}"
  max_tokens = 500

  [[workflow.tasks.inputs]]
  name = "analysis"
  source = "context"
  channel = "final_summary"

  [[workflow.tasks.outputs]]
  name = "summary"
  target = "workflow_output"
  key = "summary"
```

---

## 4. Expression DSL to TOML Equivalence

The Expression DSL is syntactic sugar. Every expression compiles to the same internal representation as a TOML workflow. The following table shows equivalences:

| Expression DSL | Equivalent TOML Structure |
|---------------|--------------------------|
| `a >> b >> c` | Three tasks: `b.dependencies = ["a"]`, `c.dependencies = ["b"]` |
| `a \| b \| c` | Three tasks with no `dependencies` between them |
| `a >> (b \| c) >> d` | `a` runs first; `b` and `c` depend on `a`; `d` depends on `b` and `c` |
| `(a \| b) >> (c \| d)` | `a`, `b` have no deps; `c` and `d` both depend on `a` and `b` |

**Limitations of Expression DSL** (use TOML instead):

| Feature | Expression DSL | TOML |
|---------|---------------|------|
| Conditional branches | Not supported | `condition` field on tasks |
| Loops | Not supported | `loop` task type |
| Custom retry | Not supported | `retry` configuration per task |
| I/O mappings | Not supported (auto-wired) | Explicit `inputs`/`outputs` |
| Channel types | Not supported (all `last_value`) | `channels` section |
| Failure strategies | Not supported | `failure` configuration per task |
| Task executor config | Not supported (default) | `executor` section per task |
| Human approval tasks | Not supported | `human_approval` task type |
| Priority levels | Not supported (all `normal`) | `priority` field per task |

---

## 5. Workflow Templates

### 5.1 Template Registration

Workflows can be registered as reusable templates with parameter schemas:

```toml
[template]
name = "web-research"
description = "Search, scrape, and summarize from multiple sources"
tags = ["research", "summarization"]

[template.parameter_schema]
type = "object"
required = ["query"]

  [template.parameter_schema.properties.query]
  type = "string"
  description = "The research query"

  [template.parameter_schema.properties.max_sources]
  type = "integer"
  default = 5

[template.definition]
# Either Expression or TOML workflow definition
mode = "expression"
value = "(web_search | arxiv_search) >> deduplicate >> summarize"
```

### 5.2 Expression Templates

Expression templates combine the Expression DSL with parameterization:

```
web_research("{{query}}")
```

This expands to a registered template's definition with variable injection. Template expansion occurs **before** DSL parsing.

### 5.3 Template Invocation

Templates are invoked via the Orchestrator API or via meta-tools:

| Method | Description |
|--------|-------------|
| `orchestrator.execute(template_id, params)` | Programmatic invocation |
| `workflow_create` meta-tool | Agent creates a new template |
| `workflow_list` meta-tool | Agent queries available templates |
| `workflow_get` meta-tool | Agent loads a template definition |

---

## 6. Implementation Reference

### 6.1 Crate Location

The Expression DSL parser is implemented in:

- **Crate**: `y-agent-core`
- **Module**: `expression_dsl` ([expression_dsl.rs](../../crates/y-agent-core/src/expression_dsl.rs))
- **Public API**: `tokenize()`, `parse()`, `expand_template()`, `DslWorkflow::to_task_dag()`

### 6.2 AST Types

```rust
/// Abstract syntax tree for parsed DSL expressions.
enum DslWorkflow {
    Task(String),                   // Single task reference
    Sequential(Vec<DslWorkflow>),   // Execute children in order
    Parallel(Vec<DslWorkflow>),     // Execute children concurrently
}
```

### 6.3 Error Types

```rust
enum DslError {
    UnexpectedChar { ch: char, pos: usize },
    UnexpectedEnd,
    UnexpectedToken(Token),
    MismatchedParens,
    EmptyExpression,
}
```

### 6.4 Compilation Pipeline

```
Input String
    ↓  expand_template() — substitute {{variables}}
Expanded String
    ↓  tokenize() — lexical analysis
Vec<Token>
    ↓  parse() — recursive descent parser
DslWorkflow (AST)
    ↓  to_task_dag() — compile to internal DAG
TaskDag
    ↓  Orchestrator.execute() — schedule and run
Execution
```

---

## 7. Performance Requirements

| Operation | Target |
|-----------|--------|
| Expression DSL parsing | < 1ms |
| Template variable expansion | < 0.1ms |
| TOML workflow deserialization | < 5ms |
| DAG validation (100-task DAG) | < 5ms |

---

## 8. Open Questions

| # | Question | Owner | Due Date | Status |
|---|----------|-------|----------|--------|
| 1 | Should Expression DSL support conditional routing (`if/else` syntax), or limit to sequential/parallel? | Orchestrator team | 2026-03-20 | Open |
| 2 | Should Expression DSL support inline retry shorthand (e.g., `search[retry=3] >> analyze`)? | Orchestrator team | 2026-04-01 | Open |
| 3 | Should TOML workflow support importing/inheriting from other workflow files? | Orchestrator team | 2026-04-15 | Open |
| 4 | What is the maximum Expression DSL length for LLM-generated workflows before TOML is recommended? | Orchestrator team | 2026-04-01 | Open |

---

## Changelog

| Version | Date | Changes |
|---------|------|---------|
| v0.1 | 2026-03-10 | Initial DSL standard document |
