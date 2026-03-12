# CLAUDE.md — y-agent Engineering Protocol

Scope: entire repository. All rules are mandatory.

## 1) Project Snapshot

**y-agent** — Rust-first modular AI Agent framework. Phase: **active implementation**. Design docs: `docs/design/`.

Goals: async-first (P95 tool dispatch < 100ms) · model-agnostic · full observability · WAL-based recoverability · self-evolving skills.

Crates: `y-core` · `y-provider` · `y-agent` · `y-session` · `y-context` · `y-hooks` · `y-tools` · `y-runtime` · `y-skills` · `y-guardrails` · `y-mcp` · `y-storage` · `y-scheduler` · `y-cli`

## 2) Engineering Principles

| # | Principle | Key rules |
|---|-----------|-----------|
| 2.1 | **Architectural Stability** | Extend via traits/middleware/plugins; feature-flag every new subsystem. |
| 2.2 | **Separation of Concerns** | Tools = business logic; Runtime = isolation; Guardrails = middleware only; Memory = 3 disjoint tiers. |
| 2.3 | **Explicit Over Implicit** | State assumptions; document rejected alternatives; measurable success criteria. |
| 2.4 | **Token Efficiency** | Skill root docs ≤ 2 000 tokens; MAP steps ≤ 2 000 tokens; Working Memory carries `token_estimate`. |
| 2.5 | **Defense in Depth** | Runtime (OS) → Guardrails (app) → Permission Model (user); no single layer removal makes system unsafe. |
| 2.6 | **Fail Fast, Recover Cheap** | Checkpoint at task level; retry from failed step; freeze/thaw providers; compensation for side effects. |
| 2.7 | **TDD** | Red → Green → Refactor. No production code without a preceding test. See `TEST_STRATEGY.md`. |
| 2.8 | **English** | All docs, comments, commits in English. `VISION.md` is the sole exception. |
| 2.9 | **No Emoji** | No emoji in any artifact: code, docs, comments, commits, diagrams, or agent output. Use plain text only. |

## 3) Repository Map

```
y-agent/
  VISION.md / DESIGN_RULE.md / DESIGN_OVERVIEW.md / CLAUDE.md
  docs/design/      — detailed design documents
  docs/standards/   — engineering standards, test strategy, schema
  docs/plan/        — project & per-module R&D plans
  crates/           — Rust workspace
  migrations/       — database migrations
```

## 4) Risk Tiers

- **Low** — typo fixes, open question additions
- **Medium** — new sections, targets, alternatives
- **High** — shared concepts (permission model, memory tiers, middleware chains), `DESIGN_OVERVIEW.md` alignment table, multi-doc changes

When uncertain → High.

## 5) Agent Workflow

### 5.1 Design Document Changes
> Full workflow: `DESIGN_RULE.md` §7-11.

1. Read target doc + `DESIGN_OVERVIEW.md` + `DESIGN_RULE.md` first.
2. Check Cross-Cutting Alignment table; update all affected docs in one batch.
3. Follow `DESIGN_RULE.md` §4 (13 required sections), §2 (diagrams), §3 (abstraction).
4. Update `DESIGN_OVERVIEW.md`; bump version in every modified doc; run §10 checklist.

### 5.2 Implementation (TDD)
> Standards: `TEST_STRATEGY.md` · `ENGINEERING_STANDARDS.md`

- **Before coding**: read the design doc in `docs/design/` + `DESIGN_OVERVIEW.md`. Implementation must conform. Impractical design → update doc first, then code.
- **TDD cycle**: Red (failing test) → Green (minimal code) → Refactor → Repeat.
- Rust casing: `snake_case` files/fns · `PascalCase` types · `SCREAMING_SNAKE_CASE` consts.
- Dependencies point inward to `y-core`; every subsystem behind a feature flag.

### 5.3 Sub-Agent Work
- Read `docs/standards/AGENT_AUTONOMY.md` before designing or implementing any sub-agent component (delegation, agent pools, autonomy).

### 5.4 R&D Planning
- **Before any R&D action**: write a plan to `docs/plan/` covering scope, steps, dependencies, and verification criteria. No implementation until the plan exists.

### 5.5 Commit Discipline
- One concern per change; cross-doc changes in one batch; English commit messages; no secrets.

## 6) Key References

| Document | Purpose |
|----------|---------|
| `DESIGN_RULE.md` | Design doc standards, playbooks, validation checklist |
| `DESIGN_OVERVIEW.md` | Authoritative index, cross-cutting alignment |
| `docs/standards/TEST_STRATEGY.md` | TDD methodology, pyramid, quality gates |
| `docs/standards/ENGINEERING_STANDARDS.md` | Rust coding standards |
| `docs/standards/DATABASE_SCHEMA.md` | SQLite / PostgreSQL / Qdrant schema |
| `docs/standards/AGENT_AUTONOMY.md` | Sub-agent autonomy model & delegation protocol |
| `docs/plan/PROJECT_PLAN.md` | Implementation phases |
| `docs/plan/R&D_PLAN.md` | Per-module R&D details |
| `VISION.md` | Project vision (Chinese) |

## 7) Formatting Constraints

- **No emoji anywhere.** All repository content -- source code, documentation, comments, commit messages, Mermaid diagrams, log output, and AI-generated responses -- must be free of emoji characters. Use plain-text markers, ASCII symbols, or descriptive words instead.