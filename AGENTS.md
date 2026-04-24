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
**Presentation**: `y-cli` (CLI + TUI) · `y-web` (REST API) · `y-gui/src-tauri` (Tauri shell)
**Frontend Package (non-Cargo)**: `crates/y-gui` (React + Vite + TypeScript desktop/web UI)
**Testing**: `y-test-utils`

### 1.2 Repository Layout

```
y-agent/
  docs/
    design/            — detailed design documents
    standards/         — engineering, testing, DB, DSL, skills, and tool-call standards
  config/
    agents/            — agent configuration
    persona/           — persona configuration
    prompts/           — prompt configuration
  crates/              — Rust workspace crates plus the `y-gui` frontend package directory
    y-gui/
      src/             — React/Vite frontend source
      src-tauri/       — Tauri Rust crate (workspace member)
  data/                — runtime SQLite data
  images/              — app / repository image assets
  scripts/             — automation, release, health-check, and test helper scripts
  skills/              — bundled/local skill content
  tests/               — workspace-level integration tests
  website/             — website / docs frontend
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
- **2.10 No Inline Lint Suppression** -- Never add `#[allow(clippy::...)]`, `#[allow(rustc_lint)]`, `// eslint-disable`, or `// @ts-ignore` to source code by default. Fix the lint, refactor the code, or move the rule adjustment to the owning config with a comment explaining why. The sole Rust exception is `#[allow(dead_code)]` on struct fields/variants kept for API completeness (e.g. deserialized-but-not-read fields). For TypeScript, `// @ts-expect-error` is allowed only when no safer option exists and the reason is documented inline.
- **2.11 Modular & Concise Code** -- Code should be modular and concise, with logic broken into reusable components or functions. Minimize duplication through abstraction and ensure loose coupling by managing dependencies carefully. Avoid unnecessary interdependencies to maintain flexibility and ease of maintenance.

## 3) Risk Tiers

- **Low** — typo fixes, open question additions
- **Medium** — new sections, targets, alternatives
- **High** — shared concepts (permission model, memory tiers, middleware chains), `DESIGN_OVERVIEW.md` alignment table, multi-doc changes

When uncertain -> High.

## 4) Agent Workflow

### 4.1 Implementation (TDD)

> Standards: `TEST_STRATEGY.md` · `ENGINEERING_STANDARDS.md`

- **Before coding**: read the design doc in `docs/design/` + `DESIGN_OVERVIEW.md`. Implementation must conform. Impractical design -> update doc first, then code.
- **TDD cycle**: Red (failing test) -> Green (minimal code) -> Refactor -> Repeat.
- **Frontend TDD**: for `crates/y-gui`, add or update Vitest coverage before changing behavior whenever the work affects UI state, rendering contracts, or interaction flows.
- Rust casing: `snake_case` files/fns · `PascalCase` types · `SCREAMING_SNAKE_CASE` consts.
- Dependencies point inward to `y-core`; every subsystem behind a feature flag.

### 4.2 Sub-Agent Work

- Read `docs/standards/AGENT_AUTONOMY.md` before designing or implementing any sub-agent component (delegation, agent pools, autonomy).

### 4.3 R&D Planning

- **Before any R&D action**: write a plan to `.claude/plans` covering scope, steps, dependencies, and verification criteria. No implementation until the plan exists.

### 4.4 Commit Discipline

- One concern per change; cross-doc changes in one batch; English commit messages; no secrets.

### 4.5 Post-Development Quality Gates

After completing code changes, run every applicable gate for the touched surface **in order** and fix all issues before considering the task done.

#### Rust changes

After completing Rust code changes, run the following checks **in order**:

- **`cargo fmt --all`** — Format all workspace crates according to `rustfmt.toml` (max_width=100, edition=2021). Run this first to establish a clean formatting baseline.
- **`cargo clippy --fix --allow-dirty --workspace -- -D warnings`** — Automatically apply Clippy suggestions. The `--allow-dirty` flag is necessary to allow operations on unstaged changes during active development.
- **`cargo clippy --workspace -- -D warnings`** — All Clippy lints must pass with zero warnings. Treat every warning as an error. Lint policy is defined in `[workspace.lints.clippy]` in `Cargo.toml` and thresholds in `clippy.toml`.
- **`cargo check --workspace`** — Full workspace compilation must succeed with no errors.
- **`cargo doc --workspace --no-deps`** — Documentation must build without errors.

#### Frontend / Tauri GUI changes (`crates/y-gui`)

After completing frontend changes in `crates/y-gui`, run the following checks **from `crates/y-gui/` and in order**:

- **`npm test`** — Full Vitest suite must pass. Do not rely only on focused test runs at completion time.
- **`npm run lint`** — ESLint must pass cleanly with zero errors and zero warnings.
- **`npm run build`** — TypeScript compilation and Vite production build must succeed. Investigate new non-fatal warnings introduced by the change; existing non-blocking bundle-size warnings should still be mentioned if they materially worsen.

#### Mixed-surface changes

- If a change touches both Rust and `crates/y-gui`, run both gate sets.
- If a change touches shared contracts that affect multiple clients, run every gate relevant to each touched client.

No task is complete until every applicable gate passes cleanly.

### 4.6 Rust Test Output Filtering

When running `cargo test`, always pipe output through `grep` to extract error information. Use the following filter:

```bash
cargo test [args] 2>&1 | grep -v '^\s*Compiling\|^\s*Running\|^\s*Downloading\|^\s*Downloaded\|^\s*Blocking\|^\s*Finished\|^\s*Doc-tests\|^running\|^test \|^$' | head -200
```

This strips compilation progress, download noise, and individual test-pass lines, leaving only failures, panics, and diagnostic output.

### 4.7 Frontend Test Discipline

- During TDD in `crates/y-gui`, prefer focused Vitest runs while iterating, for example `npm test -- --run src/__tests__/some_test.ts`.
- Before finishing, always rerun the full frontend suite with `npm test`.
- Shared browser-like test environment requirements (for example `EventSource`, `matchMedia`, or similar globals) belong in shared Vitest setup files, not copy-pasted stubs across many tests.
- When a renderer or UI contract changes intentionally, update the affected tests in the same change. Do not preserve outdated assertions just to keep old snapshots or CSS class names alive.

## 5) Key References

- `DESIGN_RULE.md` -- Design doc standards, playbooks, validation checklist
- `docs/standards/TEST_STRATEGY.md` -- TDD methodology, pyramid, quality gates
- `docs/standards/ENGINEERING_STANDARDS.md` -- Rust coding standards
- `docs/standards/DATABASE_SCHEMA.md` -- SQLite / Qdrant schema
- `docs/standards/AGENT_AUTONOMY.md` -- Sub-agent autonomy model & delegation protocol
- `docs/standards/DSL_STANDARD.md` -- DSL specification
- `docs/standards/SKILLS_STANDARD.md` -- Skills format and authoring standard
- `docs/standards/TOOL_CALL_PROTOCOL.md` -- Tool call protocol specification

## 6) Formatting Constraints

- **No emoji anywhere.** All repository content -- source code, documentation, comments, commit messages, Mermaid diagrams, log output, and AI-generated responses -- must be free of emoji characters. Use plain-text markers, ASCII symbols, or descriptive words instead.
