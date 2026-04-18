# Middleware & Hooks

The middleware system provides a uniform interception layer for all operations in y-agent. Guardrails, event handling, and HITL protocols are all implemented as middleware.

## Architecture

```mermaid
graph TB
    subgraph HookSystem["HookSystem (unified facade)"]
        direction TB
        REG["HookRegistry"]
        EB["EventBus<br/>(tokio::broadcast)"]
    end

    subgraph Chains["Middleware Chains"]
        CT["Context Chain"]
        TL["Tool Chain"]
        LM["LLM Chain"]
        CP["Compaction Chain"]
        MM["Memory Chain"]
    end

    subgraph Guardrails["Guardrail Middleware"]
        TG["ToolGuardMiddleware<br/>priority: 10"]
        LD["LoopDetectorMiddleware<br/>priority: 20"]
        LG["LlmGuardMiddleware<br/>priority: 900"]
    end

    HookSystem --> Chains
    TG --> TL
    LD --> TL
    LG --> LM

    subgraph HookHandlers["Hook Handlers (feature-gated)"]
        CMD["Command Handler"]
        HTTP["HTTP Handler"]
        PA["Prompt-Agent Handler"]
    end
```

## Middleware Chain Execution

**Entry:** `MiddlewareChain::execute()` in `y-hooks/src/chain.rs`

```mermaid
sequenceDiagram
    participant C as Caller
    participant MC as MiddlewareChain
    participant M1 as Middleware A<br/>(priority 10)
    participant M2 as Middleware B<br/>(priority 20)
    participant M3 as Middleware C<br/>(priority 100)

    C->>MC: execute(&mut ctx)
    MC->>M1: execute(&mut ctx)
    alt Continue
        M1-->>MC: MiddlewareResult::Continue
        MC->>M2: execute(&mut ctx)
        alt ShortCircuit
            M2-->>MC: MiddlewareResult::ShortCircuit
            Note over MC: Remaining middleware skipped
        else Continue
            M2-->>MC: MiddlewareResult::Continue
            MC->>M3: execute(&mut ctx)
            M3-->>MC: MiddlewareResult::Continue
        end
    else Abort
        M1->>M1: ctx.abort("reason")
        Note over MC: ctx.aborted = true<br/>Chain stops immediately
    end
    MC-->>C: ctx (with results)
```

### Execution Rules

1. Middleware entries sorted by `(priority, insertion_order)` -- lower priority number executes first
2. Equal priorities preserve insertion order (stable sort)
3. `ctx.aborted = true` stops the chain immediately
4. `MiddlewareResult::ShortCircuit` skips remaining middleware
5. Errors propagate upward (stop the chain)
6. Duplicate middleware names are rejected at registration time

### MiddlewareContext

```
MiddlewareContext {
    chain_type: ChainType,        // Context | Tool | Llm | Compaction | Memory
    payload: serde_json::Value,   // chain-specific data
    metadata: serde_json::Value,  // additional context
    aborted: bool,                // set by middleware to stop execution
    abort_reason: Option<String>, // human-readable reason for abort
}
```

### Chain Types

| Chain | When Executed | Payload |
|-------|-------------|---------|
| `Context` | During context assembly | Context items, token counts |
| `Tool` | Before/after tool execution | `{tool_name, arguments, phase}` |
| `Llm` | Before/after LLM calls | Request/response data |
| `Compaction` | During context compaction | Compaction parameters |
| `Memory` | During memory operations | Memory read/write data |

## Guardrail Middleware

### ToolGuardMiddleware (priority 10)

Intercepts tool calls for permission enforcement:

```mermaid
flowchart TD
    A["Tool call intercepted"] --> B["PermissionModel::evaluate()"]
    B --> C{"Decision?"}
    C -->|Allow| D["MiddlewareResult::Continue"]
    C -->|Deny| E["ctx.abort(reason)"]
    C -->|Notify| F["Log + Continue"]
    C -->|Ask| G["HITL prompt"]
    G -->|Approved| D
    G -->|Denied| E
```

### LoopDetectorMiddleware (priority 20)

Detects 4 types of loop patterns:

| Pattern | Detection | Example |
|---------|-----------|---------|
| **Repetition** | Same tool + same arguments N times | `FileRead("/a")` called 5 times |
| **Oscillation** | Alternating between two states | Write A -> Undo A -> Write A -> Undo A |
| **Drift** | Incremental changes without progress | Temperature 0.1 -> 0.2 -> 0.3 -> ... |
| **Redundant** | Tool call whose result was already obtained | Re-searching after results were used |

When a loop is detected, the middleware can:
- Inject a warning message into the context (soft intervention)
- Abort the tool chain (hard intervention, based on severity)

### LlmGuardMiddleware (priority 900)

Post-LLM output validation (highest priority = runs last in the chain):
- Content filter checks
- Structural validation of tool calls
- Response format compliance

### Additional Guardrail Components

| Component | Responsibility |
|-----------|---------------|
| `PermissionModel` | 4-level evaluation: allow / notify / ask / deny |
| `TaintTracker` | Tracks data flow taint through tool results |
| `RiskScorer` | Composite risk score from multiple signals |
| `HitlProtocol` | Human-in-the-loop with configurable timeout |
| `HitlHandler` | Manages the HITL interaction flow |
| `StructuralValidator` | Validates tool call structure against schemas |
| `CapabilityGapMiddleware` | Detects when agent lacks required capabilities |
| `GuardrailManager` | Hot-reloadable config via `RwLock<GuardrailConfig>` |

## Event Bus

**Component:** `EventBus` in `y-hooks/src/lib.rs`

The event bus uses `tokio::broadcast` channels for async event distribution:

```mermaid
graph LR
    subgraph Publishers
        CS["ChatService"]
        AS["AgentService"]
        TE["ToolExecutor"]
        SM["SessionManager"]
    end

    subgraph Bus["EventBus (broadcast)"]
        CH["tokio::broadcast::channel"]
    end

    subgraph Subscribers
        DS["DiagnosticsSubscriber"]
        EC["ExperienceCaptureSubscriber"]
        SA["SkillUsageAuditSubscriber"]
        HH["HookHandlerExecutor"]
    end

    CS --> CH
    AS --> CH
    TE --> CH
    SM --> CH
    CH --> DS
    CH --> EC
    CH --> SA
    CH --> HH
```

### Event Types

Events correspond to the 24 `HookPoint` variants defined in y-core:

| HookPoint | When Emitted |
|-----------|-------------|
| `PreLlmCall` | Before sending request to LLM |
| `PostLlmCall` | After receiving LLM response |
| `PreToolExecute` | Before executing a tool |
| `PostToolExecute` | After tool execution completes |
| `SessionCreated` | New session created |
| `SessionForked` | Session forked/branched |
| `ToolGapDetected` | Agent requested unavailable tool |
| `DynamicAgentCreated` | Agent created at runtime |
| `ContextOverflow` | Context window exhausted |
| `PostSkillInjection` | Skills injected into context |
| `CompactionTriggered` | Context compaction started |
| ... | (24 total hook points) |

## HITL (Human-in-the-Loop) Protocol

```mermaid
sequenceDiagram
    participant A as Agent Loop
    participant HP as HitlProtocol
    participant P as Presentation Layer
    participant U as User

    A->>HP: request_permission(tool, args)
    HP->>HP: create oneshot channel
    HP->>P: TurnEvent::PermissionRequest
    P->>U: Display permission prompt

    alt User approves
        U->>P: "Approve"
        P->>HP: send(Approve)
        HP-->>A: PermissionGranted
    else User approves all (session)
        U->>P: "Allow All"
        P->>HP: send(AllowAllForSession)
        HP->>HP: set BypassPermissions
        HP-->>A: PermissionGranted
    else User denies
        U->>P: "Deny"
        P->>HP: send(Deny)
        HP-->>A: PermissionDenied
    else Timeout
        HP->>HP: timeout expires
        HP-->>A: PermissionDenied (timeout)
    end
```

### Two HITL Entry Points

1. **Permission requests**: When `PermissionModel` returns `Ask` for a tool call
2. **AskUser tool**: When the agent explicitly requests user input

Both use `oneshot::channel()` inserted into the `ToolExecContext`'s pending maps. The presentation layer is responsible for rendering the prompt and sending the response.

## Hook Handlers (Feature-Gated)

When the `hook_handler` feature is enabled, hook events can trigger automated responses:

| Handler | Trigger | Action |
|---------|---------|--------|
| `CommandHandler` | Hook event matches pattern | Execute shell command |
| `HttpHandler` | Hook event matches pattern | Send HTTP request |
| `PromptAgentHandler` | Hook event matches pattern | Delegate to an agent |

Each handler returns a `HookDecision` that can modify the ongoing operation or allow it to proceed unchanged.

## Integration Points

### Tool Execution Integration

In `ToolExecutor::execute()`:

```
Pre-execution:
  payload = { tool_name, arguments, phase: "pre" }
  -> if ctx.aborted: return ToolRegistryError::ExecutionError

[tool.execute()]

Post-execution:
  payload = { tool_name, result: output.content, phase: "post" }
  -> failures only logged, never propagated
```

### GuardrailManager Hot-Reload

`GuardrailManager` wraps its config in `RwLock<GuardrailConfig>`, enabling runtime config changes without restart:

```rust
guardrail_manager.update_config(new_config); // takes write lock, swaps config
```

All middleware reads the config via read locks, so updates take effect on the next middleware execution.
