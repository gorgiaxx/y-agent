# AgentFS 深度分析: 可观测性与文件操作回滚借鉴

**日期**: 2026-03-06
**分析对象**: [AgentFS](https://github.com/anthropics/agentfs) v0.6.2
**关注维度**: 可观测性、细粒度文件操作、文件级回滚恢复
**y-agent 相关文档**: `diagnostics-observability-design.md`, `micro-agent-pipeline-design.md`, `orchestrator-design.md`, `tools-design.md`, `runtime-design.md`

---

## 1. AgentFS 项目概要

AgentFS 是一个专门为 AI Agent 设计的文件系统，核心理念是将 agent 的所有文件操作存储在一个 SQLite 数据库中，从而获得 auditability（可审计）、reproducibility（可复现）和 portability（可移植）。

### 1.1 技术栈

| 层次 | 技术 | 说明 |
|------|------|------|
| 存储层 | SQLite (Turso/libSQL) | 所有文件、元数据、审计日志存储在单个 `.db` 文件 |
| 核心 SDK | Rust | inode-based VFS, OverlayFS, ToolCalls, KVStore |
| CLI | Rust (clap) | init, mount, run, sync, timeline 等命令 |
| Sandbox | Rust (reverie/ptrace) | Linux 上的 syscall 拦截，文件操作虚拟化 |
| 挂载后端 | FUSE (Linux), NFS (macOS) | 将 SQLite VFS 挂载为系统目录 |
| 多语言 SDK | TypeScript, Python, Go | 通过 SDK 直接操作 agent 文件系统 |

### 1.2 核心数据模型 (SPEC v0.4)

```
SQLite Database (.db)
├── fs_inode     -- 文件/目录元数据 (mode, size, timestamps, nlink)
├── fs_dentry    -- 目录项 (name → inode 映射)
├── fs_data      -- 文件内容 (4KB 分块存储)
├── fs_symlink   -- 符号链接目标
├── fs_config    -- 文件系统配置 (chunk_size 等)
├── fs_whiteout  -- OverlayFS 删除标记
├── fs_origin    -- OverlayFS copy-up 来源映射
├── kv_store     -- 键值存储 (agent 状态)
└── tool_calls   -- 工具调用审计日志 (insert-only)
```

### 1.3 关键架构决策

| 决策 | 理由 | 效果 |
|------|------|------|
| SQLite 作为文件系统后端 | 单文件可移植、ACID 事务、SQL 可查询 | 文件操作天然可审计、可快照 |
| inode-based 设计 | 支持 hard link、高效 rename、metadata 与 data 分离 | POSIX 兼容性好 |
| 4KB 分块存储 | 支持部分读写、避免大文件整体加载 | 内存效率高 |
| OverlayFS (COW) | 基于只读 base + 可写 delta 的分层 | 天然支持回滚 |
| Insert-only 审计日志 | 不可篡改的操作记录 | 完整的操作时间线 |

---

## 2. 可观测性分析

### 2.1 AgentFS 的可观测性机制

AgentFS 的可观测性相对轻量，分为三个层次:

#### 层次 1: Tool Call 审计日志 (结构化)

```sql
-- tool_calls 表: insert-only 审计日志
CREATE TABLE tool_calls (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  name TEXT NOT NULL,           -- 工具名 (read_file, web_search 等)
  parameters TEXT,              -- JSON 序列化的输入参数
  result TEXT,                  -- JSON 序列化的结果
  error TEXT,                   -- 错误信息
  status TEXT NOT NULL,         -- pending / success / error
  started_at INTEGER NOT NULL,  -- 开始时间戳
  completed_at INTEGER,         -- 完成时间戳
  duration_ms INTEGER           -- 执行耗时
);
```

Rust SDK 实现了完整的生命周期 API:
- `start(name, parameters) -> id`: 创建 pending 记录，返回 ID
- `success(id, result)`: 标记成功，计算 duration
- `error(id, error)`: 标记失败
- `record(...)`: 一次性插入已完成的记录 (spec-compliant)
- `recent(limit)`: 查询最近的调用
- `stats()` / `stats_for(name)`: 按工具聚合统计

CLI 提供 `agentfs timeline` 命令查看操作时间线 (表格或 JSON 格式)。

#### 层次 2: Strace-like 系统调用日志 (底层)

```rust
// sandbox/src/sandbox/mod.rs -- 可选的 syscall 级追踪
static STRACE_ENABLED: AtomicBool = AtomicBool::new(false);

// 每个 syscall 进入和返回都会 eprintln
// [pid] openat(AT_FDCWD, "/agent/file.txt", O_RDONLY)
// [pid] = 3
```

仅在 Linux sandbox 模式下可用，用于深度调试。

#### 层次 3: tracing::trace! (开发调试)

OverlayFS 模块使用 Rust `tracing` crate 的 `trace!` 宏记录操作，但仅限于该模块，未在 SDK 全局使用，也没有 span 层级结构。

### 2.2 与 y-agent 可观测性的对比

| 维度 | AgentFS | y-agent (observability v0.2) |
|------|---------|------|
| **数据模型** | 扁平的 tool_calls 表 | Trace → Observation 树 + Score |
| **存储** | SQLite (嵌入式) | PostgreSQL (共享基础设施) |
| **结构化程度** | 单层审计日志 | 多层嵌套 (parent_observation_id, depth, path) |
| **工具追踪** | name/params/result/error/duration | ObservationType 枚举 (13 种类型) |
| **评分体系** | 无 | Numeric/Categorical/Boolean + 多评分来源 |
| **重放支持** | 无 (但可通过数据库快照实现) | ReplayContext 保存完整上下文 |
| **查询能力** | SQL 直接查询 + CLI timeline | FTS、标签过滤、语义搜索 |
| **成本追踪** | 无 | total_tokens, total_cost, cost by model |
| **写入模型** | 同步直写 | 异步缓冲 (5s flush) |

### 2.3 可借鉴评估

**结论: 可观测性层面 y-agent 的设计已经远超 AgentFS，无需借鉴其模型。**

AgentFS 的 tool_calls 本质上是 y-agent Observation (obs_type = 'tool_call') 的一个极简子集。y-agent 已有的 Trace → Observation 树模型、多类型 Observation、Score 评分、ReplayContext 重放、FTS 搜索等能力都是 AgentFS 不具备的。

唯一值得注意的是 AgentFS 的 **"SQL 直接查询"** 范式: 因为所有数据都在 SQLite 中，用户可以直接写 SQL 进行即席分析。y-agent 基于 PostgreSQL 同样支持这一点，且 PostgreSQL 的 JSONB、GIN 索引、物化视图等能力更强。

---

## 3. 细粒度文件操作分析

### 3.1 AgentFS 的文件操作模型

AgentFS 实现了完整的 POSIX-like 文件系统，核心操作全部在 SQLite 事务中完成:

#### 分块存储 (Chunked Storage)

```
文件 "hello.txt" (8192 bytes) 在数据库中的存储:
  fs_inode: ino=42, mode=0o100644, size=8192, ...
  fs_data:  (ino=42, chunk_index=0, data=<4096 bytes>)
            (ino=42, chunk_index=1, data=<4096 bytes>)
```

这允许部分读写: `pread(offset=4000, size=200)` 只需读取 chunk_index=0，不需要加载整个文件。

#### 事务原子性

每个文件操作都在 `Transaction::new_unchecked(..., Immediate)` 中执行:

```rust
// sdk/rust/src/filesystem/agentfs.rs 模式
let txn = Transaction::new_unchecked(&conn, TransactionBehavior::Immediate).await?;
// 1. INSERT INTO fs_inode
// 2. INSERT INTO fs_dentry
// 3. UPDATE fs_inode SET nlink = nlink + 1
// 如果任何步骤失败:
txn.rollback().await;
// 全部成功:
txn.commit().await;
```

这保证了每个文件操作要么完全成功要么完全失败，不会出现半完成状态。

#### VFS 抽象层 (Sandbox)

Sandbox 通过 ptrace 拦截系统调用，将文件操作重定向到 SQLite VFS:

```
用户进程: open("/agent/file.txt", O_RDWR)
    ↓ ptrace 拦截
Sandbox: mount_table.resolve("/agent/file.txt")
    ↓ 匹配到 SqliteVfs
SqliteVfs: translate_to_relative → "/file.txt"
    ↓ resolve_path → ino=42
SqliteFileOps: 内存缓冲读写，fsync 时写回数据库
```

### 3.2 与 y-agent 文件操作的对比

| 维度 | AgentFS | y-agent (micro-agent-pipeline) |
|------|---------|------|
| **文件存储** | SQLite 虚拟文件系统 | 宿主文件系统 (真实文件) |
| **操作粒度** | POSIX syscall 级 (open/read/write/close) | 行级 (file_inspect/search_lines/read_range/patch) |
| **事务性** | SQLite ACID 事务 | file_patch 的 old/new 验证 (乐观锁) |
| **部分读写** | 4KB 分块 pread/pwrite | file_read_range (行范围读取) |
| **适用场景** | 沙盒内完全虚拟化的文件系统 | 直接操作宿主文件，LLM 友好的接口 |
| **设计哲学** | 数据库即文件系统 | 最小上下文、最大精度 |

### 3.3 可借鉴评估

**结论: y-agent 的 Atomic File Operations 在 LLM Agent 场景下比 AgentFS 的方案更合适。**

AgentFS 的细粒度体现在存储层 (4KB chunk)，面向的是传统程序的 POSIX I/O。而 y-agent 的 Atomic File Operations (file_inspect, file_search_lines, file_read_range, file_patch) 面向的是 LLM 的认知模式，token 效率和注意力聚焦是首要目标。这两者解决的是不同层面的问题。

不过，AgentFS 的 **分块存储思想** 在一个方面值得参考: 如果 y-agent 未来需要处理大文件 (>100KB)，可以考虑在 file_read_range 的实现中采用类似的分块缓存策略，避免每次都从磁盘读取整个文件再截取行范围。

---

## 4. 文件级回滚机制分析 (核心关注点)

这是本次分析的重点。y-agent 当前的 Orchestrator 有任务级 checkpoint 和 compensation 机制，但这些机制面向的是 **工作流状态恢复**（哪些任务已完成、channel 中的值），而非 **文件内容恢复**。当 agent 执行了 file_write 或 file_patch 修改了真实文件后，如果后续步骤失败需要回滚，文件内容无法自动恢复。

### 4.1 AgentFS 的回滚能力

AgentFS 通过三个机制实现文件回滚:

#### 机制 1: SQLite 事务回滚 (操作级)

每个文件操作在事务中执行，操作本身的原子性由 SQLite 保证。但这只保证单个操作不会半完成，不能回滚多个操作的组合。

#### 机制 2: OverlayFS Copy-on-Write (会话级)

这是 AgentFS 最核心的回滚能力:

```
               OverlayFS 分层模型
┌─────────────────────────────────────────┐
│           Delta Layer (可写)             │
│  SQLite DB: 修改后的文件、新建文件       │
│  fs_whiteout: 被删除的路径              │
│  fs_origin: copy-up 来源映射            │
└─────────────────────┬───────────────────┘
                      │ copy-on-write
┌─────────────────────┴───────────────────┐
│           Base Layer (只读)             │
│  宿主文件系统 或 另一个 SQLite DB        │
└─────────────────────────────────────────┘
```

工作原理:
1. **读取**: 先查 delta，delta 无则查 base (除非有 whiteout 标记)
2. **写入**: 首次修改时从 base copy-up 到 delta，后续写入只在 delta
3. **删除**: 在 delta 插入 whiteout 记录，阻止 base 层的可见性
4. **回滚**: 直接丢弃整个 delta 数据库，base 完好无损

```rust
// sdk/rust/src/filesystem/overlayfs.rs 核心逻辑

// 写入时的 copy-up
async fn copy_up(&self, path: &str, base_ino: i64) -> Result<i64> {
    // 1. 从 base 读取文件内容
    // 2. 在 delta 创建新文件
    // 3. 写入 fs_origin 记录 (delta_ino → base_ino)
    // 4. 后续操作全部在 delta
}

// 删除时的 whiteout
async fn unlink(&self, parent_ino: i64, name: &str) -> Result<()> {
    // 如果文件在 base 层:
    //   INSERT INTO fs_whiteout (path, created_at) VALUES (?, ?)
    // 如果文件在 delta 层:
    //   正常删除 delta 中的记录
}

// 查找时的分层查询
async fn lookup(&self, parent_ino: i64, name: &str) -> Result<Option<Stats>> {
    // 1. 查 delta → 找到则返回
    // 2. 查 whiteout → 标记删除则返回 None
    // 3. 查 base → 找到则返回 base 数据
    // 4. 返回 None
}
```

#### 机制 3: 数据库快照 (时间点级)

因为整个文件系统就是一个 SQLite 文件，快照就是 `cp agent.db snapshot.db`。SQLite WAL 模式保证拷贝过程中数据一致性。

### 4.2 y-agent 当前的回滚能力缺口

分析 y-agent 现有设计文档，回滚相关的机制如下:

| 机制 | 位置 | 恢复什么 | 不能恢复什么 |
|------|------|---------|------------|
| WorkflowCheckpoint | orchestrator-design.md | channel 值、任务完成状态 | 文件系统变更 |
| CompensationTask | orchestrator-design.md | 通过执行补偿任务撤销副作用 | 需要为每个文件操作编写对应的补偿逻辑 |
| file_patch dry_run | micro-agent-pipeline-design.md | 预览变更 (预防性) | 已执行的变更 |
| old/new 验证 | micro-agent-pipeline-design.md | 防止在错误状态上 patch (乐观锁) | 已成功执行的 patch |

**核心缺口**: 当 agent 通过 file_write 或 file_patch 修改了真实文件后，y-agent 没有内建机制将文件恢复到修改前的状态。CompensationTask 理论上可以，但需要:
1. 预先保存原始内容 (谁负责? 何时保存?)
2. 为每个文件操作定义补偿逻辑 (通用性差)
3. 补偿执行本身也可能失败

---

## 5. 借鉴方案: 文件操作日志与回滚 (FileJournal)

### 5.1 方案概述

**核心思想**: 借鉴 AgentFS OverlayFS 的 "base + delta" 分层思想，但适配 y-agent 直接操作宿主文件的架构。不是将文件系统虚拟化到 SQLite 中 (太重)，而是在工具执行层引入一个轻量级的 **FileJournal**，在文件被修改前自动记录原始内容，支持按任务/管道粒度回滚。

### 5.2 方案详细设计

#### 5.2.1 数据模型

```rust
/// 文件操作日志条目
struct FileJournalEntry {
    entry_id: u64,
    /// 关联的 Orchestrator 任务 ID 或 Pipeline ID
    scope_id: ScopeId,
    /// 操作类型
    op: FileOp,
    /// 文件绝对路径
    path: PathBuf,
    /// 操作前的文件内容快照 (仅在文件被修改/删除时记录)
    /// 对于新建文件, 此字段为 None
    original_content: Option<Vec<u8>>,
    /// 操作前的文件元数据 (权限、时间戳)
    original_metadata: Option<FileMetadata>,
    /// 时间戳
    created_at: Timestamp,
    /// 此条目是否已被回滚
    rolled_back: bool,
}

enum FileOp {
    Create,   // 新建文件
    Modify,   // 修改已有文件
    Delete,   // 删除文件
    Rename { from: PathBuf }, // 重命名/移动
}

enum ScopeId {
    Task(TaskId),           // Orchestrator 任务粒度
    Pipeline(PipelineId),   // 微代理管道粒度
    Manual(String),         // 手动标记的操作组
}
```

#### 5.2.2 工作流程

```
Agent 调用 file_write("src/main.rs", new_content)
    │
    ▼
ToolExecutor (已有, tools-design.md)
    │
    ├─── ToolMiddleware: pre_execute hook (已有, hooks-plugin-design.md)
    │       │
    │       ▼
    │    FileJournalMiddleware (新增):
    │       1. 检查操作是否涉及文件修改 (file_write, file_patch, shell_exec 等)
    │       2. 如果是, 且文件已存在:
    │          - 读取当前文件内容
    │          - 读取当前文件元数据
    │          - 写入 FileJournal (SQLite)
    │       3. 如果是新建文件:
    │          - 写入 FileJournal (op=Create, original_content=None)
    │
    ├─── 执行实际的文件操作
    │
    └─── post_execute hook: 记录操作结果到 Observation
```

#### 5.2.3 回滚流程

```
回滚触发 (任务失败 / 用户命令 / CompensationTask)
    │
    ▼
FileJournal.rollback(scope_id):
    1. 查询该 scope 下所有未回滚的条目, 按时间逆序
    2. 对每个条目:
       - Create → 删除文件 (如果仍存在且未被后续操作修改)
       - Modify → 写回 original_content, 恢复 metadata
       - Delete → 写回 original_content, 恢复 metadata
       - Rename → 移回原路径
    3. 标记所有条目为 rolled_back = true
    4. 返回回滚摘要 (成功/失败/跳过的条目)
```

#### 5.2.4 与现有模块的集成

| 集成点 | 方式 | 改动量 |
|--------|------|--------|
| **y-hooks (ToolMiddleware)** | 新增 `FileJournalMiddleware`，注册到 Tool 执行链的 pre_execute 位置 | 新增 middleware，不改已有代码 |
| **Orchestrator (CompensationTask)** | `FileJournal.rollback(task_id)` 作为内建的 compensation 策略 | Orchestrator 新增一个 `FailureStrategy::FileRollback` |
| **Micro-Agent Pipeline** | Pipeline 完成时检查是否需要提升日志到 LTM 或清理 | Pipeline 完成处理器新增可选步骤 |
| **Observability** | FileJournal 条目关联到对应的 Observation | Observation metadata 中记录 journal_entry_id |
| **CLI** | `y-agent journal list`, `y-agent journal rollback <scope_id>` | 新增 CLI 命令 |

#### 5.2.5 存储策略

```
FileJournal 存储选择: SQLite (与 Orchestrator checkpoint 共用)

理由:
1. Orchestrator 已经使用 SQLite 存储 checkpoint，FileJournal 可以作为同一数据库的新表
2. 文件内容作为 BLOB 存储，SQLite 处理效率可接受
3. 单文件部署，无额外依赖
4. 事务性: journal 写入和 checkpoint 可以在同一事务中，保证一致性

表结构:
CREATE TABLE file_journal (
    entry_id INTEGER PRIMARY KEY AUTOINCREMENT,
    scope_type TEXT NOT NULL,        -- 'task' | 'pipeline' | 'manual'
    scope_id TEXT NOT NULL,
    op TEXT NOT NULL,                -- 'create' | 'modify' | 'delete' | 'rename'
    path TEXT NOT NULL,
    rename_from TEXT,                -- 仅 rename 操作
    original_content BLOB,           -- 修改/删除前的文件内容
    original_mode INTEGER,           -- 文件权限
    original_mtime INTEGER,          -- 修改时间
    created_at INTEGER NOT NULL,
    rolled_back INTEGER DEFAULT 0
);

CREATE INDEX idx_journal_scope ON file_journal(scope_type, scope_id);
CREATE INDEX idx_journal_path ON file_journal(path);
```

#### 5.2.6 大文件处理

对于大文件 (>1MB)，直接存储完整内容在 SQLite 中效率不高。采用分级策略:

| 文件大小 | 存储方式 | 回滚方式 |
|---------|---------|---------|
| < 1MB | BLOB 直接存储在 SQLite | 从 journal 读取恢复 |
| 1MB - 50MB | 存储到 `.y-agent/journal/` 目录下的独立文件，journal 表记录文件路径 | 从独立文件恢复 |
| > 50MB | 仅记录元数据和文件 hash，不保存内容 | 提示用户手动恢复，或依赖 git |

#### 5.2.7 与 git 的协作

如果工作区是 git 仓库 (这在实际使用中非常常见)，FileJournal 可以利用 git:

```
FileJournalMiddleware.pre_execute:
    if workspace_is_git:
        // git 已经有完整的文件历史，不需要重复存储
        // 只记录元数据: 操作前的 git hash + 路径
        journal.record(scope_id, op, path, git_hash=HEAD, content=None)
    else:
        // 非 git 工作区，完整记录内容
        journal.record(scope_id, op, path, content=read_file(path))

FileJournal.rollback(scope_id):
    if workspace_is_git and all entries have git_hash:
        // 使用 git checkout 恢复文件
        for entry in entries.reverse():
            git checkout {entry.git_hash} -- {entry.path}
    else:
        // 使用 journal 中保存的内容恢复
        ...
```

### 5.3 方案理由 (详细论证)

#### 理由 1: 为什么不直接采用 AgentFS 的 OverlayFS 模式

AgentFS 的 OverlayFS 是将所有文件虚拟化到 SQLite 中，然后在此之上实现 COW。这对于 y-agent 不适用:

| 因素 | AgentFS OverlayFS | y-agent 实际场景 |
|------|-------------------|-----------------|
| 文件访问方式 | 所有进程通过 FUSE/NFS 挂载访问 | 工具直接调用 `std::fs` 操作宿主文件 |
| 平台支持 | FUSE 仅 Linux, NFS 方案性能差 | 需要跨平台 (macOS 为主要开发环境) |
| 与 IDE/编辑器的兼容性 | 挂载目录, IDE 可以访问 | 需要操作真实文件, IDE 实时看到变更 |
| 架构侵入性 | 需要运行 daemon, 管理挂载点 | y-agent 是库/CLI, 不应要求额外进程 |
| 性能 | 每个 syscall 经过 ptrace/FUSE 重定向 | 直接文件 I/O, 零开销 |

**结论**: 虚拟文件系统对 y-agent 的侵入性过大、平台兼容性差、与现有工具链冲突。FileJournal 借鉴了 OverlayFS 的 "记录原始状态，仅在需要时回滚" 的思想，但以更轻量的方式实现。

#### 理由 2: 为什么不用纯 git 方案

| 因素 | 纯 git 方案 | FileJournal + git 协作 |
|------|------------|----------------------|
| 非 git 工作区 | 不可用 | 仍然可用 (BLOB 存储) |
| 回滚粒度 | commit 级 (粗) | 任务/管道级 (细) |
| 未提交修改 | 需要 stash/创建临时 commit | 天然支持，不污染 git 历史 |
| 多文件原子回滚 | 需要 `git checkout HEAD -- file1 file2...` | 按 scope_id 批量回滚 |
| 与 Orchestrator 集成 | 需要额外的 commit/stash 管理逻辑 | 直接关联 TaskId |

**结论**: 纯 git 方案在 git 工作区内可行，但粒度不够细、不能覆盖非 git 场景。FileJournal 在 git 工作区中优先利用 git (避免重复存储)，在非 git 场景下自给自足。

#### 理由 3: 为什么选择 ToolMiddleware 而非工具内嵌

| 因素 | 工具内嵌 journal 逻辑 | ToolMiddleware |
|------|---------------------|----------------|
| 代码重复 | 每个文件操作工具都需要加 journal 代码 | 一处实现，全局生效 |
| 遗漏风险 | 新增工具可能忘记 journal | middleware 自动拦截所有文件操作工具 |
| 可选性 | 需要每个工具支持开关 | 一个 feature flag 控制 |
| 关注点分离 | 工具同时关心业务逻辑和回滚 | 工具只管业务，middleware 管回滚 |
| 与 y-hooks 设计一致性 | 违反 "Guardrails 作为 middleware" 原则 | 完全符合 hooks-plugin-design.md |

**结论**: ToolMiddleware 模式完全符合 y-agent 的分层架构原则 (工具管业务、hooks 管横切关注点)，且避免了代码重复和遗漏风险。

#### 理由 4: 为什么 FileJournal 存储在 SQLite 而非独立文件

| 因素 | 独立文件 (.y-agent/journal/*.bak) | SQLite 表 |
|------|----------------------------------|-----------|
| 原子性 | 文件拷贝无事务保证 | SQLite ACID |
| 与 checkpoint 一致性 | 需要额外协调 | 同一事务中写入 journal + checkpoint |
| 查询能力 | 需要自己实现索引 | SQL 查询 + 索引 |
| 清理 | 需要自己管理文件生命周期 | 与 checkpoint 一起清理 |
| 空间效率 | 每个文件一个备份 | SQLite BLOB + page 复用 |

**结论**: SQLite 与 Orchestrator checkpoint 共用存储是最自然的选择。

### 5.4 边界情况与风险

| 场景 | 处理方式 |
|------|---------|
| 回滚时文件已被第三方修改 | 检测 mtime/hash 变化，警告用户并提供选项 (强制覆盖 / 跳过 / 合并) |
| shell_exec 中的隐式文件修改 | 对 shell_exec，journal 只能记录工作目录快照或依赖 git diff；这是已知局限 |
| 并发文件操作 (多 agent) | journal 按 scope 隔离，但文件本身无锁；依赖文件系统级别的最终一致性 |
| journal 存储空间增长 | 保留策略: 完成的 pipeline 的 journal 在用户确认后清理 |
| 二进制文件 | 按文件大小策略处理；超大二进制文件只记录元数据 |

### 5.5 不建议借鉴的部分

| AgentFS 特性 | 不借鉴的理由 |
|-------------|-------------|
| **完整的 SQLite VFS** | 架构侵入性过大; y-agent 操作真实文件, 不需要虚拟文件系统 |
| **ptrace syscall 拦截** | 仅 Linux, 性能开销大, 与 y-agent 的 Docker/Native/SSH Runtime 设计冲突 |
| **FUSE/NFS 挂载** | 平台限制, 需要额外 daemon, 与 IDE 工作流不兼容 |
| **inode-based 数据模型** | 解决的是文件系统层面的问题 (hard link, 高效 rename), y-agent 工具层不需要这个抽象级别 |
| **KV Store** | y-agent 已有 3 层 Memory 架构 (Working / Short-Term / Long-Term), KV Store 是其子集 |

---

## 6. 次要借鉴: 工具调用审计的补充

### 6.1 方案概述

虽然 y-agent 的 Observation 模型已经覆盖了工具调用追踪，但 AgentFS 的 tool_calls 表有一个值得借鉴的特性: **独立于 Trace 的工具级聚合统计**。

y-agent 当前的统计需要从 Observation 表聚合 (WHERE obs_type = 'tool_call')，这在数据量大时可能较慢。可以考虑:

```sql
-- 在 observability schema 中新增物化视图
CREATE MATERIALIZED VIEW observability.tool_call_stats AS
SELECT
    (output->>'tool_name')::text AS tool_name,
    COUNT(*) AS total_calls,
    COUNT(*) FILTER (WHERE status = 'success') AS successful,
    COUNT(*) FILTER (WHERE status = 'failed') AS failed,
    AVG(duration_ms) AS avg_duration_ms,
    PERCENTILE_CONT(0.95) WITHIN GROUP (ORDER BY duration_ms) AS p95_duration_ms,
    SUM(cost) AS total_cost
FROM observability.observations
WHERE obs_type = 'tool_call'
  AND started_at > NOW() - INTERVAL '30 days'
GROUP BY (output->>'tool_name')::text;
```

### 6.2 理由

这不是必需的，但有两个好处:
1. 快速回答 "哪个工具最慢/最贵/失败率最高" 类查询
2. 与 AgentFS 的 `agentfs timeline` 类似的快速概览能力

由于 y-agent 已经有物化视图 `trace_stats`，新增一个 `tool_call_stats` 的成本很低。

---

## 7. 总结

| 维度 | 借鉴程度 | 具体行动 |
|------|---------|---------|
| **文件级回滚** | 深度借鉴 OverlayFS 的 "记录原始状态" 思想 | 设计 FileJournal 机制，作为 ToolMiddleware 集成 |
| **可观测性** | 不需要借鉴 | y-agent 已有更完善的 Trace/Observation/Score 模型 |
| **细粒度文件操作** | 不需要借鉴 | y-agent 的 Atomic File Operations 更适合 LLM Agent |
| **工具聚合统计** | 轻度借鉴 | 新增一个 tool_call_stats 物化视图 |
| **SQLite VFS / Sandbox** | 不借鉴 | 架构不兼容，侵入性过大 |

**FileJournal 的优先级建议**: P1。这是 y-agent 当前设计中为数不多的安全缺口: agent 修改了文件后无法自动恢复。在 agent 操作越来越自动化的趋势下，文件回滚能力是可靠性的基本保障。建议在 Micro-Agent Pipeline Phase 2 (Atomic File Operations) 实现时同步引入 FileJournal。
