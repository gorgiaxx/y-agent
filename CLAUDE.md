# CLAUDE.md -- y-agent Engineering Protocol

This file defines the default working protocol for Claude agents in this repository.
Scope: entire repository.

## 1) Project Snapshot (Read First)

y-agent is a Rust-first, modular AI Agent framework designed for personal research with extreme demands on architecture quality. Current phase: **design** (pre-implementation). Primary artifacts are design documents under `docs/design/`.

Core design goals:

- High performance (async-first Rust, P95 tool dispatch < 100ms)
- Model-agnostic (works across capability tiers from Qwen/DeepSeek to Claude/GPT-4)
- Full observability (span-based tracing, structured metrics, replay)
- Complete recoverability (WAL-based persistence, task-level checkpoints)
- Self-evolution (skills learn from execution experience)

Key architectural modules (as Rust crates):

- `y-core` -- core abstractions and traits
- `y-provider` -- Provider Pool: multi-provider LLM management, freeze/thaw, tag-based routing
- `y-agent` -- Orchestrator: DAG execution, typed channels, interrupt/resume, expression DSL
- `y-session` -- Session tree, branching, canonical cross-channel sessions
- `y-context` -- Context assembly pipeline, compaction, memory recall (RAG)
- `y-hooks` -- Hook system, middleware chains (LLM/Tool/Context/Compaction/Memory), event bus, plugin loading
- `y-tools` -- Tool registry, 3 tool types (built-in, MCP, custom), parameter validation, execution pipeline
- `y-runtime` -- Docker/Native/SSH runtime adapters, capability-based permissions, image whitelist
- `y-skills` -- Skill ingestion pipeline, LLM-assisted transformation, tree-indexed proprietary format, versioning, self-evolution
- `y-multi-agent` -- Agent definitions, 4 collaboration patterns, delegation protocol, agent pool
- `y-guardrails` -- Guardrails, LoopGuard, taint tracking, unified permission model, HITL escalation
- `y-mcp` -- MCP protocol support
- `y-storage` -- Persistence layer
- `y-scheduler` -- Scheduled tasks
- `y-cli` -- CLI interface

## 2) Engineering Principles (Normative)

These principles are mandatory. They apply to both design documents and implementation.

### 2.1 Architectural Stability Over Feature Velocity

- Prefer extending via traits, configuration, and middleware over modifying core modules.
- New capabilities should be addable through the Hook/Middleware/Plugin system without core code changes.
- Feature flags gate all non-trivial new subsystems for independent rollback.

### 2.2 Separation of Concerns at Module Boundaries

- Tools handle business logic; Runtime handles isolation. No security enforcement in tool code.
- Skills contain LLM reasoning instructions only; tools/scripts belong in the Tool Registry.
- Guardrails operate as middleware in y-hooks chains; no parallel permission systems.
- Memory has three tiers with disjoint scopes: Long-Term (persistent), Short-Term (session), Working Memory (pipeline).

### 2.3 Explicit Over Implicit

- State assumptions explicitly in every design doc.
- Document rejected alternatives briefly (not just what was chosen, but what was not and why).
- For open questions, include owner and due date.
- Provide measurable success criteria where possible.

### 2.4 Token Efficiency as a First-Class Constraint

- Skill root documents must stay under 2,000 tokens; sub-documents loaded on demand.
- Micro-Agent Pipeline steps target < 2,000 tokens context each.
- Context Window Guard enforces category-based token budgets.
- Working Memory slots carry `token_estimate` fields for budget awareness.

### 2.5 Defense in Depth for Safety

- Runtime provides OS-level isolation (Docker containers, capability checks, image whitelist).
- Guardrails provide application-level safety (pre/post validators, LoopGuard, taint tracking).
- Permission Model provides user-level control (allow/notify/ask/deny per tool).
- No single layer being disabled should make the system unsafe.

### 2.6 Fail Fast, Recover Cheap

- Task-level checkpointing with committed/pending state separation.
- Micro-Agent Pipeline retries from the failed step, not from scratch.
- Provider freeze mechanism disables failed providers with adaptive thaw.
- Compensation tasks for side-effect-bearing operations.

### 2.7 Test-Driven Development (TDD)

- All production code must follow the **Red-Green-Refactor** cycle.
- No production code is written without a corresponding test that **preceded** it.
- Trait contracts in `y-core` are specified via test cases before any concrete implementation.
- See `docs/standards/TEST_STRATEGY.md` for the full TDD methodology, pyramid, and quality gates.

## 3) Repository Map (High-Level)

```
y-agent/
  VISION.md                    -- Project vision and philosophy (Chinese)
  DESIGN_RULE.md               -- Design document standards, playbooks, and validation checklist
  DESIGN_OVERVIEW.md           -- Authoritative design index and cross-cutting alignment
  CLAUDE.md                    -- This file: agent engineering protocol
  docs/design/                 -- 24 detailed design documents
  docs/standards/              -- Engineering standards, test strategy, database schema
  docs/research/               -- Research and analysis (not design docs)
  docs/plan/                   -- Project plan and per-module plans
  config/                      -- Configuration schemas (planned)
  crates/                      -- Rust workspace crates
  migrations/                  -- Database migration files
```

## 4) Risk Tiers (Review Depth Contract)

- **Low risk**: Typo fixes, open question additions
- **Medium risk**: New design doc sections, performance targets, rollout phases, alternatives
- **High risk**: Changes to shared concepts (permission model, memory tiers, collaboration patterns, middleware chains), modifications to `DESIGN_OVERVIEW.md` Cross-Cutting Alignment table, any change that affects multiple design docs simultaneously

When uncertain, classify as higher risk.

## 5) Agent Workflow (Required)

### 5.1 Design Document Changes

> **Full workflow in `DESIGN_RULE.md` (Sections 7-11).**

Summary:

1. Read target doc, `DESIGN_OVERVIEW.md`, and `DESIGN_RULE.md` before any edit.
2. Check the Cross-Cutting Design Alignment table for relevant authoritative decisions.
3. If modifying shared concepts, identify all affected docs and update them in the same change.
4. Follow `DESIGN_RULE.md` Section 4 (13 required sections), Section 2 (diagram policy), Section 3 (abstraction level).
5. Update `DESIGN_OVERVIEW.md` as needed (Component Overview, Cross-Cutting Alignment).
6. Bump version in every modified doc.
7. Run the validation checklist in `DESIGN_RULE.md` Section 10.

### 5.2 Implementation Changes (TDD Workflow)

> **Full TDD methodology in `docs/standards/TEST_STRATEGY.md`.**
> **Full coding standards in `docs/standards/ENGINEERING_STANDARDS.md`.**

Summary:

1. **Red** — Write a failing test that defines the desired behavior or API.
2. **Green** — Write the minimal production code to make the test pass.
3. **Refactor** — Improve code structure while keeping all tests green.
4. **Repeat** — Move to the next behavior, starting with a new failing test.

Additional implementation rules:

- Use Rust standard casing: modules/files `snake_case`, types/traits/enums `PascalCase`, functions/variables `snake_case`, constants `SCREAMING_SNAKE_CASE`.
- Extension points are traits in `y-core`; new capabilities added by implementing traits and registering in the appropriate registry.
- Keep dependency direction inward: concrete crates depend on `y-core` trait definitions, not on each other.
- Every subsystem gates behind a feature flag for independent rollback.
- PR reviews must verify that tests were committed **before** or **alongside** the production code they cover (not after).

### 5.3 Commit and Change Discipline

- Keep changes scoped: one concern per change.
- For cross-document alignment changes, make all affected docs consistent in one batch.
- Commit messages in English; clear and descriptive.
- Never commit secrets, API keys, or personal data.

## 6) Key Reference Documents

| Document | Purpose |
|----------|---------|
| `VISION.md` | Project vision, philosophy, and use cases |
| `DESIGN_RULE.md` | Design document standards, change playbooks, validation checklist, anti-patterns |
| `DESIGN_OVERVIEW.md` | Authoritative design index, cross-cutting alignment, module structure |
| `docs/standards/TEST_STRATEGY.md` | Test pyramid, TDD methodology, mock strategy, coverage targets, quality gates |
| `docs/standards/ENGINEERING_STANDARDS.md` | Rust coding standards, error handling, async patterns, logging, dependencies |
| `docs/standards/DATABASE_SCHEMA.md` | Database schema design for SQLite, PostgreSQL, Qdrant |
| `docs/plan/PROJECT_PLAN.md` | Project implementation plan and phases |
| `docs/plan/R&D_PLAN.md` | R&D plan and per-module implementation details |