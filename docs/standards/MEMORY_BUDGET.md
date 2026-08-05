# Memory Budget

Status: active guardrail
Updated: 2026-08-05

This document records the explicit ceilings that bound y-agent's in-memory
growth. Every row is verified against its source declaration by
`cargo run -p y-xtask -- guard memory`, so the document cannot drift from the
code: changing a constant fails CI until this file is updated in the same change.

The goal is not to freeze memory usage. The goal is to make memory changes
measurable, reviewable, and intentionally justified.

## Budget model

**Hard caps** are constants already enforced in the code. A change to any of
them is a deliberate product decision and must be justified in the pull request.

**Ratchet expectations** are structures with no constant ceiling. They are listed
so that their unboundedness is a recorded, reviewed decision rather than an
oversight.

## Hard caps

Each row is `constant`, its declaring source file, and the ceiling. The guard
parses these three cells; the rationale column is prose.

| Constant | Source | Ceiling | Why |
| --- | --- | --- | --- |
| `MAX_TOASTS` | `crates/y-cli/src/tui/state.rs` | `5` | Concurrent toasts; beyond this the newest notifications are unreadable |
| `OSC52_MAX_INPUT_BYTES` | `crates/y-cli/src/tui/clipboard.rs` | `100_000` | OSC52 payloads are held in full before transmission; terminals reject larger ones anyway |
| `MAX_IMAGE_BYTES` | `crates/y-cli/src/tui/clipboard.rs` | `20 * 1024 * 1024` | A pasted image is base64-expanded in memory; 20 MiB decodes to roughly 27 MiB of transport buffer |
| `PREVIEW_MAX_LINES` | `crates/y-cli/src/tui/overlays/tasks_picker.rs` | `10` | Inline task preview retains only what fits the pane |
| `MAX_ATTACHMENT_BYTES` | `crates/y-provider/src/attachment.rs` | `20 * 1024 * 1024` | Matches the TUI paste ceiling so the two paths cannot disagree |
| `TOOL_RESULT_MAX_CHARS` | `crates/y-context/src/compaction.rs` | `2000` | Tool results are truncated head+tail during compaction; unbounded results dominate the transcript |
| `DEFAULT_HISTORY_BUDGET` | `crates/y-context/src/load_history.rs` | `80_000` | Token ceiling for replayed history; the dominant contributor to per-turn payload size |
| `DEFAULT_BOOTSTRAP_BUDGET` | `crates/y-context/src/inject_bootstrap.rs` | `8_000` | Token ceiling for bootstrap files injected every turn |
| `DEFAULT_MEMORY_BUDGET` | `crates/y-context/src/inject_memory.rs` | `4_000` | Token ceiling for recalled memories |
| `DEFAULT_KNOWLEDGE_BUDGET` | `crates/y-knowledge/src/middleware.rs` | `4_000` | Token ceiling for retrieved knowledge |
| `MAX_TOKENS_PER_SKILL` | `crates/y-context/src/inject_skills.rs` | `2_000` | Enforces the skill-size limit in `AGENTS.md` 2.4 |
| `DEFAULT_SUFFIX_TOKEN_LIMIT` | `crates/y-context/src/pruning/superseded.rs` | `8_000` | Suffix kept intact during cache-aware pruning |
| `MAX_DELETIONS_PER_PASS` | `crates/y-context/src/pruning/intra_turn.rs` | `5` | Bounds how much context a single pruning pass may remove |
| `MAX_TAGS` | `crates/y-knowledge/src/tagger.rs` | `15` | Tags retained per document after merging |

## Ratchet expectations

These structures have no constant ceiling. Each entry states why, and what would
force it to become a hard cap.

### `AppState.messages` (`crates/y-cli/src/tui/state.rs`)

The TUI conversation transcript. Grows with every turn and is cleared only by
`/clear` and `/new`.

Deliberately unbounded: truncating it would silently delete scrollback the user
can still see, which is worse than the memory it costs. Each entry is a rendered
`ChatMessage`, and the individual contributors are already capped
(`TOOL_RESULT_MAX_CHARS`, `MAX_IMAGE_BYTES`), so growth is linear in turn count
rather than in content size.

Becomes a hard cap if a measured session shows transcript retention dominating
process RSS. The correct fix at that point is off-screen eviction backed by the
durable transcript store, not blind truncation.

### `ServiceContainer.session_operation_modes` (`crates/y-service/src/container.rs`)

Per-session operation mode, keyed by session id. Entries are removed by
`cleanup_session_state`; the ratchet expectation is that every session-scoped map
in the container is reachable from that cleanup path. Adding a new session-keyed
map without wiring it into `cleanup_session_state` is a leak.

## Review checklist

A change that touches any row above must:

1. Update the ceiling in this document in the same commit.
2. State in the pull request why the new ceiling is correct.
3. Update or add the test that covers the affected behavior.

A change that introduces a new long-lived collection must either give it a
constant ceiling and a row in **Hard caps**, or record it under **Ratchet
expectations** with the condition that would promote it to a hard cap.
