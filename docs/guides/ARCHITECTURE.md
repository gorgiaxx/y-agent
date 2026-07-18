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
| Delegated workspace isolation | `y-service` resolves effective write capability and provisions `y-runtime` Git worktrees for interactive delegated writers; results include bounded patch evidence and durable resumable snapshots | Worktrees never grant permissions or auto-merge; non-Git copy isolation and automatic conflict resolution are not implemented |
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

## Permission and HITL Boundary

Runtime tool authorization has one authoritative decision path. y-guardrails
owns policy evaluation; y-service supplies the tool input and session mode,
delivers HITL requests, and executes the tool only after approval.

The decision order is:

1. Apply a matching shell exec-policy decision.
2. Enforce explicit deny rules.
3. Record matching ask rules.
4. Invoke Tool::check_permissions with the real ToolInput.
5. Apply PermissionMode semantics.
6. Resolve allow or notify rules and the configured global default.
7. Convert unresolved requests to ask, or to deny in DontAsk mode.
8. Apply the service-owned OperationMode prompt policy.

Deny is bypass-immune. PermissionMode::BypassPermissions and
OperationMode::FullAccess may convert ordinary ask decisions to allow, but
must never convert an explicit rule deny, exec-policy deny, or tool-specific
deny to allow. Built-in agent declarations may provide an allow signal, but
they pass through the same deny and exec-policy stages.

HITL transport remains presentation-agnostic. y-service registers a pending
approval, emits the shared permission event, waits with the configured
guardrail timeout and cancellation token, removes pending state on every exit
path, and treats timeout, cancellation, or channel loss as denial.

## Workspace Activation Trust Boundary

Workspace trust and tool permission are separate decisions. Workspace trust
controls whether project-origin configuration may become active. Permission,
guardrails, sandboxing, and HITL control whether an operation from already
active configuration may execute. FullAccess and BypassPermissions affect only
the latter and never imply workspace trust.

Project configuration sources are `y-agent.toml` and split `config/*.toml`
files associated with a workspace. y-service must resolve their canonical
workspace identity and consult the user-owned trust store before merging those
sources into the effective configuration. Unknown, explicitly untrusted,
missing, or invalid workspace identities fail closed: their project sources are
recorded as blocked provenance and are not merged. User configuration,
environment overrides, CLI overrides, and built-in defaults remain available.

The trust decision is made before configuration flattening. Existing activation
paths for hooks, MCP servers, providers, runtimes, browser integration, and
other configured capabilities therefore receive only trusted project values or
user-owned values; they do not independently reinterpret folder trust. Future
project-origin LSP, plugin, or environment loaders must enter through the same
service-owned provenance and trust boundary.

Trust grants are explicit, persisted outside the workspace, auditable, and tied
to the canonical workspace path. A moved workspace has a different identity and
returns to unknown. A symlink resolving to an already trusted canonical path
shares that trust. Corrupt trust state fails closed and must not be overwritten
by read-only status checks.

CLI, Tauri, and y-web expose equivalent trust status and mutation adapters over
the same WorkspaceService methods. The shared frontend reaches those adapters
through its transport command map. Presentation code may display and submit a
decision, but it must not infer trust from operation mode or activate project
configuration directly.

## Safe File Mutation Contract

File mutation identity is capability metadata, not a list of tool names.
Built-in, dynamic, and MCP tools that write files must declare a filesystem
mutation capability containing the operation and the argument names that carry
source and destination paths. Undeclared tools are not treated as file writers;
declarations are explicit so third-party writers enter the same recovery and
audit path without name-based inference.

`FileRead` returns a SHA-256 `content_hash` computed from the same raw bytes that
were read. `FileEdit` accepts an optional `expected_content_hash`. When present,
the tool must compare it with the current file immediately before mutation and
reject a mismatch as `stale_file` without writing. Rejections include the
current hash and bounded fresh context. Same-process edits to one absolute path
are serialized so two writers using one prior hash cannot both commit. Missing
expectations remain backward compatible but do not provide lost-update
protection.

Successful declared writes produce one `FileMutationEvent` owned by `y-core`.
The event includes tool-call, session, and agent identity; actual operation;
absolute source/destination paths; before/after hashes; content-addressed
references; and creation status. File bodies are never embedded in this event.
`y-service` captures the pre-state before dispatch, updates rewind history,
captures the post-state after success, appends the event to `y-journal`, adds it
to diagnostics/tool metadata, and emits it to presentation subscribers. Journal
failure after a completed side effect is reported as degraded observability in
the tool result and logs; it must not be silently ignored or reported as a safe
retry. Failure to resolve or capture the declared pre-state fails closed before
the tool executes.

File edit errors are machine-distinguishable: `stale_file`, `ambiguous_edit`,
`edit_target_not_found`, `file_not_found`, and `permission_denied`. This allows
the model and presentation layers to re-read, request clarification, or stop
without parsing human error strings.

## Replayable Session Event Contract

Critical presentation events are persisted before live delivery. `y-core`
defines the event envelope, retention class, per-session sequence, and global
event cursor. `y-storage` appends events to SQLite. `y-service` decides which
events are durable and publishes them before invoking presentation adapters.
Presentation layers render or transport events but do not decide durability.

Each event has a globally monotonic `event_id` and a monotonic `seq` within its
session. The `(session_id, seq)` pair is unique and provides session-local
ordering. The global ID is the SSE cursor because the shared web frontend uses
one EventSource connection for multiple sessions. Service restart seeds both
orders from SQLite rather than resetting an in-memory counter.

Durable events include turn start and terminal state, tool start/result,
permission and user-interaction requests, plan review requests, file mutation
metadata, and non-reconstructable errors or lifecycle changes. Token deltas,
reasoning deltas, partial image chunks, and heartbeats remain ephemeral. A
durable append failure prevents the event from being advertised as replayable;
the failure is logged and surfaced by the owning service path.

SSE subscribers establish live buffering before querying replay. Replay emits
events after `Last-Event-ID` or an explicit cursor in ascending global order,
then drains buffered live events while dropping IDs already replayed. A lagged
session-filtered subscriber recovers from SQLite using its last delivered ID.
Unresolved permission, AskUser, and plan-review correlations are independently
queryable from service-owned pending state and are replayed to late subscribers
even when no historical cursor was supplied.

## Tool Runtime Notification Contract

`y-hooks` is the tool control plane: it may allow, deny, modify, or audit a tool
call before and after execution. `ToolRuntimeEvent` is the execution data plane:
it reports process output, progress, resource changes, and terminal state after
the permission decision. A runtime notification never grants authority and
must not be interpreted as a hook decision.

`y-core` owns the transport-neutral event and sink contracts. `y-runtime`
reports runtime facts to the injected sink but does not persist session state,
publish presentation events directly, or invoke a model. `y-service` validates
session ownership, assigns retention, persists through `session_events`, and
only then broadcasts to presentation adapters. y-web and Tauri expose the same
`tool:runtime` payload; React updates background-task state without owning
process policy.

Process start and terminal events are durable. Stdout and stderr chunks are
short-lived, bounded per process correlation, and may be removed without
affecting the durable terminal result. Runtime execution must remain independent
of notification health: sink failure is logged as degraded observability and
cannot terminate, retry, or alter the process. Explicit poll, write, and kill
operations remain authoritative control actions and are session-scoped.

Automatic completion wake-up is a separate service-layer policy and is disabled
unless its feature and configuration are enabled. It may form a new turn only
for an idle session, must suppress active plan/loop execution unless explicitly
allowed by the orchestrator, deliver a process at most once, enforce per-session
budget and cooldown, and skip user-killed, block-waited, or already-consumed
results. Runtime and presentation layers can report completion but cannot wake
the model themselves.

`SessionState` owns a per-session active-turn counter; presentation-owned run
maps are cancellation adapters and are not an authority for wake eligibility.
`BackgroundWakePolicy` atomically reserves a task before preparation and spends
budget only after the shared chat worker durably starts. Explicit poll/write
operations hold a short observation claim so a racing terminal event is
suppressed only when the caller actually consumes the terminal result. Kill
intent permanently suppresses the task.

The synthetic input comes from `config/prompts/background-task-completion.md`
and carries `background_auto_wake` metadata. Execution uses
`ChatService::prepare_turn` and the shared chat worker, preserving provider
selection, permission middleware, HITL requests, cancellation, diagnostics,
and durable event replay. The persisted and live `ChatStarted.kind` is
`background_auto_wake`; Web SSE and Tauri only translate the service-owned
event channel.

## Sampling Preflight and Compaction Lifecycle

Context optimization has two boundaries. Post-turn optimization reduces the
persisted context transcript for future turns. Sampling preflight runs before
every provider request, including later iterations of a tool loop, and protects
the immediate request from exceeding the selected route's context window.
Provider adapters report typed overflow errors but do not initiate compaction.

`y-context` owns deterministic token estimation, history fingerprints, and
safe compaction ranges. A range must never separate an assistant tool call from
its corresponding tool-result messages. The active system prompt and other
request-only context remain outside transcript compaction. `y-service` owns the
prefire task lifecycle, cache validation, transcript mutation, in-memory
sampling recovery, and retry limits.

When persisted transcript usage crosses the prefire threshold but remains below
the hard compaction threshold, y-service may start one bounded background
compaction for that session. Prefire does not mutate history. Its result records
the exact compacted range, compaction configuration, and a SHA-256 fingerprint
of the source prefix. A result is usable only while that prefix fingerprint is
unchanged; appended recent messages may remain outside the compacted range.
Stale results are discarded.

Compaction failures have explicit lifecycle classes. Deterministic failures are
suppressed for the same fingerprint until the source or configuration changes.
Transient failures use bounded retry/backoff. An LLM failure or empty summary
must not rewrite the transcript with a placeholder and discard history. Handoff
remains the preferred persisted optimization where configured; a valid
prefired compaction may be used only when handoff is unavailable or fails.

If a provider still reports context-window overflow, the current sampling
request may perform one emergency in-memory compaction and retry once. Partial
streaming output, cancellation, or a second overflow terminates recovery. This
retry does not independently mutate the persisted transcript; the normal
post-turn path remains responsible for durable optimization.

## LSP Code Intelligence Boundary

Language-server integration is an optional, feature-gated capability. Trusted
user or workspace configuration declares server executables, arguments,
language identifiers, file extensions, and project-root markers. Configuration
activation passes through the existing workspace trust/provenance boundary;
unknown or untrusted project configuration cannot start a language server.
The harness never installs or downloads an LSP server implicitly.

`y-service` owns project detection, server selection, JSON-RPC/LSP lifecycle,
request correlation, cancellation, bounded restart/backoff, tracked-document
replay, and result shaping. Project-root search stops at the trusted working or
additional-read root, so a marker in a parent directory cannot expand the
workspace boundary. `y-runtime` owns approved long-running process execution,
event publication, and stdin/stdout transport. Runtime events identify these
processes as `LspServer`, not `ShellExec`. `y-tools` exposes read-only LSP tool
contracts, while the service intercepts those calls and dispatches them to the
manager. Presentation layers do not own server state or LSP policy.

The first supported operations are definition, references, hover, document and
workspace symbols, and diagnostics. Rename, formatting, code actions, and all
workspace-edit operations are excluded. LSP responses are evidence only: they
do not grant file-write authority, execute returned commands, or bypass normal
tool activation, guardrails, sandboxing, permission checks, and HITL.

One sequential client is keyed by session, configured server, and bounded
project root. Transport and framing failures use bounded exponential restart;
ordinary JSON-RPC server errors are returned without restarting. Open documents
use monotonic versions and replay only after initialize succeeds. Session
cleanup sends `shutdown` and `exit` before force-closing the runtime-managed
process. Server-initiated JSON-RPC requests receive bounded client responses so
they cannot deadlock a pending tool request. Tool cancellation propagates to
the active JSON-RPC request through `$/cancelRequest`.

## Capability Pack Boundary

A Capability Pack is a feature-gated, local delivery unit for existing skills,
agents, workflows, MCP declarations, hook declarations, and LSP declarations.
It is not an executable plugin format, a marketplace package, or a new runtime
authority. Each capability remains owned by its existing registry or service;
`y-service` owns staging, validation, permission diff, installation
transactions, compensation, update, rollback, and removal.

The manifest is explicit and versioned. Unknown fields, unsupported schema
versions, duplicate resource identities, absolute paths, parent traversal,
symlinks, canonical path escapes, type mismatches, and SHA-256 mismatches fail
closed before any live store is mutated. Successful validation retains pack and
resource provenance so later lifecycle operations do not reconstruct origin
from flattened configuration.

Declarative installation and executable activation are separate phases. MCP,
hook, and LSP declarations preserve origin and enter the existing workspace
activation trust, Guardrail, and HITL path before activation. OperationMode and
PermissionMode do not approve a pack or its executable declarations. Pack
failure must restore the previous installed version; removal may revoke only
assets and grants owned by that pack version. The canonical format and safety
rules are defined in `docs/standards/CAPABILITY_PACK_STANDARD.md`.

Declarative installation uses a compensation transaction because skills and
agents are filesystem/registry state while workflows are SQLite state; no
single ACID transaction spans all owners. Preview is deterministic, replacement
requires explicit approval, and executable declarations remain inactive. Each
mutation captures a snapshot before apply and failures restore snapshots in
reverse order, including the resource whose apply returned an error. This
transaction is durably journaled: snapshots and transitions are persisted
before mutation, journal failures after mutation trigger immediate
compensation, and service startup reverses non-terminal records. Persisted
compensation failure blocks automatic recovery. Skill, agent, and workflow
resources also complete semantic preflight as one set before live mutation.
Managed installation has an explicit durable commit-decision boundary. Before
that marker, restart compensates; after it, restart idempotently publishes the
installed-version ownership record and completes commit. Resource ownership is
exclusive across packs. Initial updates are monotonic within one pack and keep
the same declarative identity set. Rollback unwinds one committed version;
remove repeatedly unwinds the version stack to the pre-pack state. Interrupted
rollback resumes from the durable transaction state, and ownership is repaired
after restoration. Snapshot payloads are conservatively retained without
automatic GC. Presentation commands remain unavailable until executable
activation has an equivalent trust and HITL contract.

Executable declarations have an inactive staging phase. Capability Pack install
may persist validated MCP, hook, and LSP files under service-owned data, but it
does not merge them into live configuration or start/register anything. Preview
marks them as requiring activation. The activation service requires both trusted
canonical workspace provenance and explicit HITL approval before using the
existing MCP, hook, or LSP owner lifecycle.

The persisted approval is desired activation state rather than a claim that an
owner is running. Live application and startup reconciliation revalidate the
current pack transaction, resource ownership, and workspace trust. Invalid or
stale grants fail closed. MCP declarations are connected through the existing
manager, cannot take over a user-configured server name, and publish tools and
instructions only after connection succeeds. Update, rollback, removal, and
explicit revocation remove the owner-provided runtime surface and stop the
pack-owned connection. Hook declarations are deterministic overlays over the
latest user hook base, so hot reload and revocation preserve user handlers and
injected prompt/agent runners. LSP declarations extend an already enabled
manager, never take priority over matching user servers, and close pack-owned
clients on revocation. Staging alone never mutates any live configuration.

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
`y-service` builds a bounded, request-local `CapabilitySearchIndex` from only
the capabilities visible to the current service path. Exact IDs, qualified
names, and bare names are resolved before lexical ranking. Non-exact queries
use the reusable `y-knowledge` BM25 primitive with identifier-aware
tokenization across snake case, kebab case, camel case, and Pascal case.
Documents include compact source-specific fields such as parameter names,
tags, capabilities, trigger hints, categories, and workflow inputs. Scores are
normalized into a shared deterministic scale and ties are resolved by asset
type, name, and ID. Search can return and activate visible tools, but it never
executes a capability or bypasses its later guardrail and permission checks.

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

Interactive `Task` delegation has a separate workspace boundary. After the
agent allowlist is resolved, `y-service` treats effective filesystem mutation
or shell capability as write-capable and strengthens the delegation to a Git
worktree even when the caller preferred `shared`. `y-runtime` creates a
detached worktree at the captured base revision, preserves the delegated
subdirectory, exports changed files and a bounded binary patch, persists a
durable snapshot, and removes the active worktree. A later Task call may name
the snapshot to rehydrate it into a new worktree. Snapshot creation or resume
failure is explicit; no path silently falls back to the parent workspace for a
write-capable delegation. Fresh isolation also rejects a dirty parent repository
instead of silently omitting its uncommitted state and executing from `HEAD`.
The child still inherits the parent session's tool,
permission, operation-mode, activation, HITL, progress, and cancellation
boundaries. Neither completion nor resume applies or merges changes into the
parent workspace automatically.

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
