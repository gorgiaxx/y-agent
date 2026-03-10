# Memex(RL): Scaling Long-Horizon LLM Agents via Indexed Experience Memory

> 论文精读笔记 — 与 y-agent 架构的对照分析
>
> **论文**: Zhenting Wang, Huancheng Chen, Jiayun Wang, Wei Wei (Accenture, 2026)
> **来源**: arXiv:2603.04257v1
> **记录日期**: 2026-03-06

---

## 一、论文核心思想

### 1.1 问题定义

LLM Agent 在长周期任务中受限于有限上下文窗口。随着轨迹增长，工具输出和中间推理不断累积，导致：

- 工作上下文变得过长，最终超出上下文预算
- 即使证据仍在上下文中，距离过远的信息也更难被有效利用（注意力稀释）

现有方案（截断、滚动摘要）本质上是**有损的**：它们丢弃或压缩了原始证据本身。

### 1.2 Indexed Experience Memory

Memex 的核心创新是**压缩上下文但不丢弃证据**。机制由两部分组成：

| 组件 | 位置 | 内容 |
|------|------|------|
| **Indexed Summary (σ)** | 上下文内 | 紧凑的可操作进度状态 + 索引映射 {(index, description)} |
| **External Experience Store (D)** | 上下文外 | index → content 的键值存储，保存完整原始交互 |

两个核心操作被定义为 Agent 的一等工具：

- **CompressExperience(IndexedSummary, MemoryBlocks)**: 将当前工作上下文归档到外部存储 D，并将上下文重写为 M ← [system_prompt, task, IndexedSummary]
- **ReadExperience(index)**: 从 D 中解引用一个索引，将完整内容重新注入上下文

与纯摘要压缩的关键区别：索引提供**精确的、可审计的**访问路径，指向具体的存档制品（而非模糊的语义匹配）。

### 1.3 MemexRL 训练框架

将记忆操作视为与环境工具同等的动作空间，用 RL 联合学习**写策略**（压缩什么、如何索引、何时归档）和**读策略**（何时和读什么）。

**奖励设计**（Episode 级别）：

```
R = R_task − P_context − P_redundancy − P_format
```

| 奖励/惩罚项 | 含义 |
|-------------|------|
| R_task | 任务成功奖励 |
| P_context | 上下文溢出惩罚（工作上下文超过阈值τ的累计量） |
| P_redundancy | 冗余工具调用惩罚（相同签名的重复调用比例） |
| P_format | 格式错误惩罚（畸形的工具调用） |

**三个关键训练技术**：

1. **分段轨迹处理 (Segmented Trajectory Processing)**: 压缩事件发生时，将轨迹在压缩边界处分段，每段作为独立训练样本处理。所有来自同一轨迹的段共享终端奖励，通过 GRPO 的组相对优势估计保持写策略的信用分配。

2. **软触发机制 (Soft Triggering)**: 不强制在阈值处自动压缩，而是向 Agent 暴露上下文状态信息 `[Context Status: working=w, total=t, threshold=L]`，让 Agent 学习自主决定压缩时机。

3. **双模式归档**: 模型可以(a)直接编写内容（重组笔记、摘要发现），或(b)指定文本锚点（start_anchor, mid_anchor, end_anchor）来逐字提取原始对话跨度。

### 1.4 理论分析

论文证明了两个性质：

- **Proposition 1**: 如果 indexed summary 是 B-bounded decision-sufficient 的（即最优动作可以从σ加至多B次解引用中恢复），则 Memex 策略可以匹配全上下文最优策略的期望回报。
- **Proposition 2**: 工作上下文长度被常数 C_max = τ_σ + BL 界定，与完整消息历史长度无关。压缩比随历史增长无界增长。

### 1.5 实验结果

在修改后的 ALFWorld 环境上：

| 指标 | 无RL | 有MemexRL |
|------|------|-----------|
| 任务成功率 | 24.22% | **85.61%** |
| 峰值工作上下文 | 16,934 tokens | **9,634 tokens** |

行为变化：CompressExperience 调用从约 6.5 次/episode 降至约 3 次，ReadExperience 调用从约 1 次增至约 6-7 次。RL 教会了 Agent 更选择性地压缩、更频繁地利用索引检索。

---

## 二、与 y-agent 现有设计的对照

### 2.1 核心理念对齐

Memex 的设计哲学与 y-agent 的多项设计原则高度一致：

| y-agent 原则 | Memex 对应 | 对齐程度 |
|-------------|-----------|---------|
| **Token 效率是一等约束** (CLAUDE.md §3.4) | 紧凑索引摘要 + 按需解引用 | 高度对齐 |
| **显式优于隐式** (CLAUDE.md §3.3) | 稳定索引提供精确、可审计的访问路径 | 高度对齐 |
| **快速失败、廉价恢复** (CLAUDE.md §3.6) | 压缩边界作为自然检查点 | 部分对齐 |
| **模型无关设计** (CLAUDE.md §3.4) | 弱模型也可以通过RL学会有效使用记忆工具 | 高度对齐 |

### 2.2 现有机制与 Memex 的差距

#### 上下文压缩 (context-session-design.md)

y-agent 当前的 Compaction 有三种策略：Summarize、SegmentedSummarize、SelectiveRetain。

| 维度 | y-agent Compaction | Memex Indexed Experience |
|------|--------------------|-------------------------|
| **触发方式** | 系统触发（Context Window Guard 检测到 >85%） | Agent 自主决定（上下文状态作为可学习信号） |
| **压缩结果** | 一段自然语言摘要替换旧消息 | 结构化索引映射 + 外部归档的完整内容 |
| **信息损失** | 有损（依赖 LLM 摘要质量） | 理论上无损（完整内容在外部存储中保留） |
| **恢复精度** | 无法恢复——原始信息已丢失 | 精确恢复——通过索引解引用获取原始内容 |
| **Agent 控制** | Agent 对压缩过程无感知 | Agent 完全控制压缩内容、索引方式、恢复时机 |

#### 短期记忆 (memory-short-term-design.md)

y-agent 的 STM 使用 Compact（无损磁盘卸载）+ Compress（LLM 摘要）双阶段策略。

| 维度 | y-agent STM Compact | Memex CompressExperience |
|------|---------------------|-------------------------|
| **卸载方式** | 自动选择大型 Tool 消息 → 磁盘文件 | Agent 主动决定归档什么 → 键值存储 |
| **上下文残留** | 预览文本 + 文件路径提示 | 结构化索引摘要（语义描述 + 稳定索引） |
| **恢复方式** | grep（关键词搜索）/ read（全文读取） | ReadExperience(index)（精确解引用） |
| **恢复质量** | 需要 Agent 知道关键词或文件路径 | 索引自描述，Agent 可按语义选择 |
| **组织方式** | 无组织（按消息顺序卸载） | Agent 自行组织索引结构 |

**关键洞察**: y-agent STM 的 Compact 机制在「保留完整内容」这一点上与 Memex 一致，但缺少**结构化索引**和 **Agent 自主控制**这两个关键要素。

#### Working Memory (micro-agent-pipeline-design.md)

y-agent 的 Working Memory 是 pipeline-scoped 的结构化黑板，认知分类（Perception, Structure, Analysis, Action）。

| 维度 | y-agent Working Memory | Memex Experience Store |
|------|----------------------|----------------------|
| **作用域** | Pipeline 执行（单次任务） | Session 级别（跨多个工具调用） |
| **数据结构** | 预定义 schema 的 typed slots | 自由形式的键值对（index → content） |
| **生命周期** | Pipeline 结束即销毁 | 整个 Session 持续存在 |
| **索引方式** | 按 slot key 访问 | 按 Agent 自命名的稳定索引访问 |
| **目的** | 步骤间传递结构化中间结果 | 长周期任务中的证据归档与检索 |

**关键区别**: Working Memory 解决的是 pipeline 内步骤间的状态传递；Memex Experience Store 解决的是长周期 session 内的证据持久化。两者互补，不冲突。

---

## 三、可借鉴性评估

### 3.1 值得借鉴的思想

#### (A) 记忆操作作为一等工具（高价值）

**核心思想**: CompressExperience 和 ReadExperience 不是系统自动执行的后台操作，而是 Agent 可以主动调用的工具，与环境工具处于同一动作空间。

**对 y-agent 的价值**: 当前 Compaction 对 Agent 完全透明——Agent 不知道何时发生了压缩，也无法控制压缩什么。将记忆管理权交给 Agent 意味着：
- Agent 可以在自然语义边界（如完成一个子任务后）主动压缩，而不是在硬阈值处被动触发
- Agent 可以控制索引的粒度和组织方式（如按子任务、按工具输出类型）
- Agent 可以在需要时精确取回之前的证据，而不是靠模糊的关键词搜索

这与 y-agent 的「显式优于隐式」原则完全一致。

#### (B) 结构化索引摘要替代自然语言摘要（高价值）

**核心思想**: 压缩后的上下文不是一段自然语言描述，而是一个结构化的索引映射：`{(index, description)}` + 可操作的进度状态。

**对 y-agent 的价值**: 自然语言摘要有两个问题：(1) 信息丢失不可逆，(2) LLM 在解析自己之前生成的摘要时可能产生幻觉。结构化索引映射提供：
- 精确的指针，而非模糊的文本描述
- 可审计的归档路径——每个索引指向一个具体的存档制品
- 更紧凑的上下文表示——索引映射远比完整摘要节省 token

#### (C) 上下文状态作为可学习信号（中高价值）

**核心思想**: 不是在阈值处自动触发压缩，而是将上下文状态 `[working=w, threshold=L]` 暴露给 Agent，让 Agent 自主决策。

**对 y-agent 的价值**: 当前 Context Window Guard 是硬规则。软触发让 Agent 可以：
- 在任务即将完成时延迟压缩（省去不必要的压缩开销）
- 在自然语义边界提前压缩（产生更高质量的索引）
- 根据当前任务的特征调整压缩策略

这与 y-agent 的「模型无关」原则兼容——通过 RL 训练弱模型也能学会有效的上下文管理。

#### (D) 冗余工具调用惩罚思想（中等价值）

**核心思想**: 如果 Agent 在没有状态变更的情况下重复相同的工具调用，应当受到惩罚——Agent 应当从 Experience Store 中检索之前的结果，而非重新执行。

**对 y-agent 的价值**: 这可以集成到 y-agent 的 Guardrails 系统中，作为 LoopGuard 的一个变体——检测冗余工具调用并建议 Agent 使用记忆检索替代。

### 3.2 借鉴时需要注意的局限

#### (A) RL 训练的实际成本

Memex 的实验使用 Qwen3-30B-A3B 模型，在修改后的 ALFWorld 环境中训练。对 y-agent（一个框架而非特定任务）来说：

- y-agent 不训练自己的 LLM，而是使用第三方模型（Claude、GPT-4、Qwen 等）
- RL 训练需要针对特定任务/环境，不能泛化为通用框架特性
- **替代方案**: 可以通过高质量 prompt 工程 + 示例轨迹 来教会 Agent 使用记忆工具，而非 RL 训练

#### (B) 单 Session 假设

Memex 的 Experience Store 是 episode 级别的——每个任务一个独立的存储。y-agent 需要考虑：

- 跨 session 的 Experience Store 是否有价值？（可能与 LTM 职责重叠）
- 多 Agent 协作场景下的 Experience Store 共享？
- 与 Canonical Session 的交互？

#### (C) 索引质量依赖 Agent 能力

Agent 需要：编写高质量的索引描述、选择合适的归档粒度、在正确时机进行检索。较弱的模型可能难以做到。MemexRL 通过 RL 训练解决此问题，但如前所述这对框架来说不可行。

**缓解方案**: 提供结构化 prompt 模板和索引组织指南，降低对模型能力的要求。

---

## 四、对 y-agent 的具体修改建议

### 4.1 新增：Indexed Experience Memory 作为 STM 的增强策略

**修改文档**: `memory-short-term-design.md`, `context-session-design.md`

在现有 Compact/Compress/Auto 之外，新增 **IndexedExperience** 压缩策略：

| 策略 | 触发方式 | 上下文残留 | 恢复方式 |
|------|---------|-----------|---------|
| Compact | 系统自动 | 预览 + 文件路径 | grep / read |
| Compress | 系统自动 | LLM 摘要 | 不可恢复 |
| **IndexedExperience** (新) | **Agent 主动** | **结构化索引摘要** | **ReadExperience(index)** |

IndexedExperience 策略会在 STM 层维护一个 `ExperienceStore: HashMap<String, String>`，由 Agent 通过工具调用管理。

### 4.2 新增：记忆工具注册

**修改文档**: `tools-design.md`

注册两个新的内建工具：

| 工具 | 参数 | 行为 |
|------|------|------|
| `compress_experience` | indexed_summary: String, memory_blocks: Vec<{index, content}> | 将当前工作上下文归档到 Experience Store，重写上下文为索引摘要 |
| `read_experience` | index: String | 从 Experience Store 解引用索引，将内容注入上下文 |

这两个工具的 capability 类型为 `Memory` (新增)，不需要 Runtime 隔离——它们直接操作 STM 的 ExperienceStore。

### 4.3 修改：Context Window Guard 支持软触发

**修改文档**: `context-session-design.md`

Context Window Guard 新增 `soft_trigger` 模式：

| 模式 | 行为 |
|------|------|
| `auto` (现有) | 超过 85% 阈值时自动触发 Compaction |
| `soft` (新) | 向 Agent 上下文注入 `[Context Status]` 信息；仅在超过 95% 硬限制时才强制压缩 |
| `hybrid` (新) | 软触发为主，85% 阈值时注入警告，95% 时强制压缩 |

软触发模式通过一个新的 `ContextStatusMiddleware`（注册在 y-hooks ContextMiddleware chain 中）在每步 Agent 循环开始时注入上下文状态信息。

### 4.4 修改：Guardrails 集成冗余工具调用检测

**修改文档**: `guardrails-hitl-design.md`

在 LoopGuard 的检测模式中新增 **RedundantToolCallGuard**：

| 检测条件 | 建议动作 |
|----------|---------|
| 相同 (tool_name, arguments) 重复调用，且期间无状态修改操作 | 建议 Agent 使用 `read_experience` 检索之前的结果 |

这不是一个硬阻断——而是一个中间件级别的提示，让 Agent 意识到冗余行为。

### 4.5 Memory 架构更新

**修改文档**: `memory-architecture-design.md`

在三层记忆体系中明确 Experience Store 的定位：

```
Working Memory (pipeline-scoped)  ←→  Experience Store (session-scoped, indexed)  ←→  Short-Term Memory (session-scoped, buffer)  ←→  Long-Term Memory (persistent)
```

Experience Store 不是新的记忆层，而是 STM 的一个增强组件——它由 STM 拥有和管理，但通过专用工具暴露给 Agent。

Session 结束时，Experience Store 中的高价值条目可以自动提取到 Long-Term Memory（作为 Task Memory 或 Experience Memory）。

---

## 五、优先级与实施建议

| 优先级 | 修改项 | 依赖关系 | 预计影响 |
|--------|--------|---------|---------|
| **P0** | Context Status 软触发机制 | ContextMiddleware chain | 低侵入，立即可用，所有模型受益 |
| **P0** | compress_experience / read_experience 工具注册 | Tool Registry | 低侵入，不改变现有压缩逻辑 |
| **P1** | IndexedExperience 压缩策略 | STM Engine, 上述工具 | 中等侵入，需要修改 STM 状态模型 |
| **P1** | 冗余工具调用检测 (LoopGuard 扩展) | Guardrails ToolMiddleware | 低侵入，增强现有 LoopGuard |
| **P2** | Experience Store → LTM 自动提取 | LTM Extraction pipeline | 中等侵入，需要新的提取逻辑 |
| **P2** | RL/prompt 优化记忆工具使用 | 整体系统 | 面向未来的优化方向 |

---

## 六、与竞品的关系

论文 Related Work 中提到的系统与 y-agent competitive-analysis（现位于 docs/research/competitive-analysis.md）中分析的竞品有交集：

| 系统 | 论文中的定位 | y-agent 已有借鉴 |
|------|-------------|-----------------|
| MemGPT | 早期 LLM-as-OS 记忆管理 | 部分（STM 的 Compact 思路类似） |
| Reflexion | 经验记忆（verbal RL） | 是（Skill 自演进中的经验捕获） |
| HippoRAG | 神经生物学启发的索引 | 否（但 LTM 的多维索引有相似性） |
| MEM1, MemAgent, Memory-R1 | RL 训练的记忆管理 | 否（y-agent 不做模型训练） |
| SUPO, ReSum | 多轮工具使用中的摘要压缩 | 部分（STM Compress 策略） |

Memex 与这些系统的关键区分点——**索引化归档 + 精确解引用**——是 y-agent 目前缺少的，值得引入。

---

## 七、总结

Memex 论文的核心价值在于一个简单但深刻的洞察：**压缩上下文不等于丢弃证据**。通过将完整交互归档到外部存储并在上下文中保留结构化索引，Agent 可以在有限的工作上下文中保持决策质量，同时支持任意长度的任务轨迹。

对 y-agent 而言，这不需要推翻任何现有设计，而是对 STM 和 Context 管理的自然增强。最有价值的借鉴是：

1. 将记忆操作暴露为 Agent 可调用的一等工具
2. 用结构化索引摘要替代纯自然语言摘要
3. 将上下文管理从系统硬规则转变为 Agent 可学习的技能

这三项改进与 y-agent 的「显式优于隐式」「Token 效率是一等约束」「模型无关设计」原则完全一致。
