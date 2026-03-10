# R&D Plan: y-cli

**Module**: `crates/y-cli`
**Phase**: 5.1 (Integration and Release)
**Priority**: Medium — user-facing entry point
**Design References**: `client-commands-design.md`, `client-layer-design.md`
**Depends On**: `y-core`, `y-agent-core`, `y-session`, `y-provider`, `y-context`, `y-hooks`, `y-tools`

---

## 1. Module Purpose

`y-cli` is the user-facing CLI binary. It provides an interactive session mode, configuration management, status/diagnostics commands, and the Tokio runtime entry point. It is the only crate that starts the async runtime and wires all components together.

---

## 2. Dependency Map

```
y-cli
  ├── y-agent-core (Orchestrator, AgentLoop)
  ├── y-provider (ProviderPool construction)
  ├── y-session (SessionManager)
  ├── y-context (ContextPipeline)
  ├── y-hooks (HookSystem, middleware registration)
  ├── y-tools (ToolRegistry)
  ├── y-storage (pool setup, migrations)
  ├── y-runtime (RuntimeManager)
  ├── y-guardrails (GuardrailManager — middleware registration)
  ├── clap (CLI argument parsing)
  ├── tokio (runtime — sole runtime entry point)
  ├── tracing-subscriber (logging initialization)
  ├── anyhow (top-level error reporting)
  └── toml (config file parsing)
```

---

## 3. Module Structure

```
y-cli/src/
  main.rs             — Tokio runtime entry, tracing init, clap dispatch
  config.rs           — ConfigLoader: hierarchy (CLI > env > user > project > defaults)
  commands/
    mod.rs            — Command dispatch
    chat.rs           — Interactive chat session
    status.rs         — System status and diagnostics
    config_cmd.rs     — Configuration management (show, validate, edit)
    session.rs        — Session management (list, resume, branch, archive)
    tool.rs           — Tool management (list, search, info)
    agent.rs          — Agent management (list, define, delegate)
  wire.rs             — Dependency wiring: construct all services from config
  output.rs           — Output formatting (JSON, table, plain text)
```

---

## 4. Development Tasks

### 4.1 Unit Tests (TDD — Red Phase)

#### Task: T-CLI-001 — Config loader

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-CLI-001-01 | `test_config_load_defaults` | No config files | Defaults applied |
| T-CLI-001-02 | `test_config_load_from_toml` | Project config file | Values from file |
| T-CLI-001-03 | `test_config_env_overrides_file` | Env var + file | Env var wins |
| T-CLI-001-04 | `test_config_cli_overrides_all` | CLI arg + env + file | CLI arg wins |
| T-CLI-001-05 | `test_config_validate_catches_errors` | Invalid config | Validation error |
| T-CLI-001-06 | `test_config_secrets_from_env_only` | API key in env | Resolved correctly |

#### Task: T-CLI-002 — Dependency wiring

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-CLI-002-01 | `test_wire_creates_all_services` | Valid config | All services constructed |
| T-CLI-002-02 | `test_wire_registers_middleware` | Wiring complete | All default middleware registered |
| T-CLI-002-03 | `test_wire_feature_gated_services` | Docker disabled | No DockerRuntime constructed |

#### Task: T-CLI-003 — Command parsing

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-CLI-003-01 | `test_parse_chat_command` | `y-agent chat` | Chat command parsed |
| T-CLI-003-02 | `test_parse_status_command` | `y-agent status` | Status command parsed |
| T-CLI-003-03 | `test_parse_session_list` | `y-agent session list` | Session list parsed |
| T-CLI-003-04 | `test_parse_unknown_command` | `y-agent foobar` | Error message |

### 4.2 Integration Tests

| Test ID | File | Test Name | Scenario |
|---------|------|-----------|----------|
| T-CLI-INT-01 | `cli_integration_test.rs` | `test_chat_single_turn` | User message → mock LLM → response displayed |
| T-CLI-INT-02 | `cli_integration_test.rs` | `test_status_command` | Show provider status, session count |
| T-CLI-INT-03 | `cli_integration_test.rs` | `test_config_show` | Display current configuration |
| T-CLI-INT-04 | `cli_integration_test.rs` | `test_session_list_and_resume` | List sessions, resume an existing session |

---

## 5. Implementation Tasks

| Task ID | Task | Description | Priority |
|---------|------|-------------|----------|
| I-CLI-001 | `ConfigLoader` | Config hierarchy with env override and validation | High |
| I-CLI-002 | `main.rs` | Tokio runtime, tracing, clap setup | High |
| I-CLI-003 | `wire.rs` | Dependency wiring from config → all services | High |
| I-CLI-004 | `chat` command | Interactive chat session with agent loop | High |
| I-CLI-005 | `status` command | System status display | Medium |
| I-CLI-006 | `session` subcommands | List, resume, branch, archive | Medium |
| I-CLI-007 | `config` subcommands | Show, validate | Medium |
| I-CLI-008 | `tool` subcommands | List, search, info | Low |
| I-CLI-009 | `agent` subcommands | List, define, delegate | Low |
| I-CLI-010 | Output formatting | JSON, table, plain text output modes | Medium |

---

## 6. Quality Gates

| Gate | Target | Tool |
|------|--------|------|
| Test coverage | >= 70% | `cargo llvm-cov` |
| All tests pass | 100% | `cargo test -p y-cli` |
| Clippy clean | 0 warnings | `cargo clippy -p y-cli` |
| Binary size | < 50MB (release) | `cargo build --release` |

---

## 7. Acceptance Criteria

- [ ] CLI parses all commands correctly
- [ ] Config hierarchy (CLI > env > user > project > defaults) works
- [ ] Interactive chat session runs full agent loop
- [ ] Status command shows provider and session info
- [ ] Session resume works across CLI invocations
- [ ] Clean error messages for user-facing errors
- [ ] Binary compiles and runs standalone
- [ ] Coverage >= 70%
