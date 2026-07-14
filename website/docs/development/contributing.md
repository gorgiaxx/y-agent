# Contributing

The repository engineering protocol is defined in `AGENTS.md`. This page is a
concise public summary; use the repository file when the two differ.

## Setup

```bash
cargo build --workspace

cd crates/y-gui
npm install
```

Rust 1.94 is pinned in `rust-toolchain.toml`. Node.js is required only for the
shared React/Tauri frontend.

## Architecture Rules

1. Dependencies point inward toward `y-core`.
2. `y-service` owns business logic and cross-capability orchestration.
3. CLI, Web, Tauri, and React surfaces remain thin I/O and rendering layers.
4. New subsystems are feature-gated.
5. Reusable behavior belongs in the owning capability, middleware, or
   orchestration crate.
6. Current code and tests are the implementation source of truth.

Read `docs/guides/ARCHITECTURE.md` and every applicable standard before changing
cross-crate behavior.

## Test-Driven Development

y-agent follows Red, Green, Refactor:

1. add or update a test that demonstrates the required behavior;
2. confirm it fails for the expected reason;
3. implement the smallest correct change;
4. refactor while tests remain green.

Frontend behavior, state, rendering contracts, and interaction flows require
Vitest coverage before implementation changes.

## Rust Quality Gates

Run these commands in order after Rust changes:

```bash
cargo fmt --all
cargo clippy --fix --allow-dirty --workspace -- -D warnings
cargo clippy --workspace -- -D warnings
cargo check --workspace
cargo doc --workspace --no-deps
```

When running tests, use the repository's filtered output form:

```bash
cargo test --workspace 2>&1 | grep -v '^\s*Compiling\|^\s*Running\|^\s*Downloading\|^\s*Downloaded\|^\s*Blocking\|^\s*Finished\|^\s*Doc-tests\|^running\|^test \|^$' | head -200
```

## Frontend Quality Gates

Run from `crates/y-gui`:

```bash
npm test
npm run lint
npm run build
npm run build:web
```

The Web build is required when shared desktop/browser behavior, transport
contracts, platform capabilities, or static serving change.

## R&D and Documentation

Before R&D work, create a plan under `.claude/plans` with scope, steps,
dependencies, and verification criteria.

Do not create a permanent design draft for each change. Update the canonical
architecture or the owning standard when a durable contract changes. Temporary
plans, research notes, generated documentation, provider API copies, and
point-in-time reviews are not maintained as repository documentation.

## Key References

| Document | Purpose |
| --- | --- |
| `AGENTS.md` | Complete engineering protocol |
| `docs/guides/ARCHITECTURE.md` | Canonical harness architecture and maturity boundaries |
| `docs/standards/TEST_STRATEGY.md` | TDD and test strategy |
| `docs/standards/ENGINEERING_STANDARDS.md` | Rust engineering standards |
| `docs/standards/AGENT_AUTONOMY.md` | Delegation and autonomy rules |
| `docs/standards/DSL_STANDARD.md` | Workflow DSL contract |
| `docs/standards/SKILLS_STANDARD.md` | Skill format and evolution rules |
| `docs/standards/TOOL_CALL_PROTOCOL.md` | Tool-call protocol |
| `docs/standards/FRONTEND_REUSE_STANDARD.md` | Shared desktop/Web frontend contract |

## Commit Discipline

- Keep one concern per change.
- Use English documentation, comments, and commit messages.
- Never commit credentials or runtime data.
- Do not add inline lint suppressions by default.
- Do not use emoji in repository content.
