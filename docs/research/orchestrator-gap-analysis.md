# Orchestrator Gap Analysis and Enhancement Proposal

> Comparing y-agent with FlowLLM and LangGraph patterns

**Version**: v0.1
**Created**: 2026-03-06
**Status**: Analysis Complete

---

## TL;DR

Analysis of FlowLLM and LangGraph reveals several architectural patterns that could strengthen y-agent's orchestration capabilities. Key gaps include: unified channel/state aggregation model, expression-based flow DSL for rapid prototyping, and deeper checkpoint integration with task execution. This document proposes targeted enhancements while preserving y-agent's architectural identity.

---

## Framework Comparison Matrix

| Capability | y-agent (Current) | FlowLLM | LangGraph |
|------------|-------------------|---------|-----------|
| **Execution Model** | DAG with explicit dependencies | Expression tree (Op composition) | Pregel-style supersteps |
| **State Management** | Context + Task outputs | FlowContext (shared dict) | Channels with reducers |
| **Flow Definition** | TOML configuration | Python DSL + YAML expressions | StateGraph builder / @entrypoint |
| **Checkpoint** | Workflow-level snapshots | Flow-level cache | Per-superstep with pending writes |
| **Parallelism** | Task-level (All/Any/AtLeast) | Operator-level (ParallelOp) | Node-level with max_concurrency |
| **Human Interrupts** | HumanApproval task type | Not built-in | Interrupt + Agent Inbox protocol |
| **Dynamic Routing** | Condition branches | N/A (static expressions) | Conditional edges + Send |
| **Streaming** | Event bus | stream_queue per context | StreamMode (values/updates/messages) |
| **Recovery** | Rollback + compensation | N/A | Checkpoint resume with pending writes |

---

## Key Insights from FlowLLM

### 1. Expression-Based Flow DSL

FlowLLM's `flow_content` expression syntax enables rapid flow composition:

```python
# FlowLLM style
flow_content: GenSystemPromptOp() >> ChatOp()
flow_content: (SearchOp() | AnalyzeOp()) >> SummarizeOp()
```

**Gap in y-agent**: Current TOML-based workflow definition is verbose for simple flows.

**Recommendation**: Add optional expression DSL for simple flows while keeping TOML for complex workflows.

```rust
// Proposed y-agent enhancement
enum WorkflowDefinition {
    // Existing detailed TOML
    Detailed(DetailedWorkflow),
    // New expression-based shorthand
    Expression(String),  // "search >> analyze >> summarize"
}
```

### 2. Per-Request Flow Rebuild

FlowLLM rebuilds the Op tree on each request, ensuring no state leakage.

**Gap in y-agent**: Current design reuses Workflow instances.

**Recommendation**: Clarify workflow instance lifecycle; consider copy-on-execute semantics for stateless workflows.

### 3. Unified Registry Pattern

FlowLLM's `ServiceContext` provides global registry for LLM, Op, Flow, Service.

**Current y-agent approach**: Separate registries (ProviderPool, ToolRegistry, etc.) - this is already well-designed.

**Assessment**: No change needed; y-agent's separation is cleaner for type safety.

---

## Key Insights from LangGraph

### 1. Channel-Based State Aggregation

LangGraph's Channel model with reducers handles concurrent state updates elegantly:

```python
# LangGraph: Multiple nodes writing to same channel
messages: Annotated[list, add_messages]  # Reducer appends
```

**Gap in y-agent**: Current Context uses simple HashMap with last-write-wins.

**Recommendation**: Introduce typed state channels with configurable reducers.

```rust
// Proposed enhancement
enum ChannelType {
    LastValue,                    // Current behavior
    Append,                       // List accumulation
    Merge,                        // Dict merge
    Custom(Box<dyn Reducer>),     // User-defined
}

struct WorkflowContext {
    channels: HashMap<String, TypedChannel>,
    // ...
}
```

### 2. Superstep Execution Model

LangGraph's Pregel-inspired superstep model provides:
- Clear synchronization points
- Deterministic parallel execution
- Natural checkpoint boundaries

**Gap in y-agent**: Current DAG execution lacks explicit synchronization barriers.

**Recommendation**: Consider optional superstep mode for workflows requiring strong consistency.

```rust
// Proposed enhancement
enum ExecutionModel {
    // Current: Tasks execute as dependencies satisfy
    Eager,
    // New: Batch execution in synchronized rounds
    Superstep {
        checkpoint_per_step: bool,
    },
}
```

### 3. Pending Writes for Efficient Recovery

LangGraph tracks `pending_writes` separately from committed state, enabling:
- Failed task recovery without re-executing successful tasks
- Partial rollback granularity

**Gap in y-agent**: Current recovery model is workflow-level (all or nothing).

**Recommendation**: Implement task-level pending writes.

```rust
// Proposed enhancement
struct CheckpointData {
    committed_outputs: HashMap<TaskId, TaskOutput>,
    pending_writes: Vec<(TaskId, TaskOutput)>,
    versions_seen: HashMap<TaskId, Version>,
}
```

### 4. StreamMode Flexibility

LangGraph offers multiple streaming granularities:
- `values`: Full state per step
- `updates`: Delta changes only
- `messages`: Token-level for LLM output

**Gap in y-agent**: Current EventBus is event-type based, not output-mode based.

**Recommendation**: Add StreamMode configuration to workflow execution.

```rust
// Proposed enhancement
enum StreamMode {
    None,           // Final result only
    Values,         // Full context per task completion
    Updates,        // Delta changes
    Messages,       // Token-level (for LLM tasks)
    Debug,          // All internal events
}

struct ExecutionConfig {
    stream_mode: StreamMode,
    // ...
}
```

### 5. Interrupt Protocol with Resume

LangGraph's `Interrupt` + `Command(resume=...)` enables clean human-in-the-loop:

```python
# LangGraph: Interrupt and resume
interrupt(HumanInterrupt(...))
# Later: Command(resume=HumanResponse(...))
```

**Gap in y-agent**: Current HumanApproval is task-type, not a first-class interrupt mechanism.

**Recommendation**: Elevate interrupts to workflow-level primitive.

```rust
// Proposed enhancement
enum WorkflowControl {
    Continue,
    Interrupt {
        reason: InterruptReason,
        resume_data: Option<Value>,
    },
    Cancel,
}

// Workflow can be resumed with
orchestrator.resume(workflow_id, resume_command).await?;
```

---

## Proposed Enhancements with Rationale

### Priority 1: State Management Improvements

#### 1.1 Typed Channels with Reducers

| Attribute | Value |
|-----------|-------|
| Effort | Medium |
| Impact | High |
| Reference | LangGraph channels |

**Problem Statement**:

When multiple parallel tasks write to the same context variable, the current HashMap approach uses last-write-wins semantics. This causes data loss in common scenarios:

```
# Current behavior (problematic)
Task A writes: context["results"] = ["result_a"]
Task B writes: context["results"] = ["result_b"]  # Overwrites A!
Final state: ["result_b"]  # Lost result_a
```

**Why This Enhancement**:

1. **Data Integrity**: Parallel tasks often produce complementary results (e.g., multiple search sources). Losing results silently leads to incorrect agent behavior.

2. **Explicit Semantics**: Developers must currently implement manual merge logic in downstream tasks. This is error-prone and repetitive.

3. **Determinism**: With reducers, the merge behavior is declared upfront and consistent, making workflows easier to reason about and debug.

**Benefit**:

```
# With typed channels
Task A writes: channel["results"].append(["result_a"])
Task B writes: channel["results"].append(["result_b"])
Final state (Append reducer): ["result_a", "result_b"]  # Both preserved
```

---

#### 1.2 Task-Level Pending Writes (Checkpoint Granularity)

| Attribute | Value |
|-----------|-------|
| Effort | Medium |
| Impact | High |
| Reference | LangGraph checkpoint |

**Problem Statement**:

Current workflow recovery is all-or-nothing. When a workflow with 10 tasks fails at task 8, recovery re-executes all 10 tasks, wasting:
- LLM API calls (cost)
- Time (latency)
- External side effects may be duplicated

**Why This Enhancement**:

1. **Cost Efficiency**: LLM calls are expensive. Re-executing successful tasks wastes money. For a workflow with 5 LLM calls at $0.01 each, recovering from task 4 wastes $0.03 per recovery.

2. **Idempotency Challenges**: Not all tasks are idempotent. Re-executing a "send email" task causes duplicate emails. Task-level recovery skips completed side-effect tasks.

3. **Faster Recovery**: Only re-execute failed and pending tasks. A 10-task workflow failing at task 8 recovers in ~20% of the original time.

**Benefit**:

| Scenario | Current Recovery | With Pending Writes |
|----------|------------------|---------------------|
| 10 tasks, fail at task 8 | Re-run all 10 | Re-run tasks 8-10 only |
| Recovery time | 100% | ~30% |
| Wasted LLM calls | 7 | 0 |

---

#### 1.3 StreamMode Configuration

| Attribute | Value |
|-----------|-------|
| Effort | Low |
| Impact | Medium |
| Reference | LangGraph StreamMode |

**Problem Statement**:

Current EventBus emits all events to all subscribers. Clients cannot choose their desired granularity, leading to:
- UI clients receiving internal debug events they do not need
- Debug tools missing fine-grained events they need
- No standard way to stream LLM token output

**Why This Enhancement**:

1. **Client Flexibility**: CLI wants token-level streaming for typewriter effect. API clients want final results only. Debug UI wants everything. One size does not fit all.

2. **Performance**: Streaming full state on every task completion is wasteful for simple use cases. Delta-only mode reduces bandwidth.

3. **Standardization**: Current ad-hoc event filtering leads to inconsistent client implementations. StreamMode provides a standard vocabulary.

**Benefit**:

| Use Case | Current Approach | With StreamMode |
|----------|------------------|-----------------|
| CLI typewriter effect | Custom event filtering | `StreamMode::Messages` |
| API final result | Wait for completion event | `StreamMode::None` |
| Debug dashboard | Subscribe to all events | `StreamMode::Debug` |
| Real-time progress | Custom delta calculation | `StreamMode::Updates` |

---

### Priority 2: Execution Model Extensions

#### 2.1 Optional Superstep Execution Mode

| Attribute | Value |
|-----------|-------|
| Effort | High |
| Impact | Medium |
| Reference | LangGraph Pregel |

**Problem Statement**:

Current eager DAG execution starts tasks as soon as dependencies are satisfied. This works well for most cases but causes issues when:
- Multiple tasks read/write shared state without explicit ordering
- Checkpoint timing is unpredictable
- Debugging parallel execution is difficult due to non-deterministic ordering

**Why This Enhancement**:

1. **Deterministic Execution**: Supersteps provide clear "rounds" where all ready tasks execute, then all results are committed. Same input always produces same execution order.

2. **Natural Checkpoint Boundaries**: Each superstep boundary is a consistent state. Checkpointing at superstep boundaries ensures recoverability without partial state.

3. **Debugging**: When execution is grouped into numbered rounds, tracing and debugging become straightforward. "The bug occurred in superstep 3" is actionable.

**When to Use**:

| Scenario | Recommended Mode |
|----------|------------------|
| Simple linear workflows | Eager (current) |
| Complex parallel with shared state | Superstep |
| Workflows requiring audit trail | Superstep |
| Performance-critical, independent tasks | Eager (current) |

---

#### 2.2 Expression DSL Shorthand

| Attribute | Value |
|-----------|-------|
| Effort | Medium |
| Impact | Medium |
| Reference | FlowLLM flow_content |

**Problem Statement**:

Simple workflows require verbose TOML configuration. A 3-task sequential workflow needs ~50 lines of TOML, creating friction for rapid prototyping and simple use cases.

**Why This Enhancement**:

1. **Rapid Prototyping**: Developers often start with simple workflows and add complexity later. Expression DSL enables quick iteration: `search >> analyze >> summarize`.

2. **Readability**: For simple flows, the expression `(search | scrape) >> summarize` communicates intent more clearly than 80 lines of TOML.

3. **Complementary, Not Replacement**: Expression DSL is syntactic sugar that compiles to the same internal representation. Complex workflows still use TOML. No architecture change required.

**Benefit**:

| Flow Complexity | Current (TOML) | With Expression DSL |
|-----------------|----------------|---------------------|
| 3-task sequential | ~50 lines | 1 line |
| Parallel + sequential | ~80 lines | 1 line |
| Complex with conditions | ~150 lines | Still use TOML |

---

#### 2.3 Workflow-Level Interrupt/Resume

| Attribute | Value |
|-----------|-------|
| Effort | Medium |
| Impact | High |
| Reference | LangGraph Interrupt |

**Problem Statement**:

Current `HumanApproval` is a task type, requiring:
- Pre-planned approval points in workflow definition
- No dynamic interrupts based on runtime conditions
- No standard protocol for resume handling

**Why This Enhancement**:

1. **Dynamic Interrupts**: Agent discovers risky operation at runtime and needs approval. Current design requires pre-defining all possible approval points. Workflow-level interrupt allows any task to pause execution.

2. **Unified Resume Protocol**: Current HumanApproval has ad-hoc resume handling. A standard `ResumeCommand` enum enables consistent UI/API integration across all interrupt types.

3. **Composability**: Interrupts become a cross-cutting concern. Any workflow can be made human-supervised without changing task definitions.

**Use Cases Enabled**:

| Scenario | Current Support | With Interrupt/Resume |
|----------|-----------------|----------------------|
| Pre-planned approval gates | Supported | Supported |
| Dynamic "dangerous operation detected" | Not supported | Supported |
| "Agent needs clarification" | Workaround required | Native support |
| Resume from different session/device | Not supported | Supported (via checkpoint) |

---

### Priority 3: Expression Templates (Low Priority)

| Attribute | Value |
|-----------|-------|
| Effort | Low |
| Impact | Medium |
| Reference | FlowLLM templates |

**Why This Enhancement**:

Parameterized expression templates enable workflow reuse without full TOML complexity. Example: `web_research("{{query}}")` expands to a standard search-scrape-summarize flow with variable injection.

---

## Detailed Implementation Specifications

### Specification 1: Typed Channels

**Current State**:
```rust
struct WorkflowContext {
    variables: HashMap<String, Value>,
    task_outputs: HashMap<TaskId, TaskOutput>,
}
```

**Proposed State**:
```rust
struct WorkflowContext {
    channels: HashMap<String, Channel>,
    task_outputs: HashMap<TaskId, TaskOutput>,
}

struct Channel {
    value: Value,
    channel_type: ChannelType,
    version: u64,
}

enum ChannelType {
    LastValue,
    Append,
    Merge { conflict: MergeConflict },
    BinaryOp { reducer: Box<dyn Fn(Value, Value) -> Value> },
}
```

**Migration Path**: Existing `variables` become `LastValue` channels by default.

### Specification 2: Task-Level Checkpointing

**Current Checkpoint**:
```rust
struct WorkflowSnapshot {
    context: WorkflowContext,
    task_states: HashMap<TaskId, TaskState>,
}
```

**Proposed Checkpoint**:
```rust
struct WorkflowCheckpoint {
    // Committed state
    committed_channels: HashMap<String, ChannelSnapshot>,
    committed_tasks: HashMap<TaskId, TaskOutput>,

    // Pending (uncommitted) writes
    pending_channel_writes: Vec<(String, Value)>,
    pending_task_outputs: Vec<(TaskId, TaskOutput)>,

    // Version tracking for skip-on-recovery
    versions_seen: HashMap<TaskId, u64>,

    // Metadata
    step_number: u64,
    checkpoint_time: Timestamp,
}
```

**Benefit**: On recovery, only re-execute tasks whose outputs are in `pending`, not `committed`.

### Specification 3: Workflow Interrupts

**Current Approach**:
```rust
enum TaskType {
    HumanApproval { ... },  // Task-level only
}
```

**Proposed Approach**:
```rust
// Workflow-level interrupt
enum WorkflowInterrupt {
    HumanApproval {
        prompt: String,
        options: Vec<ApprovalOption>,
        timeout: Duration,
    },
    Confirmation {
        action: String,
        details: Value,
    },
    InputRequired {
        schema: JsonSchema,
        prompt: String,
    },
}

// Resume command
enum ResumeCommand {
    Approve { selected: Value },
    Reject { reason: String },
    Provide { data: Value },
    Cancel,
}

// Orchestrator API
impl Orchestrator {
    async fn execute(&self, workflow: Workflow) -> ExecutionHandle;

    async fn resume(
        &self,
        execution_id: ExecutionId,
        command: ResumeCommand,
    ) -> Result<ExecutionHandle>;
}
```

---

## Implementation Roadmap

> Note: Scheduled Tasks module is documented separately in [scheduled-tasks-design.md](scheduled-tasks-design.md). The enhancements below focus on Orchestrator core improvements.

### Phase 1: State Management Foundation (Weeks 1-2)

| Task | Deliverable |
|------|-------------|
| Typed channels | `ChannelType` enum with LastValue, Append, Merge |
| Pending writes | `WorkflowCheckpoint` with committed/pending separation |
| Context API update | Backward-compatible channel access methods |

### Phase 2: Execution Model (Weeks 3-4)

| Task | Deliverable |
|------|-------------|
| StreamMode | 5 streaming modes with client configuration |
| Interrupt/Resume | `WorkflowInterrupt` and `ResumeCommand` types |
| Superstep mode | Optional `ExecutionModel::Superstep` |

### Phase 3: DSL and Migration (Weeks 5-6)

| Task | Deliverable |
|------|-------------|
| Expression parser | `>>` and `|` operators for flow composition |
| TOML-to-expression | Tooling to simplify existing workflows |
| Documentation | Updated orchestrator guide with examples |

---

## Alternatives Considered

### Full Pregel Adoption

**Considered**: Replacing DAG model entirely with Pregel supersteps.

**Rejected**: Would require significant rewrite; current DAG model is more intuitive for most use cases. Offer superstep as optional mode instead.

### FlowLLM-Style Op Composition

**Considered**: Adopting operator-based composition as primary model.

**Rejected**: Less explicit than current task-based model; harder to debug complex workflows. Offer expression shorthand for simple cases only.

### Separate Checkpoint Service

**Considered**: External checkpoint service (like LangGraph's checkpoint-postgres).

**Deferred**: Current SQLite-based approach sufficient for single-node. Revisit for distributed deployment.

---

## Success Metrics

| Metric | Current | Target |
|--------|---------|--------|
| Recovery granularity | Workflow-level | Task-level |
| State conflict handling | Last-write-wins | Configurable reducers |
| Streaming options | Event types | 5 stream modes |
| Interrupt resume | Not supported | Full support |
| Simple flow definition | ~50 lines TOML | ~1 line expression |

---

## Related Documents

- [Orchestrator Design](orchestrator-design.md) - Current orchestration design (to be updated with these enhancements)
- [Scheduled Tasks Design](scheduled-tasks-design.md) - Separate module for time-based and event-driven scheduling
- [Context Session Design](context-session-design.md) - Session state management (channel changes affect this)

---

## Changelog

| Version | Date | Changes |
|---------|------|---------|
| v0.1 | 2026-03-06 | Initial gap analysis |
