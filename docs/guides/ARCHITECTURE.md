# y-agent Harness Architecture

This is the canonical contributor-level architecture document. Current code,
tests, Cargo features, and public configuration remain the implementation source
of truth. Update this document when a cross-crate contract changes; do not add a
new parallel design document.

## Positioning

y-agent is an Agent Harness: the execution system around an LLM that turns a
user goal into controlled work. It is in the same product category as Codex,
while remaining model-provider and interface agnostic.

The harness owns:

- goal, constraint, decision, and progress continuity;
- execution-mode selection and orchestration;
- context, knowledge, skill, and tool assembly;
- provider routing and model communication;
- sandboxing, permissions, guardrails, and HITL;
- checkpoints, transcripts, journaling, and rewind;
- diagnostics, cost accounting, replay, and evolution feedback.

## Capability Maturity

| Capability | Current implementation | Boundary |
| --- | --- | --- |
| Goal semantics | `y-context` working memory and structured handoff documents preserve goals, constraints, decisions, and progress | No standalone persistent Goal service or CRUD API yet |
| Plan mode | `y-service/src/plan_orchestrator/` creates reviewed structured plans and executes dependency-aware phases | Plans are task-scoped execution state, not historical design documents |
| Loop mode | `y-service/src/loop_orchestrator.rs` iterates execution and self-review until convergence or limits | Used for exploratory work whose full graph is not known up front |
| Self-orchestration | `y-agent` workflow meta-tools, DAG engine, agent registry, delegation, and `y-service` orchestration | Reusable coordination belongs in workflows or agents, not presentation code |
| Skill evolution | `y-skills` experience, extraction, refinement, regression, lineage, and version modules plus `y-service/src/skill_evolution.rs` | Evolution remains versioned and approval-controlled |
| Knowledge | `y-knowledge` ingestion and retrieval with `y-service/src/knowledge_service.rs` wiring | Qdrant is optional; local retrieval remains available |
| Observability | `y-diagnostics` local trace model, SQLite store, cost/replay, and optional Langfuse native ingestion | A generic OpenTelemetry SDK/exporter is not currently wired |

## Layering

Dependencies point inward toward `y-core`.

```text
Presentation
  y-cli                 CLI and TUI
  y-web                 REST API and SSE
  y-gui/src-tauri       Tauri shell
  crates/y-gui          shared React frontend
          |
Service
  y-service             business orchestration and dependency wiring
          |
Orchestration
  y-agent               agents, delegation, workflows, DAG execution
  y-bot                 external chat-platform adapters
          |
Capabilities
  y-tools, y-skills, y-runtime, y-scheduler, y-browser, y-journal
          |
Middleware
  y-hooks, y-guardrails, y-prompt, y-mcp
          |
Infrastructure
  y-provider, y-session, y-context, y-storage, y-knowledge, y-diagnostics
          |
Core
  y-core                shared traits and boundary types
```

`y-service` is the only business-logic hub. Presentation crates may validate
transport input and render output, but they must not implement domain workflows.

## Agent Turn Lifecycle

```text
1. Accept the user request and execution-mode preference.
2. Load the session, goal context, checkpoints, and relevant configuration.
3. Assemble persona, memory, knowledge, skills, tool descriptions, and history.
4. Route the request to a provider and record the generation observation.
5. Interpret the response:
   - return a result;
   - execute a tool;
   - request user approval or clarification;
   - delegate to a sub-agent;
   - create or execute a workflow;
   - create or update a plan.
6. Apply guardrails and runtime isolation before side effects.
7. Persist transcripts, checkpoints, diagnostics, and file-journal state.
8. Capture eligible experience for later skill evolution.
```

All interfaces use the same service-layer behavior. Host-specific differences
belong in transport and platform adapters.

## Execution Modes

### Fast

Fast mode runs the normal agent loop without creating a structured plan. It is
appropriate when the task is small or immediately actionable.

### Plan

Plan mode first converts the goal into a structured, reviewable plan. Execution
then respects phase dependencies and can use bounded parallelism. Plan state is
persisted so interrupted work can be reviewed and resumed.

### Loop

Loop mode is iterative. Each round performs work, evaluates progress, records
insights, and decides whether another round is required. It is appropriate for
research, diagnosis, and tasks whose graph emerges during execution.

Mode selection changes orchestration strategy, not ownership of tools, safety,
storage, or diagnostics.

## State and Recovery

| State | Owner |
| --- | --- |
| Session metadata and operational records | `y-session` and `y-storage` |
| Context and display transcripts | session/storage implementations wired by `y-service` |
| Workflow definitions and checkpoints | `y-agent`, `y-storage`, and `y-service` |
| File mutation history and rewind | `y-journal` and `y-service` |
| Knowledge indexes | `y-knowledge` |
| Skill versions, lineage, and experience | `y-skills` |
| Traces, observations, scores, and cost | `y-diagnostics` |

SQLite runs in WAL mode for local operational durability. Optional external
systems must fail independently and must not block the agent execution path.

## Evolution Loop

```text
task execution
  -> experience capture
  -> pattern extraction
  -> skill proposal
  -> validation and regression checks
  -> human approval where required
  -> new version and lineage record
  -> measured reuse in later tasks
```

Knowledge, skills, tools, and workflows have different roles:

- Knowledge stores external facts and domain material.
- Skills store reusable reasoning and operating instructions.
- Tools perform deterministic external actions.
- Workflows coordinate reusable multi-step execution.

## Observability Boundary

The built-in diagnostics model records traces, generations, tool calls,
sub-agent work, usage, cost, status, and scores. The optional `langfuse` feature
exports this data asynchronously through Langfuse's native batch ingestion API.

The old documentation described that bridge as a generic OTLP exporter. The
current implementation does not depend on an OpenTelemetry SDK and does not
expose a general OTLP endpoint. A future OTel adapter should subscribe to the
same diagnostics events, remain feature-gated, and preserve failure isolation.

## Change Rules

1. Define cross-crate contracts in `y-core` only when multiple subsystems need
   the abstraction.
2. Put business orchestration in `y-service`.
3. Add discrete capabilities to the owning crate and feature-gate new
   subsystems.
4. Keep presentation layers thin and keep desktop/web UI differences in
   adapters.
5. Add tests before behavior changes.
6. Update this document or an owning standard instead of creating a new design
   draft.
7. Record temporary implementation work in `.claude/plans`; remove or archive
   it outside the maintained documentation tree after completion.
