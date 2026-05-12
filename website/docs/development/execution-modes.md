# Execution Modes

y-agent provides four execution modes that control how the system approaches a user's request. The GUI input area cycles through these modes: **Fast**, **Auto**, **Plan**, and **Loop**.

## Mode Overview

| Aspect | Fast | Plan | Loop |
|--------|------|------|------|
| Strategy | Direct single-turn | Decompose, then parallel DAG | Iterative convergence |
| Agent spawns | 0 (main agent only) | 3+ (writer, decomposer, N executors) | 2--25 (N rounds + review) |
| Task visibility | Upfront | All steps known before execution | Steps emerge during execution |
| Parallelism | None | Wave-based DAG (up to 4 concurrent) | Sequential rounds |
| Inter-step memory | Context window | Plan file + session transcripts | Progress file (YAML + markdown) |
| Feedback loop | None | None | Each round reads prior output |
| Self-review | None | None | Mandatory before convergence |
| Best for | Simple questions, single-file fixes | Multi-file refactors, known architecture | Research, quality refinement, unknown scope |

**Auto mode** uses a lightweight classifier agent to route requests to Fast, Plan, or Loop automatically.

---

## Mode Routing

When the user selects **Auto**, a classifier sub-agent (`complexity-classifier`) analyzes the request and returns one of three labels:

```mermaid
flowchart TD
    U["User message"] --> C{"complexity-classifier<br/>(gpt-4o, T=0.0)"}
    C -->|"fast"| F["Fast mode<br/>Direct execution"]
    C -->|"plan"| P["Plan mode<br/>Inject Plan tool"]
    C -->|"loop"| L["Loop mode<br/>Inject Loop tool"]

    style F fill:#334155,stroke:#94a3b8,color:#e2e8f0
    style P fill:#1e3a5f,stroke:#60a5fa,color:#e2e8f0
    style L fill:#3b1f6e,stroke:#a78bfa,color:#e2e8f0
```

Classification rules:
- **plan** -- multi-file changes, architectural design, multi-step coordination where all steps are known upfront
- **loop** -- iterative refinement, research with unknown scope, quality through successive passes, exploration where the full set of steps cannot be determined in advance
- **fast** -- single-file fix, formatting, direct question, simple tweak

The classifier runs with `max_completion_tokens = 5` and `temperature = 0.0` for deterministic, low-latency routing.

---

## Plan Mode

Plan mode implements a three-stage pipeline: **plan** the work, **decompose** into a task DAG, then **execute** phases in dependency order with bounded parallelism.

### Sequence Diagram

```mermaid
sequenceDiagram
    participant U as User
    participant CS as ChatService
    participant MA as Main Agent
    participant PO as PlanOrchestrator
    participant PW as plan-writer
    participant TD as task-decomposer
    participant PE as plan-phase-executor

    U->>CS: Send message (plan mode)
    CS->>MA: execute turn (Plan tool injected)
    MA->>PO: Plan({request, context})

    rect rgb(30, 58, 95)
    Note over PO,PW: Stage 1: Plan Writing
    PO->>PW: SubAgent session<br/>tools: FileRead, Glob, Grep
    PW->>PW: Explore codebase (read-only)
    PW->>PW: Write plan as markdown
    PW-->>PO: Plan content
    PO->>PO: Persist plan to disk
    end

    rect rgb(30, 58, 95)
    Note over PO,TD: Stage 2: Task Decomposition
    PO->>TD: SubAgent session<br/>tools: none (JSON response only)
    TD->>TD: Decompose plan into<br/>structured tasks with<br/>phases and dependencies
    TD-->>PO: StructuredPlan JSON
    PO->>PO: Validate DAG, repair JSON
    end

    rect rgb(30, 58, 95)
    Note over PO,PE: Stage 3: Parallel Execution
    loop Each wave (dependency order)
        par Bounded parallelism (max 4)
            PO->>PE: Phase A (SubAgent)
            PO->>PE: Phase B (SubAgent)
            PO->>PE: Phase C (SubAgent)
        end
        PE-->>PO: Phase results
        PO->>PO: Update task statuses
    end
    end

    PO-->>MA: ToolOutput (completed/failed phases)
    MA-->>CS: Final response
    CS-->>U: Display result
```

### Stage Details

**Stage 1 -- Plan Writer** (`plan-writer` agent)
- Spawns a read-only sub-agent with `FileRead`, `Glob`, `Grep`
- Explores the codebase and produces a markdown plan
- Output persisted to `data/plan/<slug>.md`

**Stage 2 -- Task Decomposer** (`task-decomposer` agent)
- Zero-tool sub-agent with JSON schema enforcement
- Decomposes the plan into structured tasks with `id`, `phase`, `title`, `description`, `depends_on[]`, `key_files[]`, `acceptance_criteria[]`
- Includes automatic JSON repair for common LLM formatting issues

**Stage 3 -- Phase Execution** (N `plan-phase-executor` agents)
- Builds a DAG from task dependencies
- Executes in waves: each wave contains tasks whose dependencies are all complete
- Up to 4 tasks run concurrently within a wave (`futures::join_all`)
- Failed tasks cause all downstream dependents to be skipped
- Falls back to sequential execution if DAG validation fails (cycles, missing deps)

### DAG Execution Model

```mermaid
graph LR
    subgraph Wave 1
        A["Task A<br/>(no deps)"]
        B["Task B<br/>(no deps)"]
    end
    subgraph Wave 2
        C["Task C<br/>(depends: A)"]
        D["Task D<br/>(depends: A, B)"]
    end
    subgraph Wave 3
        E["Task E<br/>(depends: C, D)"]
    end

    A --> C
    A --> D
    B --> D
    C --> E
    D --> E

    style A fill:#166534,stroke:#4ade80,color:#e2e8f0
    style B fill:#166534,stroke:#4ade80,color:#e2e8f0
    style C fill:#1e3a5f,stroke:#60a5fa,color:#e2e8f0
    style D fill:#1e3a5f,stroke:#60a5fa,color:#e2e8f0
    style E fill:#334155,stroke:#94a3b8,color:#e2e8f0
```

Tasks A and B execute in parallel (Wave 1). Once both complete, C and D execute in parallel (Wave 2). Finally E executes (Wave 3).

---

## Loop Mode

Loop mode implements an **iterative convergence** pattern: spawn fresh agent rounds, each reading a persistent progress file, working on remaining tasks, updating progress, and checking for convergence. A mandatory self-review round verifies completion before stopping.

### Sequence Diagram

```mermaid
sequenceDiagram
    participant U as User
    participant CS as ChatService
    participant MA as Main Agent
    participant LO as LoopOrchestrator
    participant LE as loop-executor
    participant PF as PROGRESS.md

    U->>CS: Send message (loop mode)
    CS->>MA: execute turn (Loop tool injected)
    MA->>LO: Loop({request, max_rounds})
    LO->>PF: Initialize progress file<br/>(YAML front matter + sections)

    rect rgb(59, 31, 110)
    Note over LO,PF: Round Loop (1..max_rounds)
    loop Each round (fresh context)
        LO->>PF: Read current state
        LO->>LE: SubAgent session<br/>(full toolset)
        LE->>PF: Read progress
        LE->>LE: Work on remaining tasks<br/>(FileRead, FileWrite,<br/>ShellExec, etc.)
        LE->>PF: Update progress file<br/>(tasks, insights, round log)
        LE-->>LO: Round complete
        LO->>PF: Parse front matter
        LO->>LO: Build RoundSummary

        alt converged == true
            Note over LO: Proceed to self-review
        else converged == false
            Note over LO: Continue to next round
        end
    end
    end

    rect rgb(80, 40, 20)
    Note over LO,PF: Self-Review Round
    LO->>LE: Review SubAgent session
    LE->>PF: Critically review all [DONE] tasks
    alt Issues found
        LE->>PF: Set converged: false<br/>Add new [TODO] tasks
        LE-->>LO: Review failed
        Note over LO: Resume round loop
    else All verified
        LE->>PF: Keep converged: true
        LE-->>LO: Review passed
    end
    end

    LO-->>MA: ToolOutput (converged / budget_exhausted)
    MA-->>CS: Final response
    CS-->>U: Display result
```

### Round Lifecycle

Each round follows a strict 5-step protocol:

1. **READ** -- The executor receives the full progress file content as its input message. It understands the Original Request, current task states, and insights from prior rounds.

2. **DECOMPOSE** (round 1 only) -- If `status: initial`, the executor breaks the request into concrete `[TODO]` tasks and sets `status: running`.

3. **WORK** -- The executor picks the highest-priority remaining tasks and produces concrete outputs using its full toolset (`FileRead`, `FileWrite`, `ShellExec`, `WebFetch`, `Browser`, `Glob`, `Grep`).

4. **UPDATE** -- The executor writes the updated progress file: moves completed tasks to `[DONE]`, updates `[IN PROGRESS]`, adds insights, appends a round log entry, increments `total_rounds`.

5. **CONVERGENCE** -- When all tasks are `[DONE]`, the executor performs a strict self-check. Only if truly satisfied does it set `converged: true` in the front matter.

### Progress File Format

The progress file (`data/loop/<slug>/PROGRESS.md`) is the sole inter-round memory. Each round's sub-agent starts with a fresh context window -- the progress file is how information persists.

**YAML front matter:**

```yaml
---
title: "Task description"
status: initial | running | converged | budget_exhausted
total_rounds: 0
max_rounds: 10
converged: false
created_at: "2026-05-11T12:00:00Z"
updated_at: "2026-05-11T12:00:00Z"
---
```

**Document sections:**

```markdown
## Original Request
(immutable -- never modified by the executor)

## Tasks
### [DONE] Task name
- Summary of work done, key decisions, files changed

### [IN PROGRESS] Task name
- Current status notes

### [TODO] Task name

## Insights
(concise lessons learned, useful for future rounds)

## Round Log
### Round 1
- Completed: ...
- Remaining: ...
```

The orchestrator parses task headings (`### [DONE]`, `### [IN PROGRESS]`, `### [TODO]`) to track progress. The `converged` front matter field is the sole termination signal.

### Self-Review Mechanism

When the executor sets `converged: true`, the orchestrator spawns one additional sub-agent round with a skeptical review prompt:

- Verify each `[DONE]` task is truly complete and correct
- Check the original request is fully satisfied
- If issues are found: revert `converged` to `false`, add new `[TODO]` tasks
- If everything passes: keep `converged: true`

This prevents premature convergence from optimistic agents declaring tasks "done" without thorough verification.

### Convergence State Machine

```mermaid
stateDiagram-v2
    [*] --> Initial: Initialize PROGRESS.md
    Initial --> Running: Round 1 decomposes tasks
    Running --> Running: Round N works on tasks
    Running --> ReviewPending: Executor sets converged=true
    ReviewPending --> SelfReview: Spawn review round
    SelfReview --> Converged: Review passes
    SelfReview --> Running: Review finds issues
    Running --> BudgetExhausted: max_rounds reached

    Converged --> [*]: Return success
    BudgetExhausted --> [*]: Return partial
```

---

## Comparative View

The following diagram shows how the same complex task flows through Plan mode versus Loop mode:

```mermaid
graph TB
    subgraph plan["Plan Mode (optimistic, parallel)"]
        direction TB
        P1["1. Plan Writer<br/>explores codebase,<br/>produces markdown plan"] --> P2["2. Task Decomposer<br/>structures into DAG<br/>with dependencies"]
        P2 --> P3["3. Phase Execution<br/>parallel waves,<br/>bounded concurrency"]
        P3 --> P4["Result:<br/>all phases done or failed"]
    end

    subgraph loop["Loop Mode (pessimistic, iterative)"]
        direction TB
        L1["1. Round 1<br/>decompose request,<br/>start working"] --> L2["2. Round 2..N<br/>continue from<br/>progress file"]
        L2 --> L3{"Converged?"}
        L3 -->|no| L2
        L3 -->|yes| L4["3. Self-Review<br/>skeptical verification"]
        L4 -->|pass| L5["Result:<br/>converged"]
        L4 -->|fail| L2
    end

    style plan fill:#1e3a5f,stroke:#60a5fa,color:#e2e8f0
    style loop fill:#3b1f6e,stroke:#a78bfa,color:#e2e8f0
```

**Plan mode** is optimistic: it assumes the full scope is knowable upfront, decomposes everything before executing, and exploits parallelism. Best when the task structure is clear.

**Loop mode** is pessimistic: it assumes scope will emerge during execution, each round discovers new work, and quality improves through iteration. Best when the end state is uncertain.

---

## Decision Guide

Choose **Plan mode** when:
- The task involves multiple files with clear, known dependencies
- All steps can be enumerated upfront
- Parallelism provides a meaningful speedup
- Examples: multi-file refactor, adding a new feature across layers, migration with known schema changes

Choose **Loop mode** when:
- The full scope is not clear upfront
- Quality benefits from multiple passes
- The task is exploratory or research-oriented
- Each iteration may reveal new requirements
- Examples: thorough code review, iterative research, complex debugging, quality-focused document writing

Choose **Fast mode** when:
- The task is a single-file fix, direct question, or simple tweak
- No orchestration overhead is warranted

Choose **Auto mode** to let the classifier decide. The classifier runs in under 1 second with near-zero token cost (5 max completion tokens).

---

## Implementation Reference

| Component | File | Purpose |
|-----------|------|---------|
| Plan signal tool | `crates/y-tools/src/builtin/plan.rs` | Validates Plan tool input |
| Plan orchestrator | `crates/y-service/src/plan_orchestrator.rs` | 3-stage pipeline execution |
| Plan writer agent | `config/agents/plan-writer.toml` | Read-only codebase exploration |
| Task decomposer agent | `config/agents/plan-task-decomposer.toml` | Structured JSON task output |
| Plan phase executor agent | `config/agents/plan-phase-executor.toml` | Per-phase implementation |
| Loop signal tool | `crates/y-tools/src/builtin/loop_tool.rs` | Validates Loop tool input |
| Loop orchestrator | `crates/y-service/src/loop_orchestrator.rs` | Round loop + convergence |
| Loop executor agent | `config/agents/loop-executor.toml` | Per-round execution |
| Complexity classifier | `config/agents/complexity-classifier.toml` | Auto mode 3-way routing |
| Mode routing | `crates/y-service/src/chat.rs` | Config flag management |
| Tool dispatch intercept | `crates/y-service/src/agent_service/tool_dispatch.rs` | Plan/Loop tool interception |
