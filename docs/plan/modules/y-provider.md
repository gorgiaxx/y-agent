# R&D Plan: y-provider

**Module**: `crates/y-provider`
**Phase**: 2.1 (Core Runtime)
**Priority**: High — LLM communication is the core agent capability
**Design References**: `providers-design.md`
**Depends On**: `y-core`

---

## 1. Module Purpose

`y-provider` implements the provider pool: multi-provider LLM management with tag-based routing, intelligent freeze/thaw, per-provider concurrency limits, rate limiting, and connection pooling. It is the sole gateway for all LLM API calls.

---

## 2. Dependency Map

```
y-provider
  ├── y-core (traits: LlmProvider, ProviderPool, ProviderMetadata, ProviderError)
  ├── reqwest (HTTP client with connection pooling)
  ├── tokio (async runtime, semaphores, timers)
  ├── serde / serde_json (request/response serialization)
  ├── thiserror (error types)
  ├── tracing (structured logging — provider, model, tokens, duration)
  └── uuid, chrono (request IDs, timestamps)
```

---

## 3. Module Structure

```
y-provider/src/
  lib.rs              — Public API: ProviderPoolImpl, re-exports
  error.rs            — Crate-level ProviderPoolError
  config.rs           — ProviderPoolConfig, individual ProviderConfig
  pool.rs             — ProviderPoolImpl: routing, freeze/thaw, concurrency
  router.rs           — TagBasedRouter: tag matching, priority scheduling
  freeze.rs           — FreezeManager: adaptive freeze durations, thaw scheduling
  health.rs           — HealthChecker: periodic checks, thaw verification
  providers/
    mod.rs            — Provider factory
    openai.rs         — OpenAI API client (feature: provider_openai)
    anthropic.rs      — Anthropic API client (feature: provider_anthropic)
    ollama.rs         — Ollama local API client (feature: provider_ollama)
  metrics.rs          — Per-provider metrics (requests, errors, latency, tokens)
```

---

## 4. Development Tasks

### 4.1 Unit Tests (TDD — Red Phase)

#### Task: T-PROV-001 — ProviderPoolConfig

```
FILE: crates/y-provider/src/config.rs
TEST_LOCATION: #[cfg(test)] in same file
```

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-PROV-001-01 | `test_config_deserialize_from_toml` | TOML → `ProviderPoolConfig` | Parses providers array |
| T-PROV-001-02 | `test_config_validate_no_providers_fails` | `validate()` with empty list | Error |
| T-PROV-001-03 | `test_config_validate_duplicate_ids_fails` | Two providers same ID | Error |
| T-PROV-001-04 | `test_config_default_concurrency` | Default `max_concurrency` | 5 |
| T-PROV-001-05 | `test_config_api_key_from_env` | `api_key_env` resolution | Reads from env var |

#### Task: T-PROV-002 — Tag-based routing

```
FILE: crates/y-provider/src/router.rs
TEST_LOCATION: #[cfg(test)] in same file
```

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-PROV-002-01 | `test_routing_selects_by_single_tag` | Route with tag `"reasoning"` | Returns provider with that tag |
| T-PROV-002-02 | `test_routing_selects_by_multiple_tags` | Route with `["fast", "code"]` | Returns provider matching ALL tags |
| T-PROV-002-03 | `test_routing_no_match_returns_error` | Route with unmatched tag | `NoProviderAvailable` error |
| T-PROV-002-04 | `test_routing_skips_frozen_providers` | Route with one frozen provider | Returns the unfrozen one |
| T-PROV-002-05 | `test_routing_preferred_model_exact_match` | Route with `preferred_model` | Prefers exact model match |
| T-PROV-002-06 | `test_routing_critical_priority_reserves_budget` | Critical request when at 80% capacity | Still routes (reserved 20%) |
| T-PROV-002-07 | `test_routing_idle_priority_defers_when_busy` | Idle request when at capacity | `NoProviderAvailable` or queued |
| T-PROV-002-08 | `test_routing_round_robin_among_equal_candidates` | Multiple matching providers | Load distributed |

#### Task: T-PROV-003 — Freeze/thaw manager

```
FILE: crates/y-provider/src/freeze.rs
TEST_LOCATION: #[cfg(test)] in same file
```

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-PROV-003-01 | `test_freeze_on_rate_limit` | Report `RateLimited` error | Provider frozen, thaw scheduled for retry_after |
| T-PROV-003-02 | `test_freeze_on_auth_failure` | Report `AuthenticationFailed` | Provider frozen indefinitely (permanent) |
| T-PROV-003-03 | `test_freeze_duration_adaptive` | Multiple transient errors | Freeze duration increases exponentially |
| T-PROV-003-04 | `test_thaw_after_health_check` | Frozen provider, time passes, health OK | Provider thawed |
| T-PROV-003-05 | `test_thaw_fails_if_unhealthy` | Frozen provider, health check fails | Stays frozen, new freeze schedule |
| T-PROV-003-06 | `test_manual_freeze` | `pool.freeze()` call | Provider frozen with reason |
| T-PROV-003-07 | `test_manual_thaw` | `pool.thaw()` call | Health check then thaw |
| T-PROV-003-08 | `test_freeze_status_reporting` | `provider_statuses()` | Correct is_frozen, frozen_since, thaw_at |

#### Task: T-PROV-004 — Concurrency control

```
FILE: crates/y-provider/src/pool.rs
TEST_LOCATION: #[cfg(test)] in same file
```

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-PROV-004-01 | `test_concurrency_limit_enforced` | Exceed max_concurrency | Queued or rejected |
| T-PROV-004-02 | `test_concurrency_semaphore_release_on_completion` | Request completes | Semaphore permit released |
| T-PROV-004-03 | `test_concurrency_semaphore_release_on_error` | Request fails | Permit released (no leak) |
| T-PROV-004-04 | `test_active_request_count_tracking` | During concurrent requests | `active_requests` accurate |

#### Task: T-PROV-005 — Provider metrics

```
FILE: crates/y-provider/src/metrics.rs
TEST_LOCATION: #[cfg(test)] in same file
```

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-PROV-005-01 | `test_metrics_increment_total_requests` | After successful call | `total_requests` incremented |
| T-PROV-005-02 | `test_metrics_increment_total_errors` | After failed call | `total_errors` incremented |
| T-PROV-005-03 | `test_metrics_track_token_usage` | After call with usage | Input/output tokens accumulated |
| T-PROV-005-04 | `test_metrics_reset` | Reset counters | All zeroed |

#### Task: T-PROV-006 — OpenAI provider (feature-gated)

```
FILE: crates/y-provider/src/providers/openai.rs
TEST_LOCATION: #[cfg(test)] in same file
```

| Test ID | Test Name | Behavior | Assertion |
|---------|-----------|----------|-----------|
| T-PROV-006-01 | `test_openai_request_format` | Build HTTP request from `ChatRequest` | Correct URL, headers, body JSON |
| T-PROV-006-02 | `test_openai_response_parse` | Parse JSON response body | Correct `ChatResponse` fields |
| T-PROV-006-03 | `test_openai_stream_chunk_parse` | Parse SSE stream chunks | Correct `ChatStreamChunk` sequence |
| T-PROV-006-04 | `test_openai_rate_limit_error_parse` | 429 response | `ProviderError::RateLimited` with retry_after |
| T-PROV-006-05 | `test_openai_auth_error_parse` | 401 response | `ProviderError::AuthenticationFailed` |
| T-PROV-006-06 | `test_openai_metadata` | `metadata()` | Correct provider_type, model, tags |

### 4.2 Integration Tests

```
FILE: crates/y-provider/tests/
```

| Test ID | File | Test Name | Scenario |
|---------|------|-----------|----------|
| T-PROV-INT-01 | `pool_integration_test.rs` | `test_pool_routes_to_best_provider` | 3 providers, route by tag, verify selection |
| T-PROV-INT-02 | `pool_integration_test.rs` | `test_pool_failover_on_error` | Primary frozen, fallback selected |
| T-PROV-INT-03 | `pool_integration_test.rs` | `test_pool_freeze_thaw_lifecycle` | Error → freeze → wait → health check → thaw |
| T-PROV-INT-04 | `pool_integration_test.rs` | `test_pool_concurrent_requests` | 10 parallel requests, concurrency respected |
| T-PROV-INT-05 | `pool_integration_test.rs` | `test_pool_all_providers_frozen` | All frozen → `NoProviderAvailable` |

---

## 5. Implementation Tasks

| Task ID | Task | Description | Priority |
|---------|------|-------------|----------|
| I-PROV-001 | `ProviderPoolConfig` | Config parsing, validation, env var resolution | High |
| I-PROV-002 | `TagBasedRouter` | Tag matching, priority scheduling, frozen exclusion | High |
| I-PROV-003 | `FreezeManager` | Adaptive freeze durations, thaw timer, health verification | High |
| I-PROV-004 | `ProviderPoolImpl` | Main pool struct: `ProviderPool` trait impl, semaphores | High |
| I-PROV-005 | `HealthChecker` | Periodic background health checks for frozen providers | Medium |
| I-PROV-006 | `OpenAiProvider` | HTTP client for OpenAI API (feature: `provider_openai`) | High |
| I-PROV-007 | `AnthropicProvider` | HTTP client for Anthropic API (feature: `provider_anthropic`) | High |
| I-PROV-008 | `OllamaProvider` | HTTP client for Ollama local API (feature: `provider_ollama`) | Medium |
| I-PROV-009 | `ProviderMetrics` | Per-provider counters and token tracking | Medium |

---

## 6. Performance Benchmarks

```
FILE: crates/y-provider/benches/routing_bench.rs
```

| Benchmark | Target | Measurement |
|-----------|--------|-------------|
| Provider routing (10 providers) | P95 < 1ms | `criterion` |
| Provider routing (100 providers) | P95 < 5ms | `criterion` |
| Freeze check | P95 < 100us | `criterion` |
| Metrics recording | P95 < 10us | `criterion` |

---

## 7. Quality Gates

| Gate | Target | Tool |
|------|--------|------|
| Test coverage | >= 80% | `cargo llvm-cov` |
| All tests pass | 100% | `cargo test -p y-provider` |
| Clippy clean | 0 warnings | `cargo clippy -p y-provider` |
| No `unwrap()` in lib code | 0 | Manual review |
| Benchmarks | No regression > 10% P95 | `cargo bench -p y-provider` |

---

## 8. Acceptance Criteria

- [ ] Tag-based routing correctly selects among 3+ providers
- [ ] Frozen providers are excluded from routing
- [ ] Adaptive freeze durations increase on repeated failures
- [ ] Health-check-gated thaw works end-to-end
- [ ] Concurrency limits prevent overload (semaphore-based)
- [ ] At least one provider backend (OpenAI) fully functional
- [ ] All integration tests pass with mock HTTP responses
- [ ] Benchmark baselines established
- [ ] Coverage >= 80%
