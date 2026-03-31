# Plan: Wire Real Workflow Execution into Manual Triggers

## Context

When the GUI Automation tab triggers a workflow or schedule manually, `SchedulerService::trigger_now()` and `SchedulerService::execute_workflow()` create fake execution records instantly marked "completed" with "(placeholder)" messages. No LLM call, no tool execution, no diagnostics trace. The goal is to replace these stubs with real agent-based DAG execution.

## Architecture

```
GUI commands (y-gui)
    |
    v
SchedulerService (y-service)  -- trigger_now() / execute_workflow()
    |
    v  WorkflowDispatcher::dispatch()
OrchestratorDispatcher (y-service)
    |
    v
WorkflowExecutor (y-agent/orchestrator) with registered TaskExecutors
    |
    +-- LlmCallExecutor       --> AgentService::execute()
    +-- ToolExecutionExecutor  --> tool_registry.execute()
    +-- SubAgentExecutor       --> agent_delegator.delegate()
    +-- FallbackLlmExecutor    --> handles Noop tasks (DSL-compiled) as LLM calls
```

Dependency constraint: `y-scheduler` cannot depend on `y-service`. The `WorkflowDispatcher` trait is defined in `y-scheduler`; implementation lives in `y-service`.

## Implementation Steps

### Step 1: WorkflowDispatcher trait in y-scheduler

**New file:** `crates/y-scheduler/src/dispatcher.rs`

- `DispatchResult` struct: `success`, `summary`, `output` (JSON), `duration_ms`, `error`
- `DispatchError` enum: `WorkflowNotFound`, `ParseError`, `ExecutionFailed`, `Internal`
- `WorkflowDispatcher` async trait: `async fn dispatch(&self, workflow_id: &str, parameter_values: Value) -> Result<DispatchResult, DispatchError>`

**Edit:** `crates/y-scheduler/src/lib.rs` -- add `pub mod dispatcher;` and re-exports.

### Step 2: Dispatcher slot in SchedulerManager

**Edit:** `crates/y-scheduler/src/manager.rs`

- Add field `dispatcher: Arc<Mutex<Option<Arc<dyn WorkflowDispatcher>>>>` to `SchedulerManager`
- Initialize as `Arc::new(Mutex::new(None))` in `new()`
- Add `set_dispatcher()` and `dispatcher()` methods
- Wire into `handle_fired_trigger()`: when dispatcher is Some, spawn real dispatch; when None, use existing placeholder `trigger_execution()`

### Step 3: Expose task outputs from WorkflowExecutor

**Edit:** `crates/y-agent/src/orchestrator/executor.rs`

- Add `pub fn all_outputs(&self) -> &HashMap<TaskId, TaskOutput>` to `WorkflowExecutor`

### Step 4: TaskExecutor implementations in y-service

**New module:** `crates/y-service/src/workflow_executors/`

- `mod.rs` -- declares and re-exports submodules
- `llm_call.rs` -- `LlmCallExecutor`: handles `TaskType::LlmCall`, calls `AgentService::execute()` with task's system_prompt and inputs as user query
- `tool_exec.rs` -- `ToolExecutionExecutor`: handles `TaskType::ToolExecution`, calls tool registry directly
- `sub_agent.rs` -- `SubAgentExecutor`: handles `TaskType::SubAgent`, delegates to `agent_delegator`
- `fallback_llm.rs` -- `FallbackLlmExecutor`: handles `TaskType::Noop` (DSL-compiled tasks), sends task name + inputs as prompt to LLM

All executors hold `Arc<ServiceContainer>` and implement `TaskExecutor` from `y_agent::orchestrator::task_executor`.

**Edit:** `crates/y-service/src/lib.rs` -- add `mod workflow_executors;`

### Step 5: OrchestratorDispatcher in y-service

**New file:** `crates/y-service/src/orchestrator_dispatcher.rs`

Implements `WorkflowDispatcher`:

1. Load `WorkflowRow` from `container.workflow_store`
2. Parse definition via `WorkflowDefinition::Expression/Toml` -> `.parse()` -> `ParsedWorkflow`
3. Create `WorkflowExecutor`, register all 4 task executors
4. Call `executor.execute(dag, checkpoint_store, inputs, input_mappings, output_mappings)`
5. Collect task outputs into `DispatchResult`

**Edit:** `crates/y-service/src/lib.rs` -- add `mod orchestrator_dispatcher;`

### Step 6: Rewrite SchedulerService trigger methods

**Edit:** `crates/y-service/src/scheduler_service.rs`

Both `trigger_now()` and `execute_workflow()`:

1. Create execution record with `status: Running` (not instantly Completed)
2. Check `manager.dispatcher()`:
   - **Some**: `tokio::spawn` async task that calls `dispatcher.dispatch()`, then updates execution record to Completed/Failed
   - **None**: fallback to current placeholder (backward compat for tests)
3. Return the Running execution record immediately (non-blocking GUI)

### Step 7: Wire dispatcher into ServiceContainer

**Edit:** `crates/y-service/src/container.rs`

Add `init_workflow_dispatcher()` method (same pattern as `init_agent_runner()`):

```rust
pub async fn init_workflow_dispatcher(self: &Arc<Self>) {
    let dispatcher = Arc::new(OrchestratorDispatcher::new(Arc::clone(self)));
    self.scheduler_manager.set_dispatcher(dispatcher).await;
}
```

### Step 8: Call from GUI/CLI init

**Edit:** `crates/y-gui/src-tauri/src/lib.rs` line ~118 -- add `container.init_workflow_dispatcher().await;` after `init_agent_runner()`

**Edit:** `crates/y-cli/src/main.rs` line ~203, `crates/y-cli/src/commands/tui_cmd.rs` line ~23 -- same

## Key Files

| File                                                      | Action                                                   |
| --------------------------------------------------------- | -------------------------------------------------------- |
| `crates/y-scheduler/src/dispatcher.rs`                    | New                                                      |
| `crates/y-scheduler/src/lib.rs`                           | Edit (add module + re-exports)                           |
| `crates/y-scheduler/src/manager.rs`                       | Edit (dispatcher field + methods + handle_fired_trigger) |
| `crates/y-agent/src/orchestrator/executor.rs`             | Edit (add all_outputs accessor)                          |
| `crates/y-service/src/workflow_executors/mod.rs`          | New                                                      |
| `crates/y-service/src/workflow_executors/llm_call.rs`     | New                                                      |
| `crates/y-service/src/workflow_executors/tool_exec.rs`    | New                                                      |
| `crates/y-service/src/workflow_executors/sub_agent.rs`    | New                                                      |
| `crates/y-service/src/workflow_executors/fallback_llm.rs` | New                                                      |
| `crates/y-service/src/orchestrator_dispatcher.rs`         | New                                                      |
| `crates/y-service/src/lib.rs`                             | Edit (add modules)                                       |
| `crates/y-service/src/scheduler_service.rs`               | Edit (rewrite trigger_now + execute_workflow)            |
| `crates/y-service/src/container.rs`                       | Edit (add init_workflow_dispatcher)                      |
| `crates/y-gui/src-tauri/src/lib.rs`                       | Edit (init call)                                         |
| `crates/y-cli/src/main.rs`                                | Edit (init call)                                         |
| `crates/y-cli/src/commands/tui_cmd.rs`                    | Edit (init call)                                         |

## Verification

1. `cargo clippy --workspace -- -D warnings` passes
2. `cargo check --workspace` passes
3. `cargo doc --workspace --no-deps` passes
4. `cargo fmt --all` passes
5. Manual test: launch GUI, create a workflow (DSL or TOML), trigger it, observe:
   - Execution record shows "running" then transitions to "completed" or "failed"
   - Diagnostics panel shows real LLM traces
   - Response summary contains actual LLM output
