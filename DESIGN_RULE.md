# DESIGN_RULE.md -- Design Document Standards & Playbooks

This project is an AI agent built for personal research purposes, with extremely high demands on architectural design.
You are assisting with software design documentation.

---

## 1) Output Style

- Write in clear professional English.
- Do NOT use emoji.
- Do NOT use ASCII/character diagrams.

## 2) Diagram Policy

- Use Mermaid for all diagrams.
- Choose Mermaid type by intent:
  - flowchart: architecture/module boundaries, dependencies, deployment topology
  - sequenceDiagram: request/response and service interactions over time
  - stateDiagram-v2: lifecycle/state transitions
  - erDiagram: data/entity relationships
  - gantt: milestones/timeline (optional)
- Every diagram must include:
  - a short title
  - one-sentence rationale for diagram type
  - a brief legend for critical nodes/edges

## 3) Abstraction Level

- Focus on high-level design only:
  - goals, scope, components, boundaries, interfaces, data/state flow, trade-offs, risks
- Avoid implementation details:
  - long code snippets
  - class/file-level specifics
  - low-level framework/config details
- If code is necessary, keep it minimal and illustrative.

## 4) Required Sections in Each Design Doc

- TL;DR
- Background and Goals
- Scope (In/Out)
- High-Level Design
- Key Flows/Interactions
- Data and/or State Model
- Failure Handling and Edge Cases
- Security and Permissions
- Performance and Scalability
- Observability
- Rollout and Rollback
- Alternatives and Trade-offs
- Open Questions

## 5) Decision Quality

- State assumptions explicitly.
- Provide measurable success criteria where possible.
- Document rejected alternatives briefly.
- For open questions, include owner and due date.

## 6) Output Constraint

- Prefer concise, structured Markdown.
- Ensure Mermaid blocks are syntactically valid.
- The new design document should be aligned with DESIGN_OVERVIEW.md.

---

## 7) Deep Architecture Observations (Design-Phase Realities)

These design-phase realities must drive every design-document decision:

1. **Design documents are the primary product right now**
    - 24 design docs under `docs/design/` form an interconnected system.
    - Cross-document consistency is a hard requirement; changes to one doc frequently require updates to others.
    - `DESIGN_OVERVIEW.md` is the authoritative index and must stay in sync with all sub-documents.

2. **Cross-cutting alignment is fragile**
    - Multiple subsystems share concepts (permissions, memory tiers, collaboration patterns, middleware).
    - The `Cross-Cutting Design Alignment` table in `DESIGN_OVERVIEW.md` records authoritative resolutions for conflicting designs.
    - Always check this table before introducing or modifying shared concepts.

3. **The design follows strict layered separation**
    - Tools define *what* they need; Runtime enforces *how* (capability model).
    - Hooks observe (read-only); Middleware transforms (mutable); EventBus notifies (async, fire-and-forget).
    - Skills are LLM-instruction-only; embedded tools/scripts are extracted to the Tool Registry.
    - Guardrails are implemented *as* middleware in the y-hooks chains, not as a parallel system.

4. **Model-agnostic design is a core constraint**
    - The Micro-Agent Pipeline exists specifically to enable weaker LLMs to handle complex tasks.
    - Skill root documents are capped at 2,000 tokens to minimize attention dilution.
    - Adaptive step merging adjusts pipeline granularity based on model capability.
    - Never design features that assume top-tier model capabilities.

---

## 8) Design Document Change Workflow

### 8.1 Read Before Write

- Read the target doc, `DESIGN_OVERVIEW.md`, and this file (`DESIGN_RULE.md`) before any edit.
- Check the Cross-Cutting Design Alignment table for relevant authoritative decisions.
- Read adjacent docs that share concepts with the target.

### 8.2 Check Cross-Document Impact

- If adding or modifying a shared concept (permission model, memory tier, collaboration pattern, hook point, middleware chain, event type), identify all documents that reference it.
- Make all necessary cross-document updates in the same change.

### 8.3 Follow This Template Strictly

- All 13 required sections (Section 4) must be present.
- Mermaid diagrams only; each with type rationale and legend.
- Professional English, no emoji, no ASCII diagrams.
- High-level design focus; minimal illustrative code only.

### 8.4 Update DESIGN_OVERVIEW.md

- If a new design doc is created, add it to the Component Overview table and Related Documents section.
- If a cross-cutting decision changed, update the Cross-Cutting Design Alignment table.

---

## 9) Change Playbooks

### 9.1 Adding a New Design Document

- Create the doc following this template (all 13 sections from Section 4).
- If it introduces shared concepts, add entries to the Cross-Cutting Design Alignment table.
- Update adjacent docs that need to reference the new module.

### 9.2 Modifying a Shared Concept

Examples: adding a memory tier, adding a collaboration pattern, adding middleware, changing the permission model.

- Update the authoritative doc first.
- Update all docs that reference the concept (use the Cross-Cutting Alignment table to find them).
- Update `DESIGN_OVERVIEW.md` Cross-Cutting Alignment table if the resolution changed.

### 9.3 Resolving a Cross-Document Conflict

- Determine which doc is authoritative (check the Cross-Cutting Alignment table or project history).
- Update the non-authoritative doc to align.
- Add or update the Cross-Cutting Alignment table entry with the resolution.

### 9.4 Adding a Hook Point, Middleware, or Event Type

- Define the hook/middleware/event in the module that owns the lifecycle (e.g., pipeline hooks in micro-agent-pipeline-design.md).
- Register it in `hooks-plugin-design.md`: Hook Points table, Known Middleware Implementations table, or Event Types table.
- If the hook/event is consumed by another module, update that module's doc.

---

## 10) Design Document Validation Checklist

Before completing any design document change, verify:

- [ ] All 13 required sections present per Section 4
- [ ] Mermaid diagrams syntactically valid with type rationale and legend
- [ ] No emoji, no ASCII diagrams
- [ ] Cross-references to other docs use correct file paths
- [ ] Shared concepts consistent across all referencing docs
- [ ] `DESIGN_OVERVIEW.md` Component Overview table reflects current state
- [ ] `DESIGN_OVERVIEW.md` Cross-Cutting Alignment table up to date
- [ ] Open questions have owner and due date
- [ ] Success criteria are measurable where applicable

---

## 11) Design Anti-Patterns (Do Not)

- Do not create parallel systems for the same concern (e.g., a second permission model alongside guardrails).
- Do not embed tools or scripts in skill definitions; extract them to the Tool Registry.
- Do not design features that require top-tier LLMs to function; use the Micro-Agent Pipeline pattern for complex operations.
- Do not modify `DESIGN_OVERVIEW.md` without checking all cross-references.
- Do not add hook points to `hooks-plugin-design.md` without defining them in the owning module's doc first.
- Do not mix design-level and implementation-level content in the same doc.
- Do not use the term "Working Memory" for Short-Term Memory; Working Memory is the pipeline-scoped third memory tier.
- Do not introduce new required sections to design docs without updating this file (`DESIGN_RULE.md`).
- Do not leave cross-document conflicts unresolved; fix all affected docs in the same change.