# OpenSandbox 项目分析：y-agent Runtime 设计借鉴

> 对 OpenSandbox (alibaba/OpenSandbox) 的架构进行深度分析，评估其对 y-agent Runtime 模块的借鉴价值。

**分析日期**: 2026-03-06
**分析对象**: [OpenSandbox](https://github.com/alibaba/OpenSandbox) — 阿里巴巴开源的 AI 应用沙箱平台
**关联文档**: `docs/design/runtime-design.md`, `docs/design/runtime-tools-integration-design.md`

---

## 1. OpenSandbox 项目概览

OpenSandbox 是一个面向 AI 应用场景的通用沙箱平台。它为 AI Agent（Claude Code、Gemini CLI、Codex 等）提供隔离的代码执行环境，核心能力包括：

- **多语言 SDK**：Python、Java/Kotlin、JavaScript/TypeScript、C#/.NET
- **标准化协议**：OpenAPI 规范定义生命周期 API 和执行 API
- **多运行时后端**：Docker（生产就绪）、Kubernetes（生产就绪，含 BatchSandbox CRD）
- **安全容器支持**：gVisor、Kata Containers (QEMU/Firecracker/CLH)
- **网络隔离**：基于 FQDN 的出站流量控制（DNS 代理 + nftables）

### 1.1 四层架构

```
SDKs Layer          → 客户端库，面向开发者
Specs Layer         → OpenAPI 规范，定义协议契约
Runtime Layer       → FastAPI 服务，沙箱生命周期编排
Sandbox Instances   → 容器 + 注入的 execd 守护进程
```

### 1.2 核心组件

| 组件 | 语言 | 职责 |
|------|------|------|
| **Server** | Python (FastAPI) | 沙箱生命周期管理、运行时编排 |
| **execd** | Go (Gin) | 容器内执行守护进程，提供代码执行 / Shell / 文件操作 / 指标采集 |
| **Egress Sidecar** | Go | 网络出站控制，DNS 代理 + nftables |
| **Ingress Proxy** | Go | 入站路由代理，支持 HTTP/WebSocket |
| **K8s Controller** | Go (controller-runtime) | BatchSandbox/Pool CRD 控制器 |

### 1.3 问题域差异

| 维度 | OpenSandbox | y-agent Runtime |
|------|-------------|-----------------|
| **定位** | 平台服务，服务于多个外部 Agent | 框架内部模块，服务于自身 Tool/Skill |
| **进程模型** | 多进程（Server + SDK 客户端） | 单进程（in-process 调用） |
| **沙箱生命周期** | 长期存活，带 TTL 续期 | 短期存活，命令执行后销毁 |
| **用户交互** | SDK API，远程调用 | RuntimeAdapter trait，本地函数调用 |
| **语言** | Python + Go | Rust |

---

## 2. 借鉴项详细方案与论证

### 2.1 容器池化与 `docker exec` 复用

#### 2.1.1 要解决的问题

y-agent 当前的 Docker 执行流程是严格的一次性模型：

```
container create → start → wait → logs → stats → remove
```

`runtime-design.md` 明确记录了这个流程的性能数据：

| 步骤 | 耗时 |
|------|------|
| Container create + start (warm image) | < 1.5s |
| Container create + start (cold pull) | < 30s |
| Container remove | < 500ms |

而 y-agent 全局的性能目标是 **P95 tool dispatch < 100ms**。一次容器化工具调用的最低开销是 1.5s + 0.5s = 2s（不含执行本身），是目标值的 20 倍。

同时 `runtime-tools-integration-design.md` 中的性能数据也确认了这个矛盾：

| Pattern | Overhead |
|---------|----------|
| Container (warm image) | ~1.5s |
| Skill container (warm) | ~2s |
| NativeRuntime (plain) | < 50ms |

当前设计对此的回应只有一行文字："Warm container pool: Pre-pull frequently used images; keep container templates ready"，没有任何具体机制。

#### 2.1.2 OpenSandbox 如何解决

OpenSandbox 通过两种互补机制解决这个问题：

**机制 A：K8s Pool CRD（预创建 + 自动伸缩）**

```yaml
capacitySpec:
  bufferMax: 10   # 空闲沙箱上限
  bufferMin: 2    # 空闲沙箱下限，低于此值自动补充
  poolMax: 20     # 总沙箱上限
  poolMin: 5      # 总沙箱下限
```

控制器持续维护池中的空闲实例数在 `[bufferMin, bufferMax]` 范围内。创建请求到来时，从池中取一个已经 Running 的 Pod，几乎零延迟。

**机制 B：execd 注入 + 容器长期存活**

容器不是每次命令创建/销毁的，而是创建一次后长期存活。多次命令通过容器内的 execd HTTP API 执行。每个容器带 TTL 和过期计时器，Server 重启时从容器标签恢复计时器状态。

OpenSandbox 的性能数据：

| Runtime | Cold Start | Warm Start (from Pool) |
|---------|-----------|------------------------|
| runc | ~500ms | ~50ms |
| gVisor | ~550ms | ~100ms |

#### 2.1.3 y-agent 的具体方案

**不照搬 OpenSandbox 的完整 execd 模式**（理由见 3.2 节），而是组合两个更轻量的机制：

**机制 1：docker exec 复用（替代 execd）**

对于需要容器隔离的工具，不再"创建→执行→销毁"，而是：

1. 获取一个已运行的容器（从池中或新建）
2. 通过 `docker exec` 在该容器内执行命令
3. 收集 stdout/stderr 和 exit code
4. 容器保持运行，等待下次使用

这比 execd 注入简单得多——不需要容器内守护进程，不需要 HTTP API，Docker Engine 原生支持 `exec`。`RuntimeAdapter::execute()` 的实现从 `create → start → wait → remove` 变为 `exec_create → exec_start → exec_inspect`。

`docker exec` 的开销约为 50-100ms（进程创建 + namespace 加入），在 P95 < 100ms 的目标范围内。

**风险**：容器复用引入状态泄漏问题——前一次执行留下的文件、环境变量、进程可能影响后续执行。

**缓解**：
- 每次 exec 前重置工作目录内容（或使用 tmpfs 挂载）
- 环境变量通过 `docker exec -e` 每次显式传入，不依赖容器级 env
- 对安全要求高的工具（如 shell_exec），仍然走一次性容器模式（通过 ToolManifest 中的 `require_fresh_container: true` 声明）

**机制 2：per-image 容器池**

```
ContainerPool {
    pools: HashMap<ImageTag, PoolState>,
    config: PoolConfig,
}

PoolState {
    idle: VecDeque<ContainerId>,     // 空闲容器队列
    active: HashSet<ContainerId>,    // 正在执行的容器
    total_created: usize,
}

PoolConfig {
    min_idle: usize,       // 每个 image 的最小空闲数（类似 bufferMin）
    max_idle: usize,       // 最大空闲数（类似 bufferMax）
    max_total: usize,      // 全局容器总上限
    idle_ttl: Duration,    // 空闲容器超时时间
    images: Vec<String>,   // 需要池化的镜像列表（从 ImageWhitelist 推导）
}
```

后台 tokio task 定期检查：
- `idle.len() < min_idle` → 异步创建新容器补充
- `idle.len() > max_idle` → 移除多余容器
- 空闲时间超过 `idle_ttl` → 销毁

`DockerRuntime::execute()` 的获取路径变为：`pool.try_acquire(image)` → 命中则 `docker exec` → 未命中则降级为 `create + start + wait + remove`。

#### 2.1.4 论证：为什么应该做

**正面理由**：

1. **性能目标的硬约束**：P95 < 100ms 与 Container create ~1.5s 之间存在结构性矛盾。没有池化，要么放弃这个目标（对容器化工具），要么所有工具都走 NativeRuntime（失去隔离保证）。池化是唯一同时满足性能和安全的方案。

2. **Agent 调用模式的必然要求**：一个典型的 Agent 会话中，`shell_exec`、`python_exec` 这类需要容器的工具可能被调用几十次。每次 2s 的开销意味着 60 秒的会话中有 40 秒花在容器创建/销毁上。

3. **实现成本可控**：核心是一个 `HashMap<String, VecDeque<String>>` 加一个后台补充 task。`docker exec` 是 Docker Engine 原生能力，不需要额外组件。

**反面风险**：

1. **状态泄漏**：容器复用最大的风险。缓解手段是 tmpfs 工作目录 + 显式环境变量 + 安全敏感工具走一次性模式。不是完美方案，但 trade-off 合理。

2. **资源占用**：预创建的空闲容器消耗内存（即使容器内没有活跃进程，runc 容器本身约 5-10MB 开销）。10 个空闲容器 ≈ 100MB。可以接受。

3. **复杂度增加**：池管理逻辑（补充、回收、TTL、容量上限）比当前的简单 create/remove 复杂。但这是可控的工程复杂度，不是架构复杂度。

#### 2.1.5 论证：为什么可能不做

1. **如果 y-agent 的主要工具都走 NativeRuntime**（本地执行 + bubblewrap 沙箱），那么只有少数工具需要容器，池化的收益有限。但这意味着放弃了容器级隔离的安全保证。

2. **如果 P95 < 100ms 的目标只针对 NativeRuntime 工具**，容器工具接受更高的延迟（如 P95 < 2s），那池化不是强需求。但这需要明确修改性能目标的定义。

---

### 2.2 安全容器运行时抽象

#### 2.2.1 要解决的问题

y-agent `runtime-design.md` 的 Alternatives 表中已经对比了 Docker/Firecracker/WASM 三种隔离方案，最终选择了 Docker。表中记录了关键差异：

| | Docker | Firecracker | WASM |
|-|--------|-------------|------|
| Isolation | Container (cgroups/namespaces) | MicroVM (hardware) | Sandbox (WASI) |

但标准 Docker 容器（runc）的隔离是**共享宿主机内核**的——容器内进程直接调用宿主机内核的 syscall。这意味着内核漏洞（如 CVE-2022-0185, CVE-2024-1086 等）可以被利用来逃逸容器。

对于 AI Agent 执行任意 LLM 生成的代码这个场景，威胁等级是高的：LLM 可能被 prompt injection 攻击诱导生成恶意代码，或者 LLM 自身可能因为训练数据中包含恶意样本而生成危险代码。

当前设计中 Non-Goals 明确写了："Not a general virtualization layer (Docker OCI only; no VMs)"。但 gVisor 不是 VM——它是一个用户态内核，仍然通过标准的 Docker OCI 接口使用，只是用 `--runtime=runsc` 替代默认的 `--runtime=runc`。

#### 2.2.2 OpenSandbox 如何解决

OpenSandbox 的 `SecureRuntimeResolver` 做了一件很小但很关键的事：

```python
class SecureRuntimeResolver:
    DEFAULT_DOCKER_RUNTIMES = {
        "gvisor": "runsc",
        "kata": "kata-runtime"
    }

    def get_docker_runtime(self) -> Optional[str]:
        if not self.secure_runtime or not self.secure_runtime.type:
            return None  # 使用默认 runc
        return self.secure_runtime.docker_runtime
```

本质上就是：**配置 → Docker `--runtime` 参数的映射**。加上启动时验证（检查 Docker daemon 是否安装了该 runtime），整个方案不到 50 行代码。

OSEP-0004 的核心设计决策是**服务器级配置**：管理员选择一次，所有沙箱透明使用。SDK/API 层完全无感知。这避免了"每个工具调用都要指定 runtime"的复杂度。

#### 2.2.3 y-agent 的具体方案

在 `DockerRuntime` 的配置中增加一个可选字段：

```rust
pub struct DockerRuntimeConfig {
    // ... 现有字段 ...
    pub secure_runtime: Option<SecureRuntimeConfig>,
}

pub struct SecureRuntimeConfig {
    pub runtime_type: SecureRuntimeType,  // gVisor | Kata | Firecracker
    pub docker_runtime: String,            // 映射到 Docker --runtime 参数
}

pub enum SecureRuntimeType {
    None,
    GVisor,
    Kata,
    Firecracker,
}
```

对应的 TOML 配置：

```toml
[runtime.docker]
# ... 现有配置 ...

[runtime.docker.secure_runtime]
type = "gvisor"          # "", "gvisor", "kata", "firecracker"
docker_runtime = "runsc"  # Docker --runtime 参数值
```

代码变更极小——只需要在 `DockerRuntime::execute()` 中构建容器配置时，将 `self.config.secure_runtime.docker_runtime` 传递给 Docker API 的 `HostConfig.Runtime` 字段。

启动时验证：`DockerRuntime::new()` 中调用 Docker `info()` API，检查 `Runtimes` 列表中是否包含配置的 runtime 名称。不包含则 panic with 清晰错误信息。

#### 2.2.4 论证：为什么应该做

**正面理由**：

1. **实现成本极低**：核心变更是给 Docker container create 调用加一个 `runtime` 参数。不改变 `RuntimeAdapter` trait，不改变 `RuntimeContext`，不改变任何调用方代码。大约 20-30 行 Rust 代码。

2. **安全提升是本质性的**：gVisor 拦截所有 syscall 并在用户态重新实现内核功能。即使宿主机内核存在漏洞，容器内代码也无法直接触达该漏洞。这不是"多一层防御"，而是隔离模型的根本变化。

3. **与现有设计完全兼容**：`runtime-design.md` 的 Non-Goals 说"Not a general virtualization layer (Docker OCI only; no VMs)"。gVisor 完全符合这个约束——它就是一个 Docker OCI runtime，用法和 runc 完全相同，只是隔离更强。Kata/Firecracker 是 VM，但它们也通过 OCI 接口暴露，从 Docker API 调用者视角没有区别。

4. **OpenSandbox 的性能数据证明 gVisor 开销可接受**：gVisor 冷启动 ~550ms（vs runc ~500ms），增加了 ~50ms。如果搭配容器池化，预热启动 ~100ms（vs runc ~50ms），增加也只有 ~50ms。

5. **y-agent 已经在 Alternatives 表中考虑了 Firecracker**，说明安全容器运行时的需求在设计时就已经被认识到。现在是给一个明确的低成本路径。

**反面风险**：

1. **gVisor 兼容性问题**：gVisor 不实现所有 Linux syscall。一些工具可能在 gVisor 下失败（比如依赖 `io_uring`、`ptrace` 等高级 syscall 的程序）。但 y-agent 的工具主要是 shell 命令和脚本执行，兼容性风险低。

2. **运维前提**：需要宿主机安装 gVisor runsc。这增加了部署要求。但作为可选配置（`type = ""`），不影响不需要安全容器的部署场景。

3. **macOS 不支持**：gVisor 和 Kata 都是 Linux 专属。macOS 开发环境只能用默认 runc。这没有问题——`SecureRuntimeConfig` 是可选的，开发环境不配置即可。

#### 2.2.5 论证：为什么可能不做

1. **如果 y-agent 始终在个人研究环境中运行**（可信环境，自己写的 prompt），容器逃逸威胁等级低，gVisor 不是刚需。但 VISION.md 提到了"自演化"能力，未来可能执行非预期代码。

2. **如果开发资源极度有限**，这 20-30 行代码本身不是负担，但测试验证（在 gVisor 环境下运行所有工具的兼容性测试）需要额外的 CI 环境搭建。

---

### 2.3 FQDN 级网络出站控制

#### 2.3.1 要解决的问题

y-agent 的 `NetworkCapability` 枚举已经设计了四个级别：

```rust
// None | Internal(CIDRs) | External(domains) | Full
```

其中 `External(domains)` 明确表达了"允许访问特定域名"的语义。但从设计文档到实际实施之间存在巨大鸿沟——`runtime-design.md` 的 Container Security Configuration 表中，网络隔离的实际操作只有：

```
Network mode | None | NetworkCapability != None
```

也就是说，只要 Tool 声明了任何网络需求，容器就获得 Bridge 模式的完整网络访问。`External(domains)` 和 `Full` 在实施层面没有区别。

这不是疏忽——Docker 本身不支持基于域名的网络策略。Docker 的 `--network` 只有 none/bridge/host/container 四种模式。bridge 模式下容器可以访问任何外部地址。

#### 2.3.2 OpenSandbox 如何解决

OSEP-0001 的方案包含三个层次：

**Layer 1：DNS 代理（Go 实现，~500 行代码）**
- Sidecar 容器内运行 DNS 代理，监听 127.0.0.1:15353
- iptables REDIRECT 将所有 DNS 查询（端口 53）转发到 15353
- 代理根据策略决定：允许 → 转发到上游 DNS 并"学习"返回的 IP；拒绝 → 返回 NXDOMAIN
- 学到的 IP 通知 Layer 2

**Layer 2：nftables 过滤（Go 实现，~300 行代码）**
- 创建 nftables 表，output chain 默认 DROP
- 允许 localhost、已建立连接、DNS proxy 学到的 IP
- 阻止 DoH（443 端口上的已知 DoH 提供商）和 DoT（853 端口）

**部署拓扑**：
- Sidecar 容器带 `CAP_NET_ADMIN`
- 应用容器通过 `--network container:<sidecar>` 共享网络命名空间
- 应用容器不获得任何额外权限

#### 2.3.3 y-agent 的具体方案

**方案 A：直接集成 OpenSandbox 的 egress sidecar 镜像**

OpenSandbox 的 egress sidecar 是独立的 Go 二进制，通过环境变量 `OPENSANDBOX_EGRESS_RULES` 接收策略。y-agent 可以直接使用这个镜像：

1. `DockerRuntime::execute()` 检测到 `NetworkCapability::External(domains)` 时
2. 先启动 egress sidecar 容器（传入策略 JSON）
3. 等待 sidecar healthcheck 通过
4. 启动应用容器，`--network container:<sidecar_id>`
5. 执行完毕后清理两个容器

优点：零开发成本，直接复用成熟实现。
缺点：引入 Go 二进制依赖；sidecar 镜像需要加入 ImageWhitelist。

**方案 B：Rust 原生实现**

用 Rust 重写 DNS 代理 + nftables 过滤逻辑（约 800-1000 行 Rust 代码），编译为静态二进制，通过 volume mount 注入到 sidecar 容器中（使用最小基础镜像如 `scratch` 或 `alpine`）。

优点：全 Rust 工具链；更轻量。
缺点：开发成本高；需要实现 DNS 协议解析和 nftables 交互。

**方案 C：仅实现 Layer 1（DNS-only 模式）**

只做 DNS 代理过滤，不做 nftables 网络层过滤。这意味着直接 IP 访问可以绕过策略，但 DNS 级过滤已经覆盖了大多数正常使用场景（程序通常通过域名访问外部服务）。

优点：实现简单（仅 DNS 代理，~300 行代码）。
缺点：安全性不完整——恶意代码可以通过硬编码 IP 绕过。

**推荐：方案 A（短期）→ 方案 B（长期）**，或者如果安全要求不那么严格，方案 C 作为 MVP。

#### 2.3.4 论证：为什么应该做

**正面理由**：

1. **填补设计与实施之间的空白**：`NetworkCapability::External(domains)` 是一个已经存在的设计抽象，但没有实施路径。不实现它，等于 capability 模型在网络维度只有两个有效值：None 和 Full。声明了 `External(["api.github.com"])` 的工具实际上获得了 Full 网络访问，这违反了"每个执行单元只获得 Manifest 中声明的能力"这个核心安全承诺。

2. **AI Agent 的网络控制是真实需求**：一个 `web_search` 工具需要访问搜索引擎 API，但不应该能访问内网；一个 `pip_install` 工具需要 `pypi.org` 但不应该能访问 `evil.com`。当前的设计无法表达这种区分。

3. **与 ToolManifest 的 capability 声明模型天然衔接**：Tool 已经声明了 `NetworkCapability::External(domains)`，所有声明侧的工作已经完成。缺的只是实施侧。

**反面风险**：

1. **实现复杂度高**：Sidecar 模式涉及多容器编排（sidecar 启动 → healthcheck → 应用容器启动 → 执行 → 双容器清理）。比单容器模型复杂得多。

2. **每次执行多一个容器的开销**：sidecar 容器本身有启动时间。如果搭配容器池化（sidecar 也池化），可以缓解。

3. **Linux 专属**：iptables/nftables 是 Linux 内核功能。macOS 开发环境不支持（macOS 的 Docker Desktop 运行在 Linux VM 中，可能部分支持）。

4. **DNS-only 模式的安全假象**：如果只实现 Layer 1（DNS 代理），用户可能误以为获得了完整的网络隔离，但实际上直接 IP 访问可以绕过。这比"不实现"更危险——因为给了虚假的安全感。

#### 2.3.5 论证：为什么可能不做

1. **如果大多数工具声明 `NetworkCapability::None`**（容器内纯计算，不需要网络），那么精细的网络控制使用场景有限。可以先保持 None/Bridge 两级控制。

2. **实现成本与收益不对称**：这是五项借鉴中实现成本最高的（多容器编排 + DNS 代理 + 网络过滤），但使用场景可能最窄（只有需要外部网络的容器化工具才触发）。

3. **替代方案**：如果不做 FQDN 级控制，可以通过更严格的 ToolManifest 审核来管控风险——只给可信工具开 `NetworkCapability != None`，把安全责任上移到 Manifest 审核层面。

---

### 2.4 优雅降级与能力探测框架

#### 2.4.1 要解决的问题

y-agent 的 `runtime-design.md` Failure Handling 表列举了 12 种故障场景，降级策略散落在各个条目中：

| 场景 | 当前策略 |
|------|---------|
| Docker daemon unavailable | Fall back to NativeRuntime if allowed |
| bubblewrap not available | Fall back to plain execution; warn |
| Image digest mismatch | Reject execution |
| Container OOM killed | Return ResourceLimitExceeded |

问题不在于各个策略本身（它们是合理的），而在于缺少一个**统一的框架**来回答以下问题：

- 启动时，当前环境的实际隔离能力是什么？
- 一个声明需要容器隔离的工具，在 Docker 不可用的环境中，应该降级执行还是拒绝执行？由谁决定？
- 降级行为是否被记录和可观测？用户/管理员是否知道当前处于降级状态？

#### 2.4.2 OpenSandbox 如何解决

OpenSandbox 的 egress 组件定义了清晰的三级模式：

```go
type EnforcementMode int
const (
    ModeDisabled   EnforcementMode = iota
    ModeDNSOnly
    ModeNftables
)
```

启动时通过 `probeCapabilities()` 确定当前环境的实际能力。同时提供了 `require_full_isolation` 旗帜——允许用户声明"我不接受降级，如果达不到完全隔离就失败"。

这个模式的价值在于：**将隐式的降级行为变成显式的、可观测的、可配置的**。

#### 2.4.3 y-agent 的具体方案

定义一个 `RuntimeCapabilities` 结构体，在 `RuntimeManager::new()` 时通过探测填充：

```rust
pub struct RuntimeCapabilities {
    pub docker_available: bool,
    pub docker_secure_runtime: Option<SecureRuntimeType>,
    pub bubblewrap_available: bool,
    pub firejail_available: bool,
    pub nftables_available: bool,
    pub max_isolation: IsolationLevel,
}

pub enum IsolationLevel {
    SecureContainer,   // Docker + gVisor/Kata
    Container,         // Docker + runc
    Sandbox,           // NativeRuntime + bubblewrap
    Process,           // NativeRuntime plain
}
```

探测逻辑在 `RuntimeManager::new()` 中执行一次：

```rust
impl RuntimeManager {
    pub async fn new(config: RuntimeConfig) -> Result<Self> {
        let capabilities = Self::probe_capabilities(&config).await;
        info!("Runtime capabilities: max_isolation={:?}, docker={}, bwrap={}",
              capabilities.max_isolation,
              capabilities.docker_available,
              capabilities.bubblewrap_available);
        // ...
    }
}
```

然后在 `execute()` 路径中：

```rust
// ToolManifest 声明了 min_isolation: Container
// 但当前环境 max_isolation = Sandbox (Docker 不可用)

match config.on_insufficient_isolation {
    DegradationPolicy::Reject => {
        return Err(RuntimeError::InsufficientIsolation {
            required: IsolationLevel::Container,
            available: capabilities.max_isolation,
        })
    }
    DegradationPolicy::Degrade => {
        warn!("Tool '{}' requires {:?} isolation but only {:?} available, degrading",
              tool_name, required, available);
        audit.log_degradation(tool_name, required, available);
        // 使用 NativeRuntime + bubblewrap
    }
}
```

`DegradationPolicy` 由全局配置控制（类似 OpenSandbox 的 `require_full_isolation`）。

#### 2.4.4 论证：为什么应该做

**正面理由**：

1. **成本极低，收益清晰**：核心是一个枚举 + 启动时探测 + 执行时 match。大约 50-80 行代码。但它将所有散落的降级逻辑统一到一个可审计的框架中。

2. **直接对应设计原则**：`CLAUDE.md` 明确要求"Fail Fast, Recover Cheap"和"Defense in Depth"。统一降级框架是"Fail Fast"的具体体现——在启动时就告诉你环境能力，而不是在执行时才发现 Docker 不可用。

3. **运维可观测性**：`runtime.max_isolation_level` 作为一个 gauge metric 暴露，管理员立刻知道当前环境的安全状态。

4. **为 2.2 节（安全容器运行时）提供基础**：`IsolationLevel::SecureContainer` 比 `IsolationLevel::Container` 高。如果工具声明需要 `SecureContainer` 级别，但 gVisor 未安装，框架自动拒绝或降级。

**反面风险**：

几乎没有。这是一个纯粹的代码组织优化，不改变任何行为语义。唯一的风险是过度设计——如果实际上只有两个运行时后端（Docker 和 Native），四级枚举可能显得多余。但枚举扩展成本为零。

#### 2.4.5 论证：为什么可能不做

1. **当前散点式策略实际上能工作**：每个故障场景都有明确的处理方式。统一框架的价值更多是工程优雅性而非功能必要性。

2. **如果 y-agent 始终在完整环境中部署**（Docker 可用、bubblewrap 可用），降级路径永远不会触发。框架成为永远执行不到的代码。

---

### 2.5 生命周期与执行接口分离

#### 2.5.1 要解决的问题

当前 `RuntimeAdapter` trait 包含 6 个方法：

```rust
pub trait RuntimeAdapter: Send + Sync {
    fn name(&self) -> &str;
    async fn execute(&self, ctx: &RuntimeContext, command: &Command) -> Result<ExecutionResult>;
    async fn spawn(&self, ctx: &RuntimeContext, command: &Command) -> Result<ProcessHandle>;
    async fn kill(&self, handle: &ProcessHandle) -> Result<()>;
    async fn status(&self, handle: &ProcessHandle) -> Result<ProcessStatus>;
    async fn health_check(&self) -> Result<HealthStatus>;
    async fn cleanup(&self, ctx: &RuntimeContext) -> Result<()>;
}
```

这里混合了两种关注点：
- **命令执行**：`execute`、`spawn`、`kill`、`status` — 在一个已就绪的运行时环境中执行命令
- **生命周期管理**：`health_check`、`cleanup` — 运行时环境本身的管理

如果引入容器池化（2.1 节），这个混合会变得更严重——池的管理逻辑（创建容器、回收容器、TTL 检查、容量伸缩）应该放在哪里？放在 `RuntimeAdapter` 实现内部，会导致 `DockerRuntime` 膨胀为一个包含执行逻辑 + 池管理逻辑 + 安全检查逻辑的巨型结构体。

#### 2.5.2 OpenSandbox 如何分离

OpenSandbox 通过**两套独立的 API 规范**实现了彻底分离：

- **Lifecycle API**（Server 实现）：create、delete、pause、resume、renew、get_endpoint
- **Execution API**（execd 实现）：execute code、run command、file operations

两套 API 甚至由不同进程实现。y-agent 不需要这么极端的分离（它是单进程），但可以在 trait 层面做类似的区分。

#### 2.5.3 y-agent 的具体方案

将当前的一个 trait 拆为两个：

```rust
/// 命令执行接口 — 在已就绪的运行时环境中执行命令
#[async_trait]
pub trait RuntimeExecutor: Send + Sync {
    fn name(&self) -> &str;
    async fn execute(&self, ctx: &RuntimeContext, command: &Command) -> Result<ExecutionResult>;
    async fn spawn(&self, ctx: &RuntimeContext, command: &Command) -> Result<ProcessHandle>;
    async fn kill(&self, handle: &ProcessHandle) -> Result<()>;
    async fn status(&self, handle: &ProcessHandle) -> Result<ProcessStatus>;
}

/// 运行时生命周期管理 — 环境本身的创建、健康检查、清理
#[async_trait]
pub trait RuntimeLifecycle: Send + Sync {
    async fn health_check(&self) -> Result<HealthStatus>;
    async fn cleanup(&self, ctx: &RuntimeContext) -> Result<()>;
    async fn warmup(&self, images: &[String]) -> Result<()>;       // 预热
    async fn pool_status(&self) -> Option<PoolStatus>;              // 池状态（如果支持池化）
}
```

`DockerRuntime` 同时实现两个 trait。`RuntimeManager` 通过 `RuntimeExecutor` 调用执行，通过 `RuntimeLifecycle` 管理生命周期。

#### 2.5.4 论证：为什么应该做

**正面理由**：

1. **为容器池化提供干净的扩展点**：池管理（`warmup`、`pool_status`）属于生命周期，不应该和单次命令执行混在一起。拆分后池化逻辑有明确的归属。

2. **职责更清晰**：`RuntimeManager` 的 `execute()` 方法只需要 `RuntimeExecutor`，后台维护任务只需要 `RuntimeLifecycle`。依赖更精确。

3. **未来可扩展性**：如果未来需要远程运行时（类似 OpenSandbox 的 SDK → Server 模式），`RuntimeExecutor` 可以有一个远程实现（通过网络调用），而 `RuntimeLifecycle` 由远程服务自行管理。两个关注点的远程化策略不同。

**反面风险**：

1. **过早抽象**：当前只有三个运行时后端，trait 拆分是否过早？如果 DockerRuntime、NativeRuntime、SshRuntime 的生命周期差异不大，两个 trait 增加了接口复杂度却没带来实际收益。

2. **实现冗余**：Rust 中一个 struct 实现两个 trait 是常见模式，但调用方需要处理两个 trait object 或使用 trait alias，增加了类型系统的复杂度。

#### 2.5.5 论证：为什么可能不做

1. **如果不做容器池化**，当前 trait 的 6 个方法已经足够简洁，没有拆分的动力。

2. **拆分可以推迟到需要时再做**：当池化逻辑实际需要添加 `warmup` / `pool_status` 方法时再拆分，不违反 YAGNI 原则。Rust 的 trait 拆分是向后兼容的重构。

3. **当前阶段是设计文档期**，trait 定义还没有实际代码。在实现时自然会遇到职责膨胀，那时再决定是否拆分有更充分的信息。

---

## 3. 不建议借鉴的部分

### 3.1 Python + Go 双语言架构

OpenSandbox 用 Python (FastAPI) 做编排层、Go 做容器内执行——这是因为它是平台服务，追求开发效率和生态兼容。y-agent 是 Rust-first 框架，没有理由引入额外语言。

### 3.2 完整的 execd 守护进程

OpenSandbox 的 execd 提供 Jupyter 集成、多语言内核管理、SSE 流式传输等功能，这是面向"交互式编程平台"场景的。y-agent 的 Tool 执行是单次命令调用，不需要容器内的完整 HTTP 服务。如果需要容器内 agent，一个极简的 Rust 二进制（接收 stdin JSON → 执行 → 返回 stdout JSON）更符合 y-agent 的设计哲学。

### 3.3 多语言 SDK 层

OpenSandbox 的多语言 SDK 是平台服务的必要组成部分。y-agent 是单进程框架，不需要客户端 SDK。

### 3.4 K8s CRD 控制器

BatchSandbox/Pool CRD 是 K8s 原生的资源管理方式。y-agent 阶段性目标是单机运行（non-goal: "Not a container orchestration system"），K8s 支持是未来扩展。

---

## 4. 借鉴优先级与决策建议

| 借鉴项 | 实现成本 | 安全收益 | 性能收益 | 有力论点 | 主要反对论点 | 建议 |
|--------|---------|---------|---------|---------|------------|------|
| 容器池化 + docker exec 复用 | 中（~300 行 + 后台 task） | 无 | **关键**：1.5s → 50-100ms | P95 < 100ms 无法在容器场景下达成 | 状态泄漏风险；如果大多数工具走 NativeRuntime 则收益有限 | **采纳，但需先明确哪些工具必须走容器** |
| 安全容器运行时抽象 | 极低（~30 行） | **本质性**：用户态内核/VM 隔离 | 无（gVisor 增加 ~50ms） | 成本近乎为零；与 OCI 接口完全兼容；应对容器逃逸 | 需要宿主机安装 gVisor；gVisor 有 syscall 兼容性限制 | **采纳，作为可选配置** |
| 优雅降级框架 | 极低（~60 行） | 间接（使降级可审计） | 无 | 将隐式行为变为显式/可观测；对应 Fail Fast 原则 | 当前散点式策略已经能工作；如果始终在完整环境部署则不触发 | **采纳，低成本高收益的工程改进** |
| FQDN 网络出站控制 | 高（多容器编排 + DNS 代理） | **填补空白**：External(domains) 声明无实施 | 负面（多一个 sidecar） | NetworkCapability 模型在网络维度形同虚设 | 实现复杂度最高；使用场景可能较窄；DNS-only 模式有安全假象 | **暂缓，等池化和安全运行时稳定后再评估** |
| 生命周期/执行 trait 分离 | 极低（trait 重构） | 无 | 无 | 为池化提供干净扩展点 | 过早抽象；可以实现时再拆分 | **推迟到实现阶段，根据实际需要决定** |

---

## 5. 架构对比总结

| 设计维度 | OpenSandbox | y-agent Runtime | 差距分析 |
|----------|-------------|-----------------|----------|
| **容器运行时** | Docker + K8s，支持 gVisor/Kata/Firecracker | Docker only（K8s 未来扩展） | y-agent 缺少安全容器运行时支持 |
| **网络隔离** | FQDN 级 DNS 代理 + nftables 两层防御 | Docker NetworkMode 级别 | y-agent 只有粗粒度控制 |
| **容器生命周期** | 长期存活 + TTL 续期 + 池化 | 一次性创建/销毁 | y-agent 缺少池化和复用 |
| **降级策略** | 系统化的三级降级（disabled/dns-only/dns+nft） | 散点式 fallback | y-agent 缺少统一降级框架 |
| **安全层次** | 7 层防御 + 安全容器 + FQDN 网络控制 | 7 层防御（设计文档层面） | 总体对齐，细节实现差异大 |
| **可扩展性** | SandboxService ABC + Factory + Resolver | RuntimeAdapter trait + RuntimeManager | 模式相近，y-agent 更紧凑 |
| **性能** | 池化冷启动 ~50ms，非池化 ~500ms | 目标 P95 < 100ms（尚未实现） | 池化是达成目标的关键 |

---

## 6. 结论

OpenSandbox 与 y-agent 面向相同的威胁模型（AI Agent 执行不可信代码），但已经在生产环境中验证了多项关键技术。通过逐项详细论证，形成以下判断：

**建议立即采纳（低成本，高确定性收益）**：

- **安全容器运行时抽象**（~30 行代码）：给 Docker container create 加一个 `--runtime` 参数，就能从共享内核隔离跳到用户态内核隔离。成本几乎为零，作为可选配置不影响现有行为。唯一前提是宿主机安装 gVisor。
- **优雅降级框架**（~60 行代码）：将散落的降级逻辑统一为 `IsolationLevel` 枚举 + 启动时探测 + 执行时策略匹配。使隐式降级变为显式、可观测、可配置。

**建议采纳但需先明确前提条件**：

- **容器池化 + docker exec 复用**：收益最大（解决 1.5s vs 100ms 的结构性矛盾），但实施成本也最大，且引入状态泄漏风险。**前提条件**：先明确 y-agent 中有多少工具必须走容器路径（如果大多数工具走 NativeRuntime + bubblewrap 就能满足安全需求，池化的紧迫性大幅降低）。

**建议暂缓**：

- **FQDN 网络出站控制**：虽然填补了 `NetworkCapability::External(domains)` 从声明到实施的空白，但实现复杂度最高（多容器编排 + DNS 代理），使用场景可能较窄。可以先靠 Manifest 审核管控网络权限，等核心运行时稳定后再评估。
- **生命周期/执行 trait 分离**：推迟到实现阶段根据实际需要决定，避免过早抽象。

所有借鉴项的共同特征是：它们不改变 y-agent Runtime 的核心抽象（`RuntimeAdapter` trait 和 capability-based model），只在实现层面引入更精细的机制。这与"Architectural Stability Over Feature Velocity"原则一致。

需要决策的关键问题是：**y-agent 的工具执行中，有多大比例必须走 Docker 容器路径？** 这个比例直接决定了池化方案的优先级和 FQDN 控制的紧迫性。如果答案是"大多数走 NativeRuntime"，那安全容器运行时 + 降级框架两项低成本改进就足以构成第一批借鉴。
