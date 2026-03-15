# Skills Module R&D Plan — Subagent-Based Ingestion & Evolution Push

**Version**: v0.2
**Created**: 2026-03-11
**Status**: Draft
**Design References**: `skills-knowledge-design.md` (v0.3), `skill-versioning-evolution-design.md` (v0.2)
**Research Reference**: `autoskill-ell.md`

---

## 1. Current State

### 1.1 What's Done

`y-skills` has **29 source files** and **117 passing tests**. All structural code exists:

| Area | Status |
|------|--------|
| Skill Registry (register, search, rollback) | ✅ Complete |
| Version Control (CAS + reflog + diff + GC) | ✅ Complete |
| Manifest Parsing (nested TOML) | ✅ Complete |
| Evolution Framework (metrics, proposals, approval) | ✅ Complete |
| Experience Capture (evidence provenance) | ✅ Complete |
| Pattern Extraction (incl. skillless analysis) | ✅ Complete |
| Ingestion/Transformation modules (analyzer, decomposer, classifier, etc.) | ⚠️ Deterministic rules only |

### 1.2 What's Missing

| Gap | Description |
|-----|-------------|
| **Intelligent ingestion/transformation** | analyzer/decomposer/classifier use pattern matching instead of LLM reasoning |
| **Feature flag wiring** | Flags defined in `Cargo.toml` but not applied to modules |
| **Cross-module integration** | Skills not wired into Orchestrator, Hooks, CLI |
| **CLI commands** | `commands/skills.rs` exists as stub |

### 1.3 Design Document Assessment

> [!IMPORTANT]
> **All design documents are comprehensive and sufficient.** No supplementation needed.

---

## 2. Architecture Decision: Subagent-Based Ingestion

### 2.1 Problem with Scattered LLM Calls

The original approach was to add `LlmProvider` calls individually to `analyzer.rs`, `decomposer.rs`, `classifier.rs`, `security.rs` — each module making its own independent LLM call. Problems:

- 4+ independent LLM calls per ingestion, each needing its own prompt engineering
- No shared context between analysis, classification, decomposition, and conversion
- Introduces a new `SkillLlmProvider` abstraction not in the design documents

### 2.2 Subagent Approach

Create a **`skill-ingestion` agent** that handles the entire third-party skill transformation as a single, coherent task:

```
CLI: y skill import ./path/to/skill.md
  │
  └─► AgentDelegator.delegate("skill-ingestion", {path: "./path/to/skill.md"})
        │
        └─► skill-ingestion agent (TOML-defined, with preset prompt + tools)
              ├── read_file(path)         → reads source content
              ├── analyze_content()       → analyzes purpose, capabilities, security
              ├── query_registry()        → checks for duplicates/overlaps
              ├── decompose_and_convert() → transforms to proprietary format
              └── register_skill()        → writes to SkillRegistry
```

**Advantages**:
- **Context coherent**: the agent reads the file once and does all analysis in one context window
- **Uses existing infra**: `AgentDelegator` + `AgentPool` + `AgentRunner` — no new abstractions
- **Configurable**: system prompt defines transformation rules, tools define what it can do
- **Composable**: the agent can call other agents if needed (e.g., a security-audit agent)

### 2.3 Agent-to-Agent Delegation: Current Framework State

**Question**: Can subagents call other agents? Is this already supported?

**Answer**: The architecture **supports it**, but the runner needs to be extended:

| Layer | Status | What it does |
|-------|--------|-------------|
| `AgentDelegator` trait (`y-core`) | ✅ Ready | Any module can call `delegate(agent_name, input)` |
| `AgentPool` implements `AgentDelegator` | ✅ Ready | Resolves agent definition, builds run config, executes via runner |
| `DelegationProtocol` | ✅ Ready | Tracks depth, timeout, parent-child chains |
| `delegation_depth` on `AgentInstance` | ✅ Ready | Prevents infinite recursion (configurable `max_delegation_depth`) |
| `AgentRunner` trait | ⚠️ Single-turn only | Current `SingleTurnRunner`: system_prompt + input → **one** LLM call |

**Current limitation**: `SingleTurnRunner` does `system_prompt + input → single LLM call → text output`. This is fine for system agents like `title-generator` (one-shot tasks), but a `skill-ingestion` agent needs **multi-turn with tool use** (read file → analyze → decide → write).

**Two approaches to solve this**:

| Approach | Description | Effort |
|----------|-------------|--------|
| **A: Single-turn with structured output** | Agent receives file content in input JSON, returns structured JSON with transformation result. Tools run by the caller, not the agent. | Low — works now |
| **B: Multi-turn `AgentRunner`** | New `MultiTurnRunner` that runs an agent loop (LLM call → tool execution → LLM call → ...). True autonomous agent. | Medium — requires `y-agent (orchestrator)` orchestrator integration |

**Recommendation**: Start with **Approach A** (single-turn) for the initial implementation. The agent receives the file content + registry state as input, and returns the transformation result as structured output. The CLI/caller executes the actual writes. This works with the existing `SingleTurnRunner` and can be upgraded to Approach B when the multi-turn runner is ready.

> [!NOTE]
> Approach B (multi-turn runner) is a fundamental capability needed across all subagents, not just skills. It should be developed as part of the `y-agent (orchestrator)` orchestrator R&D, not as a skills-specific feature.

---

## 3. R&D Tracks

### Track A — Skill Ingestion Agent (Est. 3-4 days)

Create the `skill-ingestion` agent that transforms third-party skills.

#### A1: Agent Definition (Est. 0.5 day)

##### [NEW] [skill-ingestion.toml](file:///Users/gorgias/Projects/y-agent/config/agents/skill-ingestion.toml)

Agent TOML config with:
- `mode = "build"` — this agent transforms/creates
- System prompt containing:
  - Proprietary format specification (root.md + sub-documents + skill.toml schema)
  - Transformation rules from `skills-knowledge-design.md` §Transformation Engine
  - Security screening rules (prompt injection, privilege escalation detection)
  - Token budget constraints (root < 2000 tokens)
  - Classification taxonomy (`llm_reasoning` / `api_call` / `tool_wrapper` / `agent_behavior` / `hybrid`)
  - Output schema (structured JSON for `SkillManifest` + decomposed documents)
- `allowed_tools = []` — Approach A, tools handled by caller
- `preferred_models = ["gpt-4o", "claude-sonnet-4-20250514"]` — needs strong reasoning
- `temperature = 0.2` — precise, deterministic output
- `max_context_tokens = 16384` — needs room for file content + output

##### Tests

| Test ID | Description | Type |
|---------|-------------|------|
| T-SK-A1-01 | Agent TOML parses into valid `AgentDefinition` | Unit |

---

#### A2: Ingestion Service (Est. 2-3 days)

Wire the agent delegation into a service layer that handles the full import workflow.

##### [NEW] [skill_ingestion_service.rs](file:///Users/gorgias/Projects/y-agent/crates/y-service/src/skill_ingestion_service.rs)

`SkillIngestionService`:
1. Read source file from path
2. Run deterministic pre-checks (format detection, size limits)
3. Delegate to `skill-ingestion` agent via `AgentDelegator`:
   - Input: `{ source_content, source_format, existing_skills: [...], existing_tools: [...] }`
   - Output: structured JSON with `{ classification, security_verdict, manifest, root_content, sub_documents[], extracted_tools[] }`
4. Validate agent output (schema check, token budget check)
5. Register in `SkillRegistry` via existing `register()` method
6. Track lineage via existing `LineageRecord`

The service uses the existing deterministic modules as **pre/post validators**:
- `FormatDetector` — pre-check (deterministic)
- `SecurityScreener` — post-check (defense in depth: agent does first pass, deterministic rules verify)
- `SkillValidator` — post-check (format/schema/token/uniqueness)

```rust
pub struct SkillIngestionService {
    delegator: Arc<dyn AgentDelegator>,
    registry: Arc<dyn SkillRegistry>,
    // Deterministic validators (existing code)
    format_detector: FormatDetector,
    security_screener: SecurityScreener,
    skill_validator: SkillValidator,
}

impl SkillIngestionService {
    pub async fn import(&self, path: &Path) -> Result<ImportResult, SkillModuleError> {
        // 1. Read + format detect (deterministic)
        // 2. Delegate to skill-ingestion agent
        // 3. Parse structured output
        // 4. Security post-check (deterministic)
        // 5. Validate (deterministic)
        // 6. Register
    }
}
```

##### Tests

| Test ID | Description | Type |
|---------|-------------|------|
| T-SK-A2-01 | Markdown file → agent output → registered skill | Integration (mock delegator) |
| T-SK-A2-02 | Agent returns unsafe content → blocked by post-check | Integration (mock delegator) |
| T-SK-A2-03 | Agent returns oversized root → rejected by validator | Integration (mock delegator) |
| T-SK-A2-04 | Agent classifies as `api_call` → rejected with tool redirect | Integration (mock delegator) |
| T-SK-A2-05 | Duplicate skill name → handled by registry | Integration (mock delegator) |
| T-SK-A2-06 | Batch import 3 files → results per file | Integration (mock delegator) |

---

#### A3: CLI Commands (Est. 1 day)

Wire CLI `skill` subcommands.

##### [MODIFY] [skills.rs](file:///Users/gorgias/Projects/y-agent/crates/y-cli/src/commands/skills.rs)

- `skill import <path>` — calls `SkillIngestionService::import()`
- `skill list` — table display from `SkillRegistry`
- `skill inspect <name>` — full manifest + version history
- `skill validate` — run `SkillValidator` on all registered skills
- `skill rollback <name> <version>` — rollback via `SkillRegistry`

##### Tests

| Test ID | Description | Type |
|---------|-------------|------|
| T-SK-A3-01 | `skill list` outputs formatted table | Manual CLI |
| T-SK-A3-02 | `skill import path/to/skill.md` succeeds | Manual CLI |

---

### Track B — Evolution Loop Wiring (Est. 3-4 days)

### B1: Skill Usage Audit Hook (Est. 1-2 days)

##### [NEW] Skill Usage Audit as post-task hook

Same subagent pattern: create a `skill-usage-auditor` agent that receives:
- Input: `{ injected_skills[], user_query, assistant_reply }`
- Output: `{ verdicts: [{ skill_id, relevant: bool, used: bool }] }`

Results update `SkillMetrics.injection_count` and `actual_usage_count`.

Fallback: deterministic keyword overlap (existing `SkillUsageAudit` code).

Feature-gated: `skill_usage_audit`

##### [NEW] [skill-usage-auditor.toml](file:///Users/gorgias/Projects/y-agent/config/agents/skill-usage-auditor.toml)

- Lightweight agent, `preferred_models = ["gpt-4o-mini"]`
- System prompt with the core rule: "If the reply can be produced well without this skill, set used=false"

##### Tests

| Test ID | Description | Type |
|---------|-------------|------|
| T-SK-B1-01 | Audit hook updates metrics | Unit (mock delegator) |
| T-SK-B1-02 | Low usage_rate triggers obsolete signal | Unit |

---

### B2: Experience Capture Hook (Est. 1 day)

Post-task hook to auto-create `ExperienceRecord` from execution data, classifying evidence with provenance tags.

Feature-gated: `evolution_capture`

##### Tests

| Test ID | Description | Type |
|---------|-------------|------|
| T-SK-B2-01 | Task complete → experience record created | Unit (mock) |

---

### B3: Evolution Scheduling (Est. 1 day)

Periodic `PatternExtractor::extract()` via `y-scheduler` or `tokio::spawn` interval.

Feature-gated: `evolution_extraction`

---

### Track C — Feature Flags & Cleanup (Est. 0.5 day)

#### [MODIFY] [lib.rs](file:///Users/gorgias/Projects/y-agent/crates/y-skills/src/lib.rs)

Apply `#[cfg(feature = "...")]` to module declarations. Core modules always on; advanced modules opt-in.

---

## 4. Prioritized Execution Order

| Priority | Phase | Track | Est. | Dependencies |
|----------|-------|-------|------|--------------|
| **P0** | A1 | Agent Definition | 0.5d | None |
| **P0** | A2 | Ingestion Service | 2-3d | A1, `y-service` |
| **P1** | A3 | CLI Commands | 1d | A2 |
| **P2** | B1 | Usage Audit Hook | 1-2d | `y-hooks` |
| **P2** | B2 | Experience Capture | 1d | `y-hooks` |
| **P3** | B3 | Evolution Scheduling | 1d | `y-scheduler` |
| **P3** | C | Feature Flags | 0.5d | All |

**Total**: ~8-10 days

---

## 5. Verification Plan

### Automated Tests

After each phase:

```bash
# All existing tests must continue passing
cargo test -p y-skills

# Clippy clean
cargo clippy -p y-skills -- -D warnings

# Service crate tests
cargo test -p y-service

# Full workspace build
cargo build --workspace
```

### Phase-Specific

| Phase | Verification | Expected |
|-------|-------------|----------|
| A1 | `cargo test -p y-agent -- definition` | Agent TOML parses |
| A2 | `cargo test -p y-service -- skill_ingestion` | Mock integration tests pass |
| A3 | `cargo run -p y-cli -- skill list` | CLI outputs table |
| B1 | `cargo test -p y-skills -- usage_audit` | Audit hook tests pass |

### Regression Security

- 117 existing tests must pass at every phase
- No modification to existing public APIs

---

## 6. Risks & Mitigations

| Risk | Mitigation |
|------|------------|
| LLM structured output unreliable | Deterministic post-validators (security, schema, token budget) catch bad output |
| Single-turn agent insufficient for complex skills | Upgrade to multi-turn runner later; single-turn handles 90% of cases |
| Agent prompt too long for context | Modular prompt: base rules + format spec loaded from skill.toml template |

---

## 7. Future: Multi-Turn Agent Runner

When `y-agent (orchestrator)` develops a multi-turn orchestrator loop, the skill-ingestion agent naturally upgrades:

- Add `allowed_tools = ["read_file", "write_skill", "query_registry", "validate_skill"]`
- Agent autonomously reads, analyzes, queries, and registers — no caller orchestration needed
- Same TOML definition, just enable tools and switch to `MultiTurnRunner`
- Agent can delegate to other agents (e.g., `skill-usage-auditor`) via `AgentDelegator`

This is fully supported by the existing `DelegationProtocol` (depth tracking, timeout, parent chains).
