# Capability Pack Standard

**Version**: v0.1
**Status**: Draft
**Architecture Reference**: [y-agent Harness Architecture](../guides/ARCHITECTURE.md)

## 1. Purpose

A Capability Pack is a local, versioned delivery unit that groups existing
y-agent capabilities. It does not own runtime behavior, replace any capability
registry, or introduce a second permission or trust system.

The first implementation is local-only. Remote registries, implicit downloads,
native libraries, arbitrary package code, and marketplace discovery are not
part of this standard.

## 2. Canonical Layout

```text
example-pack/
  capability-pack.toml
  skills/
  agents/
  workflows/
  mcp/
  hooks/
  lsp/
```

Only `capability-pack.toml` is mandatory. Resource paths are explicitly listed
in the manifest; convention-based discovery is forbidden.

## 3. Manifest

```toml
[pack]
schema_version = 1
id = "rust-team"
version = "1.0.0"
description = "Shared Rust engineering capabilities"

[[resources]]
kind = "skill"
id = "rust-review"
path = "skills/rust-review"
sha256 = "0123456789abcdef..."

[[resources]]
kind = "agent"
id = "rust-reviewer"
path = "agents/rust-reviewer.toml"
sha256 = "abcdef0123456789..."
```

Rules:

- Unknown manifest fields are rejected.
- `schema_version` is mandatory; unsupported versions are rejected.
- Pack IDs and resource IDs use lowercase kebab-case and are 3-64 characters.
- Pack versions are semantic versions.
- Resource identity is `(kind, id)` and must be unique within the pack.
- Supported kinds are `skill`, `agent`, `workflow`, `mcp`, `hook`, and `lsp`.
- Skill resources are directories. Other first-version resources are files.
- Every resource declares a lowercase 64-character SHA-256 digest.

Declarative resource contracts are intentionally narrow:

- A `skill` directory uses the existing filesystem skill layout, contains
  `skill.toml`, and its parsed skill name equals the declared resource ID.
- An `agent` resource is UTF-8 TOML accepted by `AgentDefinition`; its embedded
  agent ID equals the declared resource ID.
- A `workflow` resource is UTF-8 source. `.toml` selects the canonical workflow
  TOML format and `.dsl` selects the expression DSL. The declared resource ID
  is the workflow name in the existing workflow store.
- An `mcp` resource is one UTF-8 TOML `McpServerConfig`; its server name equals
  the declared resource ID.
- A `hook` resource is one UTF-8 TOML hook declaration containing a hook point,
  matcher, optional timeout, and handlers accepted by the existing hook config
  validator.
- An `lsp` resource is one UTF-8 TOML `LspServerConfig`; its server ID equals the
  declared resource ID.
- Other extensions and semantic identity mismatches fail preflight validation.

## 4. Staging and Filesystem Safety

Validation canonicalizes the pack root and manifest path before inspecting
resources. Resource paths must be relative, must not contain parent traversal,
and must resolve within the canonical pack root.

Symbolic links are forbidden for both declared resources and every descendant
inside a declared directory. Missing paths, unsupported filesystem entry types,
path escapes, and resource type mismatches fail validation. Validation never
modifies live capability stores.

Directory hashes are deterministic. Files are ordered by normalized relative
path. The digest includes each relative path, file length, and file bytes so
renames and content changes both change the resource hash.

## 5. Provenance

A successful validation report retains:

- canonical pack root;
- canonical manifest path and manifest SHA-256;
- pack ID, pack version, and schema version;
- canonical resource paths and verified resource SHA-256 values;
- local-directory source kind.

This provenance must be carried into later installation, activation, update,
rollback, and removal stages. Presentation layers may display it but may not
reconstruct or override it.

## 6. Ownership and Activation

- `y-service` owns pack validation and later installation transactions.
- Skills, agents, workflows, MCP, hooks, and LSP remain owned by their existing
  registries and services.
- Declarative installation and executable activation are separate phases.
- MCP, hook, and LSP declarations retain their origin and enter the existing
  workspace activation trust/Guardrail and HITL path before becoming active.
- Operation modes and permission modes cannot implicitly approve pack
  activation.

Installing MCP, hook, or LSP resources copies their validated declarations into
the service data directory but does not merge runtime configuration, connect a
server, create a hook executor, spawn a process, or register tools. Preview marks
these resources as requiring activation. This inactive staging participates in
the same ownership, update, rollback, and removal transaction as declarative
resources.

Activation is a later service operation and requires both a trusted canonical
workspace and an explicit activation approval produced through the existing
HITL transport. Workspace trust alone does not approve activation; permission or
operation modes cannot synthesize either prerequisite.

An activation grant is durable desired state, not evidence that a runtime owner
successfully started. Applying a grant must revalidate that the granted pack
version and transaction are still current, that the canonical workspace is
still trusted, and that every executable resource is still owned by that
transaction. A failed owner start leaves the grant available for an explicit or
startup retry, but must not report the resource as live.

Long-lived service startup replays only grants that still pass those checks.
Stale grants and grants whose workspace is no longer trusted are revoked rather
than activated. Owner application is idempotent. Version update invalidates the
previous transaction's grants and stops its live owners before replacing their
staged declarations. Rollback and removal stop the selected version's live
owners before restoring its snapshots.

MCP activation uses the existing connection manager and tool registry. A pack
MCP server name may not collide with a server in user configuration; ordinary
pack activation never transfers or overrides that runtime ownership. Tools and
server instructions are published only after the connection succeeds and are
removed when the grant is revoked or the pack version is deactivated.

Hook activation is an overlay, not a replacement configuration. The service
retains the latest user hook configuration as the base and deterministically
appends active pack handler groups. User hot reload rebuilds the effective
configuration with the same pack overlays; revocation removes only the selected
pack group. A user-level `handlers_enabled = false` setting prevents activation.
Prompt and agent handlers require the existing `hook_handlers` and `llm_hooks`
features. Reload preserves injected runners, and the service-owned agent runner
exposes only `FileRead`, `Glob`, and `Grep` with dynamic trust.

LSP activation requires a build with the `lsp` feature and user-level LSP
enablement. A pack server ID may not collide with a user server. User server
selection has priority for matching files and workspaces; pack servers extend
coverage instead of overriding it. LSP processes remain lazy under the existing
manager and start on first tool use. Revocation removes the dynamic server and
shuts down its live per-session clients.

Declarative ownership is explicit and persisted separately from the live owner
stores. Each active `(kind, id)` maps to exactly one pack ID and version. A pack
may replace a user-owned resource after explicit replacement approval, because
its transaction snapshot can restore that resource. It may not replace a
resource currently owned by another pack; ownership transfer requires a future
explicit transfer operation rather than ordinary replacement approval.

An update keeps the same pack ID, must increase the semantic version, and in
the first lifecycle implementation must retain the same declarative resource
identity set. Adding or dropping resources remains disabled in version 1; a
future manifest revision must define an explicit per-resource ownership diff
rather than interpreting omission as deletion.

Rollback applies only to the current installed version and restores exactly one
previous snapshot layer. If no earlier version exists, rollback returns the
resources to their pre-pack state and removes ownership. Remove repeatedly
performs that same recoverable rollback until the pack has no installed
versions. A partial remove therefore stops at a valid earlier version rather
than leaving mixed ownership.

## 7. Atomicity and Rollback

Later installation stages must validate the complete pack before live mutation.
Declarative resources commit as one logical transaction. Any failed resource
causes compensation and restores the previous installed version. Removal may
delete only assets and activation grants owned by the selected pack version.

The transaction kernel computes a deterministic dry-run ordered by resource
kind and ID. Changes are classified as add, replace, or unchanged. Replacements
require an explicit install option. MCP, hook, and LSP changes are additionally
marked `requires_activation` and only update inactive staging.

Before each add or replacement, the owning backend captures a restorable
snapshot. A snapshot is recorded before apply so even an apply operation that
partially mutates state can be compensated. Failure restores the failing
resource and every earlier resource in reverse order. Compensation failure is a
distinct terminal result and must never be reported as an ordinary apply
failure.

The durable transaction journal records prepared, applying, awaiting-commit,
commit-decided, rolling-back, committed, rolled-back, and
compensation-failed states. It persists each resource snapshot before mutation
and each applied/restored transition with an atomic file replacement and
directory sync. A journal failure after possible live mutation triggers
immediate compensation. Startup recovery reverses transactions that never
reached a commit decision. Once `commit-decided` is durable, startup completes
the ownership record and commit instead of rolling live state back. A persisted
compensation-failed transaction blocks automatic recovery and requires explicit
operator repair.

The commit decision is the crash-consistency boundary between live owner stores
and the ownership index. Before that decision, an interrupted install is
compensated. After it, ownership publication is idempotently completed. A
managed committed transaction missing from the ownership index is repaired at
startup; an unmanaged generic transaction is never silently claimed.

Filesystem owner snapshots are durable descriptors under the service data
directory, not process-local memory. Workflow snapshots retain the complete
existing row. Owner restoration must be idempotent because startup recovery may
repeat a compensation whose last journal update was interrupted.

The first retention policy is conservative: transaction journals and durable
snapshot payloads for active, superseded, rolled-back, and removed versions are
retained indefinitely. Automatic garbage collection is disabled until an audit
and retention-age policy is standardized. This uses more disk but guarantees
that lifecycle recovery never depends on an already-pruned snapshot.

The service lifecycle API does not imply presentation activation. The shared
GUI and y-web contract exposes preview, HITL progress, cancellation, and typed
lifecycle receipts. CLI lifecycle commands remain absent. Presentation
adapters delegate every mutation to `y-service` and use the existing pending
permission channel for executable activation.

## 8. Validation Result

Validation returns a deterministic report containing the parsed pack identity,
verified resource inventory, and sorted issues. Repeated validation of unchanged
content must produce the same report. Errors are machine-readable and include
the affected resource identity and path when available.

## 9. Feature Flag

Capability Pack support is compiled behind the `capability_packs` feature.
`y-service` keeps the subsystem opt-in as a library feature, while the desktop
and y-web product binaries enable it by default now that equivalent lifecycle
adapters and shared UI management are available. Installation remains inert
until an explicit operation; executable declarations additionally require
workspace trust and HITL activation.
