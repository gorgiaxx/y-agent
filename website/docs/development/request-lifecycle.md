# Request Lifecycle

This document traces the end-to-end journey of a user message through the y-agent system, from input to response.

## High-Level Flow

```mermaid
sequenceDiagram
    participant U as User
    participant P as Presentation<br/>(CLI/Web/GUI)
    participant CS as ChatService
    participant AS as AgentService
    participant CP as ContextPipeline
    participant PP as ProviderPool
    participant LLM as LLM Provider
    participant TE as ToolExecutor
    participant T as Tool

    U->>P: Send message
    P->>CS: send_message(session_id, message)
    CS->>CS: prepare_turn()
    Note over CS: Load session, append message<br/>to dual transcripts,<br/>resolve system prompt,<br/>filter tool definitions

    CS->>AS: execute(config, progress, cancel)
    AS->>CP: assemble_with_request()
    Note over CP: Priority-ordered providers:<br/>100: SystemPrompt<br/>200: Bootstrap<br/>300: Memory<br/>350: Knowledge<br/>400: Skills<br/>500: Tools<br/>600: History<br/>700: Status
    CP-->>AS: AssembledContext

    loop Agent Turn Loop
        AS->>AS: prune_working_history()<br/>strip_historical_thinking()
        AS->>PP: chat_completion(request, route)
        PP->>PP: TagBasedRouter::select()
        Note over PP: Freeze filter -><br/>Tag filter -><br/>Priority filter -><br/>Strategy select
        PP->>LLM: chat_completion(request)
        LLM-->>PP: ChatResponse
        PP-->>AS: ChatResponse

        alt Has tool_calls
            loop For each ToolCall
                AS->>AS: check permissions
                AS->>TE: execute_tool_call()
                TE->>T: execute(ToolInput)
                T-->>TE: ToolOutput
                TE-->>AS: result string
            end
            Note over AS: Append tool results<br/>to working_history,<br/>continue loop
        else No tool_calls
            AS-->>CS: AgentExecutionResult
        end
    end

    CS->>CS: append assistant reply<br/>to dual transcripts
    CS-->>P: TurnEvent::Complete
    P-->>U: Display response
```

## Phase 1: Turn Preparation

**Entry:** `ChatService::send_message()` in `y-service/src/chat.rs`

1. **Load session** from `SessionManager` using `session_id`
2. **Append user message** to both transcripts:
   - Context transcript (LLM-facing, subject to compaction)
   - Display transcript (UI-facing, immutable)
3. **Resolve system prompt** from `PromptContext` (rendered template with mode overlays)
4. **Load conversation history** from the context transcript
5. **Filter tool definitions** via `AgentService::filter_tool_definitions()` -- respects agent `allowed_tools` allowlist
6. **Build `AgentExecutionConfig`** with session_id, messages, system_prompt, tool_definitions, max_iterations, trust_tier

## Phase 2: Context Assembly

**Entry:** `ContextPipeline::assemble_with_request()` in `y-context/src/pipeline.rs`

The pipeline iterates registered `ContextProvider` implementations in priority order. Each provider appends `ContextItem` entries to the `AssembledContext`.

```mermaid
graph LR
    subgraph Pipeline["Context Pipeline (priority order)"]
        A["100: System Prompt"] --> B["200: Bootstrap"]
        B --> C["300: Memory Recall"]
        C --> D["350: Knowledge Search"]
        D --> E["400: Skill Injection"]
        E --> F["500: Tool Definitions"]
        F --> G["600: History"]
        G --> H["700: Context Status"]
    end
```

Each `ContextItem` carries:
- `category` -- SystemPrompt, Bootstrap, Memory, Knowledge, Skills, Tools, History, Status
- `content` -- the actual text injected into the prompt
- `token_estimate` -- estimated token count for budget tracking
- `priority` -- ordering weight within the category

**Fail-open design:** If any provider errors, the pipeline logs a warning and continues. Partial context is better than no context.

## Phase 3: Agent Execution Loop

**Entry:** `AgentService::execute()` in `y-service/src/agent_service/mod.rs`

### Initialization

1. Set up `DiagnosticsContext` and trace scope (if tracing enabled)
2. Build `working_history` from assembled context + conversation messages
3. Initialize `ToolExecContext` with iteration counters, token accumulators, cancellation token

### Loop Body (each iteration)

```mermaid
flowchart TD
    A[Check CancellationToken] -->|cancelled| B[Return partial result]
    A -->|active| C{iteration > 0?}
    C -->|yes| D[Prune working history<br/>Strip historical thinking]
    C -->|no| E[Build LLM request]
    D --> E
    E --> F{iteration > max?}
    F -->|yes| G[ToolLoopLimitExceeded]
    F -->|no| H[Acquire provider from pool]
    H --> I[call_llm]
    I -->|success| J{Has tool_calls?}
    I -->|error| K[handle_llm_error]
    J -->|yes| L[Execute tool calls]
    L --> M[Append results to history]
    M --> A
    J -->|no| N[Build final result<br/>Return AgentExecutionResult]
```

### Intra-Turn Pruning

Between iterations, the system applies three pruning strategies:

1. **`IntraTurnPruner::prune_working_history()`** -- removes failed tool call branches (error results that the LLM has already seen and reacted to)
2. **`pruning::prune_old_tool_results()`** -- truncates or removes stale tool outputs from earlier iterations
3. **`pruning::strip_historical_thinking()`** -- removes `reasoning_content` from previous turns (only the current turn's thinking is preserved)

## Phase 4: LLM Communication

**Entry:** `llm::call_llm()` in `y-service/src/agent_service/llm.rs`

1. Build `ChatRequest` with model, temperature, max_tokens, tools, thinking config
2. Build `RouteRequest` with preferred provider/model, required tags, priority tier
3. Provider pool selects a provider via `TagBasedRouter` (see [Provider Pool](./provider-pool))
4. Call `provider.chat_completion()` or `chat_completion_stream()`
5. On success: accumulate `TokenUsage` into cumulative counters
6. On error: provider pool classifies error and may freeze the provider

## Phase 5: Tool Execution

**Entry:** `tool_handling::handle_native_tool_calls()` in `y-service/src/agent_service/tool_handling.rs`

```mermaid
sequenceDiagram
    participant AS as AgentService
    participant TD as ToolDispatch
    participant PM as PermissionModel
    participant TR as ToolRegistry
    participant MW as MiddlewareChain
    participant T as Tool

    AS->>TD: execute_and_record_tool(tool_call)
    TD->>PM: evaluate(tool_name, is_dangerous)
    alt Deny
        PM-->>TD: PermissionDecision::Deny
        TD-->>AS: error content
    else Ask
        PM-->>TD: PermissionDecision::Ask
        TD->>TD: block on oneshot channel
        Note over TD: User approves/denies<br/>via TurnEvent::PermissionRequest
    else Allow
        PM-->>TD: PermissionDecision::Allow
    end

    alt Meta-tool (ToolSearch, Task, Plan, Workflow)
        TD->>TD: dispatch to orchestrator
    else Regular tool
        TD->>TR: get_tool(name)
        TR->>MW: pre-execution chain
        MW-->>TR: continue or abort
        TR->>T: execute(ToolInput)
        T-->>TR: ToolOutput
        TR->>MW: post-execution chain
        TR-->>TD: result
    end

    TD->>TD: truncate_tool_result() (10K char cap)
    TD-->>AS: formatted result
```

### Meta-Tool Interception

Before reaching the registry, certain tool names are intercepted and dispatched to specialized orchestrators:

| Tool Name | Orchestrator | Purpose |
|-----------|-------------|---------|
| `ToolSearch` | `ToolSearchOrchestrator` | Activates tools into the LRU set |
| `Task` | `TaskDelegationOrchestrator` | Spawns sub-agent with isolated session |
| `Plan` | `PlanOrchestrator` | Structured planning via agent delegation |
| `WorkflowCreate/List/...` | `WorkflowOrchestrator` | DAG workflow CRUD |
| `ScheduleCreate/List/...` | `WorkflowOrchestrator` | Schedule management |

### Permission Model

The permission check follows a layered evaluation:

1. `PermissionModel::evaluate(tool_name, is_dangerous)` -- config-based rules (allow/notify/ask/deny)
2. `session_permission_mode()` -- session-level override (`BypassPermissions` converts Ask -> Allow, but never overrides Deny)
3. **Built-in trust auto-allow**: if `trust_tier == BuiltIn && agent_allowed_tools.contains(name)` -> Allow

## Phase 6: Response Delivery

After the loop exits (no tool calls in the LLM response):

1. `build_final_result()` constructs `AgentExecutionResult` with:
   - `content` -- the assistant's text response
   - `new_messages` -- all messages generated during the turn (assistant + tool results)
   - `cumulative_usage` -- total token counts across all iterations
   - `cumulative_cost` -- total cost in USD
   - `iterations` -- number of loop iterations
2. `ChatService` appends the assistant reply to dual transcripts
3. Emits `TurnEvent::Complete` through the progress channel
4. Presentation layer renders the response to the user

## Streaming Flow

For streaming responses, the flow differs at Phase 4:

```mermaid
sequenceDiagram
    participant P as Presentation
    participant AS as AgentService
    participant PP as ProviderPool
    participant LLM as LLM Provider

    AS->>PP: chat_completion_stream(request, route)
    PP->>LLM: streaming request
    loop Token chunks
        LLM-->>PP: StreamDelta
        PP-->>AS: StreamDelta
        AS->>P: TurnEvent::StreamDelta(text)
        P->>P: Render incremental text
    end
    LLM-->>PP: stream end (with tool_calls)
    PP-->>AS: final ChatResponse
    Note over AS: Continue tool loop<br/>as in non-streaming path
```

The provider pool wraps the stream with an `ActiveRequestGuard` (RAII) to ensure the per-provider concurrency counter is decremented even if the consumer aborts mid-stream.

## HITL (Human-in-the-Loop) Interrupts

Two types of HITL interrupts can pause the turn loop:

### Permission Request
When a tool requires user approval (`PermissionDecision::Ask`):
1. A `oneshot::channel()` is created
2. The `Sender` is inserted into `ctx.pending_permissions`
3. `TurnEvent::PermissionRequest` is emitted to the presentation layer
4. Execution blocks on `receiver.await`
5. User responds with Approve / AllowAllForSession / Deny

### AskUser Tool
When the agent invokes the `AskUser` tool:
1. The tool output is delivered to the user
2. A `oneshot::channel()` is inserted into `ctx.pending_interactions`
3. Execution blocks until the user provides a response
4. The response becomes the tool result, and the loop continues

## Error Recovery

| Error Type | Behavior |
|-----------|----------|
| LLM quota/rate-limit | Provider frozen, pool retries with next available provider |
| LLM auth error | Provider frozen permanently |
| Tool execution error | Error string returned as tool result, LLM sees it and adapts |
| Tool loop limit exceeded | Turn ends with `ToolLoopLimitExceeded`, partial results preserved |
| Cancellation | Turn ends with `Cancelled`, partial results and messages preserved |
| Context overflow | `ContextWindowGuard` triggers compaction or pruning |
