# Development

The development documentation is intentionally small. Detailed point-in-time
designs and implementation plans become stale quickly, so the repository keeps
only maintained architecture, observability, and contribution guidance.

| Document | Purpose |
| --- | --- |
| [Harness Architecture](./architecture) | System boundaries, execution modes, self-orchestration, evolution, knowledge, and recovery |
| [Observability](./observability) | Local diagnostics, Langfuse export, privacy, and the current OTel boundary |
| [Contributing](./contributing) | TDD workflow, quality gates, and repository rules |

The canonical contributor architecture is also available at
`docs/guides/ARCHITECTURE.md` in the repository. Code and tests are the source of
truth for implementation detail.
