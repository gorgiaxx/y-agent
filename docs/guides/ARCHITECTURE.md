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
| Self-orchestration | `y-agent` workflow meta-tools, DAG engine, durable dynamic-agent lifecycle and proposals, agent registry, and delegation plus `y-service` Auto-mode decisions and bounded cross-asset reuse planning | Dynamic creation/update/deactivation is validated and permission-bounded; repeated regression evidence can be rolled back or refined into a validated candidate, while strong existing assets are surfaced before new ones are created |
| Skill evolution | `y-skills` durable experience/proposal journals, extraction, validation, content-addressed versions, regression, and rollback plus `y-service` turn capture and governed promotion orchestration | Validated promotion, automatic metric-triggered rollback, bounded Auto-mode skill reuse, and idempotent trace-linked user feedback are wired |
| Knowledge | `y-knowledge` ingestion, canonical filtered retrieval requests, collection isolation, weighted RRF retrieval, provenance, and deterministic IR evaluation with `y-service` wiring | Qdrant is optional; local retrieval remains available, while richer candidate-loss and citation observability remain incomplete |
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
  -> durable experience capture
  -> pattern extraction
  -> aggregated pending skill proposal
  -> tool-free candidate refinement
  -> supervised approval/rejection/deferral
  -> validation and regression checks
  -> new version and lineage record
  -> measured reuse in later tasks
```

Auto-mode skill reuse is service-owned: `ChatService::prepare_turn` may select
at most two strong existing skill matches when the caller did not explicitly
choose skills. The selected names are persisted in user-message metadata,
injected by the context pipeline for every presentation layer, and captured in
the turn experience used by regression evaluation. The resolved Fast, Plan, or
Loop mode and selected skills are persisted on the final assistant message as
an orchestration decision and on the diagnostics trace. `DiagnosticsService`
aggregates those traces into per-mode success, token, cost, and duration
metrics, including failed and cancelled executions.

Unified capability search covers tools, skills, agents, and durable workflows.
Workflow matches are bounded and activate the dangerous `WorkflowRun` signal
tool, whose service handler resolves an existing workflow by ID or name and
passes explicit parameters through the scheduler dispatcher. This lets agents
reuse a matching workflow instead of recreating its coordination graph.

Before a text-chat turn executes, `CapabilityReusePlanner` performs a bounded,
deterministic comparison against existing skills, callable agents, tools, and
durable workflows. It keeps at most one strong recommendation per asset type
and four overall, persists the decision in trace and assistant-message
metadata, and injects a reuse-before-create guard into the agent context. The
guard requires the model to explain the missing capability before creating a
new asset; it does not bypass model review or execute a workflow automatically.

Runtime-created agents use an append-only JSONL definition journal in
`y-agent`. `y-service` rehydrates active definitions into `AgentRegistry` at
startup and owns create, update, and deactivation synchronization. Runtime
definitions materialize the creator-intersected tool and numeric limits before
delegation; dynamic descendants inherit the persisted effective permission
snapshot and reduced delegation depth. Delegated traces record the durable
agent ID and version, and diagnostics can aggregate success, failure,
cancellation, token, cost, and duration metrics per version for regression
review. The `AgentEvaluate` tool requires repeated samples and reports
adjacent-version success-rate regressions without mutating the active
definition. Findings create idempotent append-only rollback proposals. The
read-only `AgentProposalList` exposes evidence and decision history.
`AgentProposalRefine` delegates to the tool-free, structured-output
`agent-refiner`, validates its candidate against immutable identity fields and
the current effective tool permissions, and stores the candidate without live
mutation. `AgentProposalDecide` is dangerous and applies approve/reject/defer.
Approval either restores a historical snapshot or applies the validated
candidate as a new monotonic version and synchronizes the live registry, while
rejection and deferral leave the active definition unchanged.

Skill evolution follows the same proposal-first boundary. The read-only
`SkillProposalList` and `SkillProposalRefine` tools expose evidence and delegate
candidate drafting to the tool-free, structured-output `skill-refiner`. The
candidate root instructions and rationale are journaled without modifying the
active skill. `SkillProposalDecide` is dangerous: approval revalidates and
content-addresses the candidate, activates a new version, and refreshes skill
discovery; rejection and deferral only append decision history. Parent
snapshots remain available for rollback and regression recovery.

Runtime-created script tools use a service-owned append-only JSONL lifecycle.
`ToolCreate`, `ToolUpdate`, and `ToolDelete` are dangerous, while `ToolGet` and
`ToolList` are read-only. All five are hidden unless
`tools.allow_dynamic_tools` is enabled. `y-service` validates names,
interpreters, source size, and built-in collisions; synchronizes exact versions
with the live registry; rehydrates active definitions at startup; and records
execution evidence. Dynamic tools still execute through the normal registry,
guardrail, permission, and `RuntimeManager` path. The built-in `tool-engineer`
must search for reuse first and use these lifecycle tools for activation.

Explicit feedback uses a caller-supplied idempotency ID and diagnostics trace
ID. The service stores a `UserFeedback` score, annotates the trace, converts
selected-skill feedback into provenance-tagged durable experience, and lets
strong positive or negative feedback override the observed trace outcome in
adaptation metrics. Negative feedback requires a correction comment. Repeated
feedback-adjusted dynamic-agent samples therefore flow through the same
`AgentEvaluate` proposal gate rather than mutating an agent directly.
Assistant feedback controls are rendered only when a persisted diagnostics
trace ID is available. Positive feedback can be submitted directly; negative
feedback requires an actionable correction. Desktop and web transports share
the same `/api/v1/chat/feedback` service contract and caller-generated
idempotency IDs.

The legacy `y-guardrails::CapabilityGapMiddleware` remains an isolated library
stub and is not the production self-healing path. Production adaptation uses
the service-owned reuse planner, explicit delegation to the governed refiner or
builder agents, and dangerous lifecycle decisions. No subsystem may report a
gap as resolved until the durable service mutation and live-registry
synchronization have succeeded.

Knowledge retrieval uses one `KnowledgeRetrievalRequest` contract for service,
tool, and automatic context paths. Collection, resolution, domain, fallback,
and limit semantics are applied by `y-knowledge`; the hybrid retriever owns the
single configured similarity threshold. Automatic context retrieval searches
only the collections selected for the turn, uses bounded L0 material, and
includes source, collection, and resolution provenance in injected context and
public search results.

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
