# Agent Crate Merge R&D Plan — y-agent-core + y-multi-agent → y-agent

**Version**: v0.2
**Created**: 2026-03-11
**Completed**: 2026-03-12
**Status**: Completed
**Research Reference**: [`MERGE_AGENT_CORE_MULTI_AGENT.md`](../research/MERGE_AGENT_CORE_MULTI_AGENT.md)
**Supersedes**: `y-agent-core` crate, `y-multi-agent` crate (both absorbed into unified `y-agent`)

---

## 1. Overview

将 `y-agent-core`（DAG/workflow 编排引擎，9 个模块，~72KB）与 `y-multi-agent`（Agent 生命周期管理，16+ 个模块，~173KB）合并为单一 crate **`y-agent`**。合并后的 crate 通过 `orchestrator/` 和 `agent/` 两个子模块组织代码，实现 agent 对编排能力的直接访问，解锁递归 Agent 模型和未来的 MultiTurnRunner。

### Key Motivations

1. **递归 Agent 模型** — Agent 和 SubAgent 使用完全相同的执行路径，不再有 "subagent" 这一特殊概念
2. **MultiTurnRunner 天然落地** — Agent loop 直接使用 DAG executor + checkpoint + interrupt
3. **消费端简化** — `y-cli` 和 `y-service` 从两个依赖变为一个
4. **设计意图对齐** — multi-agent-design.md 明确写道 "Agents are first-class executors in the existing DAG engine"

### Scope

- **In scope**: 物理模块迁移、`use` 路径更新、命名冲突解决、Cargo.toml 更新、workspace 配置、旧 crate 移除、设计文档更新
- **Out of scope**: MultiTurnRunner 实现（后续 R&D）、新功能开发、业务逻辑变更

---

## 2. Current State Assessment

### 2.1 y-agent-core — 编排器

| 模块 | 行数 | 迁移目标 |
|------|------|---------|
| `dag.rs` | 280 | `y-agent::orchestrator::dag` |
| `channel.rs` | 184 | `y-agent::orchestrator::channel` |
| `checkpoint.rs` | 158 | `y-agent::orchestrator::checkpoint` |
| `executor.rs` | 256 | `y-agent::orchestrator::executor` |
| `interrupt.rs` | 182 | `y-agent::orchestrator::interrupt` |
| `expression_dsl.rs` | 639 | `y-agent::orchestrator::expression_dsl` |
| `workflow_meta.rs` | 190 | `y-agent::orchestrator::workflow_meta` |
| `micro_pipeline.rs` | 403 | `y-agent::orchestrator::micro_pipeline` |

### 2.2 y-multi-agent — Agent 生命周期

| 模块 | 行数 | 迁移目标 |
|------|------|---------|
| `definition.rs` | 302 | `y-agent::agent::definition` |
| `registry.rs` | 695 | `y-agent::agent::registry` |
| `pool.rs` | 711 | `y-agent::agent::pool` |
| `delegation.rs` | 252 | `y-agent::agent::delegation` |
| `executor.rs` | 454 | `y-agent::agent::executor` |
| `context.rs` | ~450 | `y-agent::agent::context` |
| `mode.rs` | ~420 | `y-agent::agent::mode` |
| `dynamic_agent.rs` | 910 | `y-agent::agent::dynamic_agent` |
| `gap.rs` | ~400 | `y-agent::agent::gap` |
| `meta_tools.rs` | ~600 | `y-agent::agent::meta_tools` |
| `task_tool.rs` | ~340 | `y-agent::agent::task_tool` |
| `trust.rs` | 112 | `y-agent::agent::trust` |
| `config.rs` | 72 | `y-agent::agent::config` |
| `error.rs` | 27 | `y-agent::agent::error` |
| `patterns/sequential.rs` | ~200 | `y-agent::agent::patterns::sequential` |
| `patterns/hierarchical.rs` | ~200 | `y-agent::agent::patterns::hierarchical` |
| `patterns/peer_to_peer.rs` | ~200 | `y-agent::agent::patterns::peer_to_peer` |
| `patterns/micro_pipeline.rs` | ~200 | `y-agent::agent::patterns::micro_pipeline` |

### 2.3 Cross-Crate Dependencies (Exact Import Sites)

**y-agent-core consumers**:

| Crate | File | Imports |
|-------|------|---------|
| `y-cli` | `commands/workflow.rs:8-9` | `y_agent_core::dag::TaskDag`, `y_agent_core::expression_dsl` |

> `y-service` does NOT import `y-agent-core` at all (confirmed via grep).

**y-multi-agent consumers**:

| Crate | File | Imports |
|-------|------|---------|
| `y-cli` | `commands/agent.rs:6-7` | `y_multi_agent::definition::{AgentDefinition, AgentMode, ContextStrategy}`, `y_multi_agent::TrustTier` |
| `y-service` | `container.rs:20` | `y_multi_agent::{AgentPool, AgentRegistry, MultiAgentConfig}` |

### 2.4 Naming Conflicts

| Conflict | y-agent-core | y-multi-agent | Resolution |
|----------|-------------|---------------|------------|
| `executor.rs` | `WorkflowExecutor` | `AgentExecutor` | 语义不同，各在其子模块内，无需重命名 |
| `TaskOutput` | `checkpoint::TaskOutput` | — | 保留原名（仅在 orchestrator 子模块内使用） |

> 两个 crate 无其他命名冲突。

---

## 3. Target Module Structure

```
crates/y-agent/
├── Cargo.toml
├── src/
│   ├── lib.rs                          # 统一入口，re-export 两个子模块的公共 API
│   │
│   ├── orchestrator/                   # 原 y-agent-core
│   │   ├── mod.rs
│   │   ├── dag.rs
│   │   ├── channel.rs
│   │   ├── checkpoint.rs
│   │   ├── executor.rs                 # WorkflowExecutor
│   │   ├── interrupt.rs
│   │   ├── expression_dsl.rs
│   │   ├── workflow_meta.rs
│   │   └── micro_pipeline.rs
│   │
│   └── agent/                          # 原 y-multi-agent
│       ├── mod.rs
│       ├── definition.rs
│       ├── registry.rs
│       ├── pool.rs
│       ├── delegation.rs
│       ├── executor.rs                 # AgentExecutor
│       ├── context.rs
│       ├── mode.rs
│       ├── dynamic_agent.rs
│       ├── gap.rs
│       ├── meta_tools.rs
│       ├── task_tool.rs
│       ├── trust.rs
│       ├── config.rs
│       ├── error.rs
│       └── patterns/
│           ├── mod.rs
│           ├── sequential.rs
│           ├── hierarchical.rs
│           ├── peer_to_peer.rs
│           └── micro_pipeline.rs
│
└── tests/
    └── integration.rs                  # 原 y-multi-agent/tests/integration.rs
```

---

## 4. Implementation Phases

### Phase 1: Create y-agent Crate Skeleton (Est. 0.5 day)

> **Goal**: 创建新 crate 的 Cargo.toml 和 lib.rs 骨架，合并两个旧 crate 的依赖。

| Task ID | Description | File |
|---------|-------------|------|
| I-MG-01 | 创建 `crates/y-agent/Cargo.toml`，合并两个旧 crate 的依赖 | `crates/y-agent/Cargo.toml` [NEW] |
| I-MG-02 | 创建 `crates/y-agent/src/lib.rs`，声明 `orchestrator` 和 `agent` 子模块 | `crates/y-agent/src/lib.rs` [NEW] |
| I-MG-03 | 更新 workspace `Cargo.toml`，添加 `y-agent` 成员 | `Cargo.toml` (root) |

#### Phase 1 验证

```bash
cargo check -p y-agent
```

---

### Phase 2: Migrate y-agent-core → y-agent::orchestrator (Est. 0.5 day)

> **Goal**: 将 y-agent-core 的 8 个模块移入 `orchestrator/` 子模块。

| Task ID | Description | File |
|---------|-------------|------|
| I-MG-04 | 创建 `src/orchestrator/mod.rs`，声明所有子模块并 re-export 公共 API | `src/orchestrator/mod.rs` [NEW] |
| I-MG-05 | 复制 8 个源文件到 `src/orchestrator/` | `dag.rs`, `channel.rs`, `checkpoint.rs`, `executor.rs`, `interrupt.rs`, `expression_dsl.rs`, `workflow_meta.rs`, `micro_pipeline.rs` |
| I-MG-06 | 更新模块内的 `crate::` 引用为 `crate::orchestrator::` 或相对路径 | 所有 orchestrator 模块 |

#### Phase 2 验证

```bash
# orchestrator 模块的所有测试通过
cargo test -p y-agent -- orchestrator
```

---

### Phase 3: Migrate y-multi-agent → y-agent::agent (Est. 1 day)

> **Goal**: 将 y-multi-agent 的 16+ 个模块移入 `agent/` 子模块。

| Task ID | Description | File |
|---------|-------------|------|
| I-MG-07 | 创建 `src/agent/mod.rs`，声明所有子模块并 re-export 公共 API | `src/agent/mod.rs` [NEW] |
| I-MG-08 | 复制所有源文件到 `src/agent/` 和 `src/agent/patterns/` | 所有 agent 模块 |
| I-MG-09 | 更新模块内的 `crate::` 引用为 `crate::agent::` 或相对路径 | 所有 agent 模块 |
| I-MG-10 | 迁移 `y-multi-agent/tests/integration.rs` 到 `y-agent/tests/integration.rs`，更新 `use` 路径 | `tests/integration.rs` |

#### Phase 3 验证

```bash
# agent 模块的所有测试通过
cargo test -p y-agent -- agent

# 集成测试
cargo test -p y-agent --test integration
```

---

### Phase 4: Update lib.rs Re-exports (Est. 0.5 day)

> **Goal**: 在 `lib.rs` 中建立便利的 re-export，保持消费端迁移的最小化。

| Task ID | Description | File |
|---------|-------------|------|
| I-MG-11 | 设计 re-export 策略：从 `y-agent` 根直接暴露常用类型 | `src/lib.rs` |
| I-MG-12 | Re-export orchestrator 高频 API：`TaskDag`, `expression_dsl`, `WorkflowExecutor` | `src/lib.rs` |
| I-MG-13 | Re-export agent 高频 API：`AgentDefinition`, `AgentMode`, `ContextStrategy`, `TrustTier`, `AgentPool`, `AgentRegistry`, `MultiAgentConfig` | `src/lib.rs` |

Re-export 策略示例：

```rust
// crates/y-agent/src/lib.rs

pub mod orchestrator;
pub mod agent;

// 便利 re-export (保持消费端最小改动)
pub use agent::config::MultiAgentConfig;
pub use agent::definition::{AgentDefinition, AgentMode, ContextStrategy};
pub use agent::pool::AgentPool;
pub use agent::registry::AgentRegistry;
pub use agent::trust::TrustTier;
```

#### Phase 4 验证

```bash
cargo test -p y-agent
cargo doc -p y-agent --no-deps
```

---

### Phase 5: Update Consumer Crates (Est. 1 day)

> **Goal**: 更新所有消费端的 Cargo.toml 和 `use` 路径。

| Task ID | Description | File |
|---------|-------------|------|
| I-MG-14 | **y-cli/Cargo.toml**: 移除 `y-agent-core` 和 `y-multi-agent`，添加 `y-agent` | `crates/y-cli/Cargo.toml` |
| I-MG-15 | **y-cli/commands/workflow.rs**: `use y_agent_core::` → `use y_agent::orchestrator::` | `workflow.rs:8-9` |
| I-MG-16 | **y-cli/commands/agent.rs**: `use y_multi_agent::` → `use y_agent::` (利用 re-export) | `agent.rs:6-7` |
| I-MG-17 | **y-service/Cargo.toml**: 移除 `y-multi-agent`，添加 `y-agent`（注意：y-service 不依赖 y-agent-core） | `crates/y-service/Cargo.toml` |
| I-MG-18 | **y-service/container.rs**: `use y_multi_agent::` → `use y_agent::` (利用 re-export) | `container.rs:20` |

#### 详细变更映射

**y-cli/commands/workflow.rs** (2 处):
```diff
-use y_agent_core::dag::TaskDag;
-use y_agent_core::expression_dsl;
+use y_agent::orchestrator::dag::TaskDag;
+use y_agent::orchestrator::expression_dsl;
```

**y-cli/commands/agent.rs** (2 处):
```diff
-use y_multi_agent::definition::{AgentDefinition, AgentMode, ContextStrategy};
-use y_multi_agent::TrustTier;
+use y_agent::agent::definition::{AgentDefinition, AgentMode, ContextStrategy};
+use y_agent::TrustTier;
```

**y-service/container.rs** (1 处):
```diff
-use y_multi_agent::{AgentPool, AgentRegistry, MultiAgentConfig};
+use y_agent::{AgentPool, AgentRegistry, MultiAgentConfig};
```

#### Phase 5 验证

```bash
# 全 workspace 编译
cargo build --workspace

# 所有测试
cargo test --workspace

# Clippy
cargo clippy --workspace -- -D warnings
```

---

### Phase 6: Remove Old Crates & Cleanup (Est. 0.5 day)

> **Goal**: 移除旧 crate 目录，更新 workspace 配置，更新设计文档引用。

| Task ID | Description | File |
|---------|-------------|------|
| I-MG-19 | 从 workspace `Cargo.toml` 移除 `y-agent-core` 和 `y-multi-agent` 成员 | `Cargo.toml` (root) |
| I-MG-20 | 删除 `crates/y-agent-core/` 目录 | 目录删除 |
| I-MG-21 | 删除 `crates/y-multi-agent/` 目录 | 目录删除 |
| I-MG-22 | 更新 `GEMINI.md` crate 列表 | `GEMINI.md` |
| I-MG-23 | 更新 `DESIGN_OVERVIEW.md` crate 索引 | `docs/design/DESIGN_OVERVIEW.md` |
| I-MG-24 | 更新 `multi-agent-design.md` 架构图中的 crate 引用 | `docs/design/multi-agent-design.md` |
| I-MG-25 | 更新 `MULTI_AGENT_AUTONOMY_PLAN.md` 中所有 `y-multi-agent` 和 `y-agent-core` 引用 | `docs/plan/MULTI_AGENT_AUTONOMY_PLAN.md` |
| I-MG-26 | 更新 `R&D_PLAN.md` 中的 crate 引用 | `docs/plan/R&D_PLAN.md` |
| I-MG-27 | 更新 `SKILLS_RND_PLAN.md` 中提及 `y-agent-core` 的地方 | `docs/plan/SKILLS_RND_PLAN.md` |

#### Phase 6 验证

```bash
# 确认旧 crate 不再存在
ls crates/y-agent-core && echo "ERROR: still exists" || echo "OK: removed"
ls crates/y-multi-agent && echo "ERROR: still exists" || echo "OK: removed"

# 全 workspace 编译
cargo build --workspace

# 全测试
cargo test --workspace

# Clippy 无警告
cargo clippy --workspace -- -D warnings

# 文档无警告
cargo doc --workspace --no-deps
```

---

## 5. Phase Dependencies

```
Phase 1 (Crate Skeleton)
  ↓
Phase 2 (Migrate orchestrator)  ←→  Phase 3 (Migrate agent)  [可并行]
  ↓                                    ↓
Phase 4 (Re-exports)
  ↓
Phase 5 (Update Consumers)
  ↓
Phase 6 (Remove Old Crates & Cleanup)
```

---

## 6. Risk Assessment

| 风险 | 概率 | 影响 | 缓解措施 |
|------|------|------|---------|
| `crate::` 内部引用遗漏 | 中 | 编译错误 | Phase 2-3 通过 `cargo check` 逐步修复 |
| 消费端遗漏导致编译失败 | 低 | 编译错误 | Phase 5 前已通过 grep 精确定位所有 4 处 import |
| 旧 crate 残留引用 | 低 | 编译错误 | Phase 6 通过 `cargo build --workspace` 和 `grep -r "y_agent_core\|y_multi_agent" crates/` 全量检查 |
| 设计文档引用遗漏 | 低 | 文档不一致 | Phase 6 通过 `grep -r "y-agent-core\|y-multi-agent" docs/` 全量检查 |
| 中断 `MULTI_AGENT_AUTONOMY_PLAN.md` 进行中的工作 | 中 | 计划失效 | 合并后立即更新 plan 中的 crate 引用和命令 |

---

## 7. Quality Gates

| Gate | Target | Command |
|------|--------|---------|
| 全 workspace 编译 | 0 errors | `cargo build --workspace` |
| 全测试通过 | 100% | `cargo test --workspace` |
| Clippy 无警告 | 0 warnings | `cargo clippy --workspace -- -D warnings` |
| 文档无警告 | 0 warnings | `cargo doc --workspace --no-deps` |
| 零残留引用 | 0 matches | `grep -r "y_agent_core\|y_multi_agent" crates/ --include='*.rs'` + `grep -r "y-agent-core\|y-multi-agent" crates/ --include='*.toml'` |

---

## 8. Estimated Timeline

| Phase | 内容 | 预计工作量 |
|-------|------|-----------|
| Phase 1 | Crate Skeleton | 0.5 天 |
| Phase 2 | Migrate orchestrator | 0.5 天 |
| Phase 3 | Migrate agent | 1 天 |
| Phase 4 | Re-exports | 0.5 天 |
| Phase 5 | Update Consumers | 1 天 |
| Phase 6 | Remove & Cleanup | 0.5 天 |
| **Total** | | **4 天** |

---

## 9. Impact on Other R&D Plans

| 计划 | 影响 | 动作 |
|------|------|------|
| `MULTI_AGENT_AUTONOMY_PLAN.md` | 所有 `y-multi-agent` crate 引用需更新为 `y-agent` | Phase 6 统一更新 |
| `SKILLS_RND_PLAN.md` | §7 "Future: Multi-Turn Agent Runner" 中提到 `y-agent-core` | Phase 6 统一更新 |
| `R&D_PLAN.md` | crate 列表需更新 | Phase 6 统一更新 |
| `GEMINI.md` | crate 列表需更新 | Phase 6 统一更新 |

---

## 10. Future Work (Post-Merge)

合并完成后，以下工作将在统一的 `y-agent` crate 中自然推进：

| 项目 | 描述 | 受益于合并 |
|------|------|-----------|
| **MultiTurnRunner** | Agent loop = DAG executor（每轮 LLM/tool call 是 DAG 节点 + checkpoint） | ✅ 可直接使用 `orchestrator::executor` + `orchestrator::checkpoint` |
| **统一 Agent Loop** | `loop.rs` — 基于 DAG 的 multi-turn 循环 | ✅ orchestrator 和 agent 在同一 crate |
| **SingleTurnRunner 迁移** | 从 `y-provider` 移入 `y-agent::agent::runner` | ✅ runner 与 agent 定义在同一 crate |
| **递归调用** | Agent → task tool → AgentPool.delegate → 同样的 AgentLoop | ✅ 无需跨 crate 编排 |

---

## 11. Acceptance Criteria

- [ ] `y-agent` crate 存在于 `crates/y-agent/`，包含 `orchestrator/` 和 `agent/` 两个子模块
- [ ] `y-agent-core/` 和 `y-multi-agent/` 目录已删除
- [ ] `cargo build --workspace` 零错误
- [ ] `cargo test --workspace` 所有测试通过（包括迁移后的 integration.rs）
- [ ] `cargo clippy --workspace -- -D warnings` 零警告
- [ ] `grep -r "y_agent_core\|y_multi_agent" crates/ --include='*.rs'` 零匹配
- [ ] `grep -r "y-agent-core\|y-multi-agent" crates/ --include='*.toml'` 零匹配
- [ ] `GEMINI.md` 和 `DESIGN_OVERVIEW.md` 已更新 crate 列表
- [ ] `MULTI_AGENT_AUTONOMY_PLAN.md` 中所有 crate 引用已更新
