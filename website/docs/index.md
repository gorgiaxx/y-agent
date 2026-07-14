---
layout: home
title: y-agent -- Rust Agent Harness
titleTemplate: false

head:
  - - meta
    - name: description
      content: y-agent is a Rust-first, model-agnostic Agent Harness with plan and loop execution, self-orchestration, knowledge, self-evolving skills, recovery, and observability.

hero:
  name: y-agent
  text: Rust Agent Harness
  tagline: Goal-directed, self-orchestrating, recoverable, and observable
  image:
    src: /logo-hero.png
    alt: y-agent
  actions:
    - theme: brand
      text: Get Started
      link: /guide/getting-started
    - theme: alt
      text: Architecture
      link: /development/architecture
    - theme: alt
      text: Download
      link: /download

features:
  - title: Plan and Loop Execution
    details: Use reviewed structured plans for known work or iterative loops for exploratory tasks.
  - title: Self-Orchestration
    details: Delegate to sub-agents and create reusable workflows backed by a checkpointed DAG engine.
  - title: Self-Evolving Skills
    details: Capture experience, extract patterns, validate proposals, and publish versioned improvements with explicit approval.
  - title: Knowledge-Aware Context
    details: Ingest domain material, build multi-resolution chunks, and combine keyword and vector retrieval.
  - title: Tool and MCP Harness
    details: Discover and execute built-in, dynamic, and MCP tools through schema validation, permissions, and sandboxing.
  - title: Recovery and Observability
    details: Persist WAL-backed state, checkpoints, transcripts, journals, traces, usage, cost, and replay data.
---
