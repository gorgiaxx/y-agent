# y-agent Engineering Protocol

Scope: entire repository. All rules are mandatory.

## 1) Project Snapshot

**y-agent** — Rust-first modular AI Agent framework. Phase: **active implementation**. Design docs: `docs/design/`.

Goals: async-first (P95 tool dispatch < 100ms) · model-agnostic · full observability · WAL-based recoverability · self-evolving skills.

### 1.1 Workspace Crates

**Core**: `y-core`
**Infrastructure**: `y-provider` · `y-session` · `y-context` · `y-storage` · `y-knowledge` · `y-diagnostics`
**Middleware**: `y-hooks` · `y-guardrails` · `y-prompt` · `y-mcp`
**Capabilities**: `y-tools` · `y-skills` · `y-runtime` · `y-scheduler` · `y-browser` · `y-journal`
**Orchestration**: `y-agent` · `y-bot`
**Service**: `y-service` (all business logic)
**Presentation**: `y-cli` (CLI + TUI) · `y-web` (REST API) · `y-gui` (Tauri desktop app)
**Testing**: `y-test-utils`

### 1.2 Repository Layout

```
y-agent/
  docs/
    design/            — detailed design documents
    standards/         —
    plan/              — project & per-module plans and remediation docs
    guides/            —
    api/               —
    research/          — just for research
    schema/            —
  config/              — TOML config templates (providers, runtime, guardrails, browser, etc.)
  builtin-skills/      — built-in skill packages
  migrations/sqlite/   — SQLite migrations
  scripts/             — build-release.sh, deploy.sh, health-check.sh, native-install.sh
  data/                — SQLite database (runtime data)
  tests/               — workspace-level integration tests
  crates/              — 24 Rust crates (see table above)
```

## 2) Engineering Principles

- **2.1 Architectural Stability** -- Extend via traits/middleware/plugins; feature-flag every new subsystem.
- **2.2 Separation of Concerns** -- `y-service` orchestrates business logic; Presentation layers are thin I/O wrappers; Capabilities provide discrete functions (`y-tools`, `y-skills`); Infrastructure safely abstracts state (`y-storage`, `y-knowledge`).
- **2.3 Explicit Over Implicit** -- State assumptions; document rejected alternatives; measurable success criteria.
- **2.4 Token Efficiency** -- Skill root docs <= 2 000 tokens; MAP steps <= 2 000 tokens; Working Memory carries `token_estimate`.
- **2.5 Defense in Depth** -- Isolation via sandbox (`y-runtime`) -> Interception via middleware (`y-guardrails`) -> User-approved execution (HITL); no single abstraction layer failure makes the system fundamentally unsafe.
- **2.6 Fail Fast, Recover Cheap** -- Checkpoint at task level; retry from failed step; freeze/thaw providers; compensation for side effects.
- **2.7 TDD** -- Red -> Green -> Refactor. No production code without a preceding test. See `TEST_STRATEGY.md`.
- **2.8 English** -- All docs, comments, commits in English. `VISION.md` is the sole exception.
- **2.9 Service-Layer Ownership** -- All business logic lives in `y-service`; `y-cli`, `y-web`, `y-gui` (Tauri) are thin presentation layers -- they handle I/O, rendering, and user interaction only. No domain logic in presentation crates.
- **2.10 No Inline Lint Suppression** -- Never add `#[allow(clippy::...)]` or `#[allow(rustc_lint)]` to source code. Fix the lint or add the allow to `[workspace.lints]` in `Cargo.toml` with a comment explaining why. The sole exception is `#[allow(dead_code)]` on struct fields/variants kept for API completeness (e.g. deserialized-but-not-read fields).
- **2.11 Modular & Concise Code** -- Code should be modular and concise, with logic broken into reusable components or functions. Minimize duplication through abstraction and ensure loose coupling by managing dependencies carefully. Avoid unnecessary interdependencies to maintain flexibility and ease of maintenance.

## 3) Risk Tiers

- **Low** — typo fixes, open question additions
- **Medium** — new sections, targets, alternatives
- **High** — shared concepts (permission model, memory tiers, middleware chains), `DESIGN_OVERVIEW.md` alignment table, multi-doc changes

When uncertain -> High.

## 4) Agent Workflow

### 4.1 Design Document Changes

> Full workflow: `DESIGN_RULE.md` S7-11.

1. Read target doc + `DESIGN_OVERVIEW.md` + `DESIGN_RULE.md` first.
2. Check Cross-Cutting Alignment table; update all affected docs in one batch.
3. Follow `DESIGN_RULE.md` S4 (13 required sections), S2 (diagrams), S3 (abstraction).
4. Update `DESIGN_OVERVIEW.md`; bump version in every modified doc; run S10 checklist.

### 4.2 Implementation (TDD)

> Standards: `TEST_STRATEGY.md` · `ENGINEERING_STANDARDS.md`

- **Before coding**: read the design doc in `docs/design/` + `DESIGN_OVERVIEW.md`. Implementation must conform. Impractical design -> update doc first, then code.
- **TDD cycle**: Red (failing test) -> Green (minimal code) -> Refactor -> Repeat.
- Rust casing: `snake_case` files/fns · `PascalCase` types · `SCREAMING_SNAKE_CASE` consts.
- Dependencies point inward to `y-core`; every subsystem behind a feature flag.

### 4.3 Sub-Agent Work

- Read `docs/standards/AGENT_AUTONOMY.md` before designing or implementing any sub-agent component (delegation, agent pools, autonomy).

### 4.4 R&D Planning

- **Before any R&D action**: write a plan to `docs/plan/` covering scope, steps, dependencies, and verification criteria. No implementation until the plan exists.

### 4.5 Commit Discipline

- One concern per change; cross-doc changes in one batch; English commit messages; no secrets.

### 4.6 Post-Development Quality Gates

After completing rust code change, run the following checks **in order** and fix all issues before considering the task done:

```bash
cargo fmt --all
cargo clippy --fix --allow-dirty --workspace -- -D warnings
cargo clippy --workspace -- -D warnings
cargo check --workspace
cargo doc --workspace --no-deps
```

- **`cargo fmt --all`** — Format all workspace crates according to `rustfmt.toml` (max_width=100, edition=2021). Run this first to establish a clean formatting baseline.
- **`cargo clippy --fix --allow-dirty --workspace -- -D warnings`** — Automatically apply Clippy suggestions. The `--allow-dirty` flag is necessary to allow operations on unstaged changes during active development.
- **`cargo clippy --workspace -- -D warnings`** — All Clippy lints must pass with zero warnings. Treat every warning as an error. Lint policy is defined in `[workspace.lints.clippy]` in `Cargo.toml` and thresholds in `clippy.toml`.
- **`cargo check --workspace`** — Full workspace compilation must succeed with no errors.
- **`cargo doc --workspace --no-deps`** — Documentation must build without errors.

No task is complete until all four commands pass cleanly.

## 5) Key References

- `DESIGN_RULE.md` -- Design doc standards, playbooks, validation checklist
- `docs/standards/TEST_STRATEGY.md` -- TDD methodology, pyramid, quality gates
- `docs/standards/ENGINEERING_STANDARDS.md` -- Rust coding standards
- `docs/standards/DATABASE_SCHEMA.md` -- SQLite / Qdrant schema
- `docs/standards/AGENT_AUTONOMY.md` -- Sub-agent autonomy model & delegation protocol
- `docs/standards/DSL_STANDARD.md` -- DSL specification
- `docs/standards/SKILLS_STANDARD.md` -- Skills format and authoring standard
- `docs/standards/TOOL_CALL_PROTOCOL.md` -- Tool call protocol specification
- `VISION.md` -- Project vision (Chinese)

## 6) Formatting Constraints

- **No emoji anywhere.** All repository content -- source code, documentation, comments, commit messages, Mermaid diagrams, log output, and AI-generated responses -- must be free of emoji characters. Use plain-text markers, ASCII symbols, or descriptive words instead.
