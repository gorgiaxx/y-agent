# R&D Plan: y-runtime

**Module**: `crates/y-runtime`
**Phase**: 3.3 (Execution Layer)
**Priority**: High — security enforcement layer for tool execution
**Design References**: `runtime-design.md`, `runtime-tools-integration-design.md`
**Depends On**: `y-core`

---

## 1. Module Purpose

`y-runtime` provides isolated execution environments for tools. Three backends implement the `RuntimeAdapter` trait: Docker (container-based isolation for untrusted code), Native/bubblewrap (lightweight sandbox for trusted tools), and SSH (remote execution, deferred). The runtime enforces capability-based permissions — tools declare what they need; the runtime enforces how.

---

## 2. Dependency Map

```
y-runtime
  ├── y-core (traits: RuntimeAdapter, RuntimeCapability, ExecutionRequest/Result)
  ├── tokio (process spawning, timeouts, signals)
  ├── bollard (Docker API — feature: runtime_docker)
  ├── thiserror (errors)
  ├── tracing (backend, duration, exit_code spans)
  └── uuid (container naming)
```

---

## 3. Module Structure

```
y-runtime/src/
  lib.rs              — Public API: RuntimeManager, re-exports
  error.rs            — RuntimeModuleError
  config.rs           — RuntimeConfig (default backend, image whitelist, resource defaults)
  manager.rs          — RuntimeManager: selects backend based on capabilities
  capability.rs       — CapabilityChecker: validates request against policy
  native.rs           — NativeRuntime: bubblewrap/direct process execution
  docker.rs           — DockerRuntime: container lifecycle (feature: runtime_docker)
  ssh.rs              — SshRuntime: skeleton (feature: runtime_ssh)
  cleanup.rs          — Resource cleanup: container removal, temp files
```

---

## 4. Development Tasks

### 4.1 Unit Tests (TDD — Red Phase)

#### Task: T-RT-001 — CapabilityChecker

```
FILE: crates/y-runtime/src/capability.rs
TEST_LOCATION: #[cfg(test)] in same file
```

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-RT-001-01 | `test_capability_no_network_allowed` | Request with `NetworkCapability::None` | Passes |
| T-RT-001-02 | `test_capability_full_network_denied_by_policy` | Request `Full`, policy allows `External` only | `CapabilityDenied` |
| T-RT-001-03 | `test_capability_external_domains_validated` | Request `External{["api.example.com"]}` | Allowed if domain whitelisted |
| T-RT-001-04 | `test_capability_external_domain_blocked` | Request `External{["evil.com"]}` | `CapabilityDenied` |
| T-RT-001-05 | `test_capability_filesystem_mount_validated` | Request mount `/data:ro` | Allowed if in policy |
| T-RT-001-06 | `test_capability_filesystem_host_access_denied` | Request `host_access=true`, policy denies | `CapabilityDenied` |
| T-RT-001-07 | `test_capability_image_whitelist` | Request image `python:3.11` | Allowed if whitelisted |
| T-RT-001-08 | `test_capability_image_not_whitelisted` | Request image `evil:latest` | `ImageNotAllowed` |
| T-RT-001-09 | `test_capability_resource_limits_enforced` | Request 2GB memory, policy allows 512MB | Capped to 512MB |
| T-RT-001-10 | `test_capability_shell_denied` | Request `shell=true`, policy denies | `CapabilityDenied` |

#### Task: T-RT-002 — NativeRuntime

```
FILE: crates/y-runtime/src/native.rs
TEST_LOCATION: #[cfg(test)] in same file
```

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-RT-002-01 | `test_native_execute_simple_command` | Run `echo hello` | Exit code 0, stdout contains "hello" |
| T-RT-002-02 | `test_native_execute_failing_command` | Run `false` | Non-zero exit code |
| T-RT-002-03 | `test_native_execute_with_env` | Run with env var | Env var visible in process |
| T-RT-002-04 | `test_native_execute_with_stdin` | Pipe stdin | Process receives input |
| T-RT-002-05 | `test_native_execute_timeout` | Command exceeds timeout | `RuntimeError::Timeout` |
| T-RT-002-06 | `test_native_execute_output_limit` | Command produces huge output | Truncated to `max_output_bytes` |
| T-RT-002-07 | `test_native_health_check` | `health_check()` | Available on unix |
| T-RT-002-08 | `test_native_backend_type` | `backend()` | Returns `RuntimeBackend::Native` |

#### Task: T-RT-003 — DockerRuntime (feature-gated)

```
FILE: crates/y-runtime/src/docker.rs
TEST_LOCATION: #[cfg(test)] in same file (requires Docker daemon for integration)
```

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-RT-003-01 | `test_docker_execute_in_container` | Run command in container | Correct output |
| T-RT-003-02 | `test_docker_resource_limits_applied` | Memory/CPU limits | Container created with limits |
| T-RT-003-03 | `test_docker_mount_applied` | Mount specification | Volume mounted in container |
| T-RT-003-04 | `test_docker_network_isolation` | `NetworkCapability::None` | Container has no network |
| T-RT-003-05 | `test_docker_cleanup_on_completion` | Execution completes | Container removed |
| T-RT-003-06 | `test_docker_cleanup_on_timeout` | Execution times out | Container killed and removed |
| T-RT-003-07 | `test_docker_image_pull_when_allowed` | `allow_pull=true` | Image pulled |
| T-RT-003-08 | `test_docker_image_pull_denied` | `allow_pull=false`, image missing | Error |
| T-RT-003-09 | `test_docker_health_check` | Docker daemon running | `available=true` |
| T-RT-003-10 | `test_docker_health_check_no_daemon` | Docker not running | `available=false` |

#### Task: T-RT-004 — RuntimeManager

```
FILE: crates/y-runtime/src/manager.rs
TEST_LOCATION: #[cfg(test)] in same file
```

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-RT-004-01 | `test_manager_selects_docker_for_container_caps` | Request with container capability | Uses DockerRuntime |
| T-RT-004-02 | `test_manager_selects_native_for_simple_commands` | Request with no container needs | Uses NativeRuntime |
| T-RT-004-03 | `test_manager_fallback_when_docker_unavailable` | Docker down, simple request | Falls back to Native |
| T-RT-004-04 | `test_manager_error_when_no_backend_available` | All backends unavailable | `RuntimeNotAvailable` |
| T-RT-004-05 | `test_manager_validates_capabilities_before_dispatch` | Request with denied capability | `CapabilityDenied` before backend call |

### 4.2 Integration Tests

```
FILE: crates/y-runtime/tests/
```

| Test ID | File | Test Name | Scenario |
|---------|------|-----------|----------|
| T-RT-INT-01 | `native_integration_test.rs` | `test_native_full_lifecycle` | Execute command, capture output, check resource usage |
| T-RT-INT-02 | `native_integration_test.rs` | `test_native_concurrent_execution` | 5 parallel executions, all complete |
| T-RT-INT-03 | `docker_integration_test.rs` | `test_docker_full_lifecycle` | Create container → execute → capture → cleanup |
| T-RT-INT-04 | `docker_integration_test.rs` | `test_docker_network_isolation_verified` | Container cannot reach external network |
| T-RT-INT-05 | `manager_integration_test.rs` | `test_manager_backend_selection` | Various capability combinations, correct backend |

---

## 5. Implementation Tasks

| Task ID | Task | Description | Priority |
|---------|------|-------------|----------|
| I-RT-001 | `RuntimeConfig` | Config with image whitelist, resource defaults, backend prefs | High |
| I-RT-002 | `CapabilityChecker` | Policy validation for all 4 capability types | High |
| I-RT-003 | `NativeRuntime` | Process execution with timeout, output limit, env | High |
| I-RT-004 | `DockerRuntime` | Container lifecycle via bollard (feature-gated) | High |
| I-RT-005 | `RuntimeManager` | Backend selection, capability pre-check, dispatch | High |
| I-RT-006 | Resource cleanup | Container removal, temp file cleanup, task cleanup | Medium |
| I-RT-007 | `SshRuntime` skeleton | Placeholder impl (deferred to Phase 5) | Low |

---

## 6. Performance Benchmarks

```
FILE: crates/y-runtime/benches/runtime_bench.rs
```

| Benchmark | Target | Measurement |
|-----------|--------|-------------|
| Native execution (echo hello) | P95 < 50ms | `criterion` |
| Capability check | P95 < 100us | `criterion` |
| Docker container create + execute | P95 < 2s | `criterion` |
| Docker cleanup | P95 < 500ms | `criterion` |

---

## 7. Quality Gates

| Gate | Target | Tool |
|------|--------|------|
| Test coverage | >= 75% (Docker tests optional in CI) | `cargo llvm-cov` |
| All tests pass | 100% (native); Docker tests require daemon | `cargo test -p y-runtime` |
| Clippy clean | 0 warnings | `cargo clippy -p y-runtime` |
| No capability bypass | Verified by test | Code review |

---

## 8. Acceptance Criteria

- [ ] Capability checker enforces all 4 capability types (network, fs, container, process)
- [ ] Image whitelist blocks unauthorized images
- [ ] NativeRuntime executes commands with timeout and output limiting
- [ ] DockerRuntime creates isolated containers with resource limits
- [ ] Containers cleaned up on completion, timeout, and error
- [ ] RuntimeManager selects correct backend based on capabilities
- [ ] No capability bypass paths exist
- [ ] Coverage >= 75%
