# AutoSkill: Experience-Driven Lifelong Learning via Skill Self-Evolution

> 项目深度分析 — 与 y-agent Skill 体系的对照
>
> **项目**: ECNU-ICALK/AutoSkill (华东师范大学 + 上海人工智能实验室)
> **论文**: arXiv:2603.01145
> **版本**: AutoSkill 1.0 (2025-02-04), 离线抽取 (2026-03-01)
> **记录日期**: 2026-03-06

---

## 一、AutoSkill 核心思想

### 1.1 问题定义

LLM 的每次对话是孤立的——上一轮中用户表达的偏好、纠正、约束不会自动迁移到下一轮。用户被迫反复重申同一套要求（"不要幻觉""用学术口吻""先给大纲再展开"）。

AutoSkill 将这个问题定义为 **Experience-driven Lifelong Learning (ELL)**：从真实交互经验中持续抽取可复用的能力单元（Skill），并通过 merge + version 机制让 Skill 随经验积累自动演进。

### 1.2 核心闭环

AutoSkill 运行两条并行管线：

```
管线 A — 服务 (同步):
  User Query → Query Rewrite → Skill Retrieval → Skill Injection → LLM Response

管线 B — 演进 (异步):
  {上一轮上下文 + 当前用户反馈} → Extraction Gating → Skill Extraction → Maintenance (add/merge/discard) → SkillBank
```

两条管线的耦合点：管线 A 的 top-1 检索结果作为 `retrieved_reference` 传给管线 B 的 Extraction，用于 identity context（判断是更新已有 skill 还是创建新 skill）。管线 B 的产出更新 SkillBank 的向量索引，立即可被下一轮管线 A 检索到。

### 1.3 Skill 格式

采用 `SKILL.md` 格式（兼容 Anthropic Agent Skill 约定），YAML frontmatter + Markdown body：

```yaml
---
id: "release-safety-protocol"
name: "Release Safety Protocol"
description: "Standard pre-release safety checklist"
version: "0.1.1"
tags: ["devops", "release"]
triggers: ["release", "deploy", "rollout"]
examples:
  - input: "How should I do a safe release?"
    output: "1. Run regression..."
---

# Release Safety Protocol

Standard pre-release safety checklist

## Prompt

Before each release, follow these steps:
1. Run regression tests on staging
2. Deploy canary to 5% traffic
...
```

Skill 的核心内容是 `instructions`（即 `## Prompt` 部分）——直接作为 LLM 指令注入系统提示词。

### 1.4 Skill 存储与检索

存储采用文件系统 + 向量索引的混合结构：

| 层 | 位置 | 内容 |
|----|------|------|
| 文件存储 | `SkillBank/Users/<uid>/<slug>/SKILL.md` | 完整 Skill 定义，可人工编辑 |
| 向量索引 | `SkillBank/vectors/` | Embedding 向量（支持 flat/Chroma/Pinecone/Milvus） |
| 关键词索引 | `SkillBank/index/skills-bm25.*` | BM25 倒排索引 |
| 使用统计 | `SkillBank/index/skill_usage_stats.json` | 每 skill 的检索次数、实际使用次数 |

检索采用 Hybrid Ranking（embedding 相似度 + BM25 分数加权融合），支持 user/library/all 三种 scope。

---

## 二、自我迭代机制详解

这是 AutoSkill 的核心价值，也是与传统 RAG/Skill 系统的根本区别。

### 2.1 Extraction — 从对话中抽取 Skill

#### 抽取时机 (Gating)

Gating 模块使用轻量启发式判断是否该尝试抽取：

- **ack 检测**: 短文本 (<=40 chars) 且以 thanks/ok/got it 开头 → 不触发
- **话题切换检测**: 关键词重叠率、revision hints（rewrite/edit + "it"/"this"引用）、显式新话题标志（"by the way"/"new topic"）
- **抽取策略**: `auto` (每 N 轮) / `always` (每轮) / `never` (仅手动)

关键设计决策：Gating 只控制"是否尝试"，质量过滤在 Extractor 内部完成（可以返回空结果）。

#### 抽取证据模型

LLM Extractor 接收的 payload 将对话拆分为两个部分：

| 部分 | 内容 | 作用 |
|------|------|------|
| `primary_user_questions` | 仅 USER 角色的消息 | 唯一证据来源 |
| `full_conversation` | 完整对话（USER + ASSISTANT） | 仅用于消歧，不作为证据 |

这是一个深思熟虑的设计：**Skill 只能从用户表达的偏好/约束/流程中提取，不能从 Assistant 生成的内容中提取**。这避免了 LLM 将自己的生成风格自我强化为 Skill 的循环偏差。

#### 抽取 Prompt 的关键规则

1. **约束优于内容**: 关注 HOW（流程、约束、格式要求），而非 WHAT（具体内容）。用占位符（`<SOURCE_CONTENT>`）替代实例数据。
2. **去身份化**: 组织名、URL、日期等具体标识被移除或参数化。
3. **不发明**: 只提取对话中有证据支持的逻辑，不推断用户未说明的工作流。
4. **弱确认不构成证据**: "ok""continue" 这类弱应答不验证 assistant 的细节。
5. **最近性**: 聚焦最近 3-6 轮 user turns，检测 boundary turn（新目标/新任务的分界点），boundary 之后的意图是权威的。

#### 输出

每次抽取至多产出 `max_candidates` 个 `SkillCandidate`，每个包含 name、description、instructions、triggers、tags、examples、confidence。

### 2.2 Maintenance — add/merge/discard 三择决策

这是整个系统最复杂的决策链路，决定新抽取的 candidate 如何影响现有 SkillBank。

#### 决策流水线

```
SkillCandidate
  │
  ├─ Step 1: Exact-identity fast path
  │   identity_desc_norm(name, description) → SHA1 hash
  │   若 SkillBank 中已有相同 identity_desc_norm 的 skill → merge
  │
  ├─ Step 2: Previous-skill hint
  │   上一次该用户的 upsert 记录指向的 skill
  │   向量搜索 score ≥ force_merge_threshold → merge
  │
  ├─ Step 3: Vector search
  │   query = name + description + instructions
  │   对 user + library scope 分别检索 top-N
  │
  ├─ Step 4: LLM decision (若 maintenance_strategy == "llm")
  │   LLM 判断: add / merge(target_skill_id) / discard
  │   规则: same capability → 绝不 add; name 冲突 → 绝不 add
  │
  ├─ Step 5: Guardrails
  │   LLM 说 add 但向量相似度很高 + 可合并 → 强制 merge
  │   LLM 说 add 但只有 library skill 匹配 → 强制 discard
  │
  └─ Step 6: Heuristic fallback
      user hit above threshold + can_merge → merge
      library hit above threshold + can_merge → discard
      else → add
```

#### Merge 质量判断

Merge 决策使用双通道评估：

| 通道 | 方法 | 权重 |
|------|------|------|
| LLM judge | 判断 `same_capability` + confidence 分数 | 主通道 |
| 确定性评分 | `0.70 × semantic_sim + 0.18 × signal_sim + 0.12 × name_sim` | 回退通道 |

当 LLM confidence < 0.55 时，切换到确定性评分决策。

#### Merge 执行

两种 merge 实现：

- **启发式 merge**: 比较指令质量分数（编号步骤、"output format"、"validation" 等关键词加分），高质量方优先。合并 triggers、tags、examples，去重。Bump patch version。
- **LLM merge**: LLM 将两份 Skill 合并为一份，输出合并后的完整 JSON。遵循反重复、去身份化、不发明规则。失败回退启发式。

### 2.3 Usage Tracking — 闭环反馈

每轮对话后，异步判断检索到的 Skill 是否真正被使用：

| 组件 | 方法 | 输出 |
|------|------|------|
| `LLMSkillUsageJudge` | LLM 审计每个 skill 的 `relevant` + `used` | per-skill judgment |
| 确定性回退 | 关键词重叠（query ∩ skill、reply ∩ skill） | per-skill judgment |

Usage 统计累积在 `skill_usage_stats.json` 中。当一个 user skill 被检索 >= 40 次但实际使用 <= 0 次时，自动标记为 stale 候选，可被剪枝。

这形成了完整的反馈闭环：**抽取 → 存储 → 检索 → 使用 → 统计 → 剪枝**。

### 2.4 Version History

版本通过 metadata 内嵌的 snapshot 栈管理（不是独立的版本存储）：

```python
skill.metadata["_autoskill_version_history"] = [
    {version: "0.1.0", instructions: "...", ...},
    {version: "0.1.1", instructions: "...", ...},
]
```

支持 push/pop 语义——rollback 即 pop 最近 snapshot 并 apply。上限 30 个历史版本。

### 2.5 离线抽取

支持三种离线数据源：

| 数据源 | 输入格式 | 适用场景 |
|--------|---------|---------|
| 对话 | OpenAI `.json`/`.jsonl` | 历史聊天记录批量抽取 |
| 文档 | 文本/Markdown 文件 | 从知识文档中提取 SOP/流程 |
| Agent 轨迹 | 含 messages/events/trace 的 JSON | 从成功执行中提取工作流 skill |

Agent 轨迹抽取支持 `success_only` 过滤（只从成功执行中学习）和 `include_tool_events`（将工具调用事件纳入证据）。

---

## 三、与 y-agent Skill 体系的对照分析

### 3.1 设计哲学对比

| 维度 | y-agent | AutoSkill |
|------|---------|-----------|
| **定位** | Agent 框架的 Skill 子系统 | 独立的 Skill 演进服务 |
| **Skill 本质** | LLM 推理指令（LLM-instruction-only） | LLM 推理指令（instructions/prompt） |
| **演进驱动** | 经验记录 → 模式提取 → 提案 → 审批 | 对话 → 实时抽取 → add/merge/discard |
| **演进速度** | 批次作业（日/周/按需） | 实时（每次对话可触发） |
| **审批模型** | 人工审批为默认 | 全自动为默认 |
| **Token 预算** | 根文档 < 2000 tokens，子文档按需加载 | 无显式预算，检索时 `max_chars` 截断 |
| **格式** | 私有格式（skill.toml + root.md + details/） | 通用格式（SKILL.md，兼容 Anthropic） |
| **工具分离** | 严格——工具/脚本提取到 Tool Registry | 不分离——instructions 中可包含任意内容 |

### 3.2 核心机制对比

#### (A) 经验捕获

| 维度 | y-agent | AutoSkill |
|------|---------|-----------|
| **来源** | Agent 执行记录（task outcome + trajectory） | 用户对话（USER turns 为证据） |
| **粒度** | 一次完整 task 执行 | 一段对话窗口（最近 N 轮） |
| **结构** | 结构化 Experience Record (skill_id, outcome, key_decisions, tool_calls...) | 半结构化 payload (messages, events, hint) |
| **存储** | Memory System (EXPERIENCE 类型，LTM) | 不独立存储——直接转化为 Skill |
| **反馈信号** | outcome: Success/Partial/Failure | 用户下一轮消息作为隐式反馈 |

关键差异：y-agent 将经验独立存储后批次分析；AutoSkill 将经验即时转化为 Skill 更新。y-agent 的方式更审慎（有中间态可检查），AutoSkill 的方式更敏捷（闭环更快）。

#### (B) 模式提取 vs 实时抽取

| 维度 | y-agent Pattern Extraction | AutoSkill Skill Extraction |
|------|---------------------------|---------------------------|
| **时机** | 批次作业（积累足够经验后） | 实时（每轮对话后异步） |
| **输入** | 按 skill_id 分组的多条 Experience Records | 单次对话窗口 + 可选 retrieved_reference |
| **模式类型** | Edge Case / Common Error / Better Phrasing / New Capability / Obsolete Rule | 单一类型：SkillCandidate |
| **输出** | Evolution Proposal (需审批) | 直接 add/merge/discard（无需审批） |
| **LLM 消耗** | 一次批次分析多条经验 | 每次对话一次抽取 + 一次维护判断 |

y-agent 的设计更适合高风险场景（agent 执行有副作用的任务），AutoSkill 更适合低风险场景（对话辅助，最差结果是 Skill 质量不高）。

#### (C) Skill 演进决策

| 维度 | y-agent Skill Refinement | AutoSkill Maintenance |
|------|-------------------------|----------------------|
| **决策方** | 人/策略审批 (Supervised/Auto-minor/Auto-evaluated/Autonomous/Frozen) | 系统自动 (LLM + 启发式回退) |
| **决策选项** | approve / reject / defer | add / merge / discard |
| **回归检测** | 基于最近 N 次使用的 success_rate 对比 | 基于 usage_stats 的 stale 剪枝 |
| **回滚** | content-addressable storage, O(1) reflog rollback | metadata 内嵌 snapshot 栈, pop 回滚 |

#### (D) 版本管理

| 维度 | y-agent | AutoSkill |
|------|---------|-----------|
| **存储** | Content-addressable objects (git-like) | Metadata 内嵌 snapshot list |
| **寻址** | Content hash | 版本字符串 (semver) |
| **历史** | RefLog (append-only) | Snapshot 栈 (默认上限 30) |
| **GC** | 保留最近 N 版本，清理无引用对象 | 无 GC（栈上限自然限制） |
| **diff** | 支持任意两版本间 diff | 不支持（只能 rollback 到前一版本） |

y-agent 的版本管理明显更成熟，适合长期运行的 agent 系统。AutoSkill 的 snapshot 栈方案简单实用，但不适合复杂版本历史。

### 3.3 AutoSkill 有而 y-agent 缺少的

| 能力 | AutoSkill 实现 | y-agent 现状 | 差距评估 |
|------|---------------|-------------|---------|
| **实时抽取** | 每轮对话异步抽取 | 批次作业（日/周） | 高价值差距 |
| **证据分离** | USER turns = 证据, ASSISTANT = 上下文 | 未区分 | 高价值差距 |
| **Merge 判断** | LLM judge + 确定性评分双通道 | 设计中但未详细定义 merge 判断逻辑 | 中高价值差距 |
| **Usage Tracking** | 每轮审计 skill 实际使用情况 | 设计了 metrics 但无 per-turn usage audit | 高价值差距 |
| **Stale 剪枝** | retrieved >= 40 && used <= 0 → stale | 无自动剪枝 | 中等价值差距 |
| **离线轨迹抽取** | 从 agent 成功执行轨迹中抽取 workflow skill | 设计了 Experience Record 但无轨迹 → skill 转化 | 高价值差距 |
| **Skill 注入** | 检索后注入 system prompt | 设计了 Context Assembly 但未详述 skill 注入细节 | 中等价值差距 |
| **Query Rewrite** | LLM 改写查询提升检索质量 | Context pipeline 有 compaction 但无检索前查询改写 | 中等价值差距 |

### 3.4 y-agent 有而 AutoSkill 缺少的

| 能力 | y-agent 设计 | AutoSkill 现状 |
|------|-------------|---------------|
| **Token 预算控制** | 根文档 2000 tokens 硬限制，子文档按需加载 | 无预算——instructions 可以任意长 |
| **树形文档结构** | root.md + details/ 多级子文档 | 单文件 SKILL.md |
| **工具分离** | 严格——工具/脚本提取到 Tool Registry | instructions 中混合任意内容 |
| **安全筛查** | Prompt injection/privilege escalation/data exfiltration 检测 | 无安全筛查 |
| **Skill 分类** | llm_reasoning/api_call/tool_wrapper/agent_behavior/hybrid | 无分类（全部作为 instructions 处理） |
| **Cross-resource linking** | skill 引用其他 skill/tool/knowledge | skill 之间无引用关系 |
| **Human-in-the-loop** | 分层审批模型 (Supervised → Autonomous) | 无人工审批（完全自动） |
| **Content-addressable versioning** | Git-like 版本存储，支持任意版本 diff | Metadata 内嵌 snapshot 栈 |
| **回归检测** | 基于 success_rate 的统计回归检测 + 自动回滚提案 | 仅 stale 剪枝 |

---

## 四、可借鉴性评估

### 4.1 高价值借鉴

#### (A) 实时抽取闭环（而非仅批次分析）

**AutoSkill 方案**: 每轮对话后异步尝试抽取，将"下一轮用户消息"作为对上一轮的隐式反馈。抽取不阻塞对话延迟。

**借鉴理由**: y-agent 当前的经验 → skill 路径过长：Experience Record → 积累 → 批次 Pattern Extraction → Evolution Proposal → 审批 → 新版本。这个路径适合复杂 agent 任务（有副作用、高风险），但对于低风险的对话辅助、代码生成等场景，用户期望"说一次就记住"。

**建议**: 在现有批次演进管线之外，新增 **Fast-Path Extraction** 模式。当 agent 运行在低风险模式（对话辅助、无副作用工具）时，启用实时抽取。高风险模式仍走批次审批路径。这不需要修改现有设计，只需在 Skill 演进管线上增加一条快速通道。

与 y-agent 原则的对齐：
- **架构稳定性优于功能速度** (3.1): 新增通道，不修改现有批次管线
- **关注点分离** (3.2): Fast-Path 作为演进策略的一种配置，不改变 Skill Registry 的接口

#### (B) 用户证据与 Agent 生成的分离——防止自我强化偏差

**AutoSkill 方案**: 抽取时严格区分 USER turns（唯一证据来源）和 ASSISTANT turns（仅作上下文参考）。弱确认（"ok""continue"）不构成对 assistant 细节的验证。

**借鉴理由**: 这解决了一个微妙但重要的问题——如果从 assistant 输出中提取 skill，LLM 的风格偏好会通过 skill 自我强化形成反馈环。例如，模型喜欢用长列表回答，抽取为 skill 后更强化了长列表风格，用户实际上可能并不想要这种风格。

**什么是隐式反馈**:

AutoSkill 的 `_PendingExtraction` 机制将用户的**下一轮消息**作为对上一轮交互的反馈。具体流程：

```
Turn N:
  用户: "帮我写一份部署报告"
  助手: (生成报告，使用了表格和正式用语)
  系统: 暂存 {latest_user, latest_assistant, window} 到 _pending

Turn N+1:
  用户: "不要用表格，换成纯文本列表"   ← 这条消息就是对 Turn N 的反馈
  系统: 将 _pending.window + 这条反馈消息 打包送入 Extraction
```

"隐式"是相对于"显式反馈按钮"而言的——用户没有点击"有用/没用"，而是通过自然对话行为传递信号。AutoSkill 的 Extractor 会从这条反馈消息中提取出约束（"不要用表格"），但不会从助手的输出（"使用表格"）中提取。

**对 y-agent 的实际影响**: y-agent 的 Experience Record 当前只记录 `outcome` (Success/Partial/Failure) 和 `user_feedback`（显式反馈），但没有区分这些信号的来源可信度。问题在于 Pattern Extraction 分析经验时，如果 `trajectory_summary` 中 agent 自行总结了"用户偏好表格格式"（实际上用户只是没反对），这个错误判断会变成 skill 更新的依据。

**建议**: 不是引入抽象的"加权系数"，而是在 Experience Record 中标注每条证据的来源类别，让 Pattern Extraction 的 LLM prompt 中明确区分：

| 证据来源 | 定义 | Pattern Extraction 中的处理规则 |
|----------|------|-------------------------------|
| `user_stated` | 用户在对话中明确说出的约束、偏好、流程要求 | 可直接作为 skill 更新依据 |
| `user_correction` | 用户对 agent 输出的纠正（如"不要用表格"） | 可作为 skill 更新依据，且优先级最高（纠正 > 初始要求） |
| `task_outcome` | 任务成功/失败的客观结果 | 可作为统计信号（成功率），但不能直接作为 skill 内容依据 |
| `agent_observation` | Agent 从自身执行过程中总结的模式 | 仅当有 `user_stated` 或 `user_correction` 佐证时才可采信 |

不需要数值加权——而是在 Pattern Extraction 的 LLM prompt 中添加规则："从 `agent_observation` 提取的模式，必须有至少一条 `user_stated` 或 `user_correction` 佐证，否则丢弃。" 这是一条硬规则，不是模糊的权重。

#### (C) Usage Tracking 闭环——判断 skill 是否真正被使用

**AutoSkill 方案**: 每轮对话后，LLM 审计检索到的 skill 是否真正被使用（`relevant` + `used`），累积统计，自动标记 stale skill。

**判断依据的具体实现** (`usage_tracking.py`):

AutoSkill 用两个通道判断，主通道失败时回退到备用通道：

**主通道 — LLM 审计**: 将 `{query, assistant_reply, skills[]}` 打包发给 LLM，对每个 skill 判断两个布尔值：

- `relevant`: 该 skill 是否匹配当前用户查询的意图
- `used`: 助手回复是否**实际依赖并应用了**该 skill 的独特约束/工作流

LLM prompt 中的关键判断规则是：**"Be strict: if the reply can be produced well without this skill, set used=false."** 也就是说，如果不注入这个 skill 也能生成质量相当的回复，那就判定为未使用。`used=true` 要求 `relevant=true` 作为前置条件。

**备用通道 — 关键词重叠**: 当 LLM 输出缺失或不可解析时：

```
query_tokens  = keywords(user_query)
reply_tokens  = keywords(assistant_reply)
skill_tokens  = keywords(skill.name + description + tags + triggers + instructions[:400])

relevant = (query_tokens ∩ skill_tokens 非空) OR (该 skill 被注入了上下文)
used     = relevant AND (被注入了上下文) AND (reply_tokens ∩ skill_tokens 非空)
```

这个备用通道是粗粒度的——关键词重叠不等于真正使用——但胜过没有信号。

**累积信号与自动剪枝**: usage 判断结果累积到 `skill_usage_stats.json`。当一个 user skill 满足 `retrieved >= 40 && used <= 0`（被检索了 40 次但一次都没真正使用），自动标记为 stale 候选。

**对 y-agent 的落地建议**:

y-agent 目前有 `use_count`、`success_rate` 等 skill 指标，但这些只反映"用了这个 skill 之后任务成不成功"，没有回答"这个 skill 被注入到上下文后，LLM 到底有没有用它"。

具体做法：在 Agent Orchestrator 的 `TaskComplete` 事件中，除了现有的 outcome/trajectory 信息，新增一个 `skill_usage_audit` 字段。audit 逻辑可以在 Hook 系统的 PostToolMiddleware 或 PostTaskMiddleware 中执行，同样采用 LLM 主通道 + 关键词回退的双通道设计。

audit 产生的信号直接写入 Skill 的 evaluation metrics：

| 已有指标 | 回答的问题 | 新增指标 | 回答的问题 |
|----------|-----------|---------|-----------|
| `use_count` | 这个 skill 被关联到了多少次任务？ | `injection_count` | 这个 skill 被注入到上下文多少次？ |
| `success_rate` | 用了这个 skill 后任务成功率如何？ | `actual_usage_count` | 注入后 LLM 真正依赖它的次数 |
| — | — | `usage_rate` = actual / injection | 注入效率：是否在浪费 token 预算？ |

当 `usage_rate` 持续低于阈值（如 < 0.1），说明这个 skill 的 triggers/tags 定位不准（被错误检索）或 instructions 质量低（注入了但 LLM 不理解）。这个信号可以直接驱动 Pattern Extraction 中的 `Obsolete Rule` 检测。

#### (D) 从 Agent 执行轨迹中创建新的 Workflow Skill

**AutoSkill 方案**: 离线模块 (`offline/trajectory/extract.py`) 接收 agent 执行轨迹（含 messages、tool_calls/events/trace/steps、success/failure 标记），通过 `sdk.ingest()` 走完整的 Extraction → Maintenance 流程，产出新 skill。支持 `success_only=True` 只从成功执行中学习。

**y-agent 当前设计为什么不直接支持这个能力——差距在哪里**:

y-agent 的 Experience Record 确实包含了 `trajectory_summary`、`key_decisions`、`tool_calls` 这些字段。但问题不在于"数据够不够"，而在于 **Pattern Extraction 的处理逻辑只面向已有 skill 的改进，没有"无中生有"的路径**。

具体看 y-agent 当前的 Pattern Extraction 流程 (skill-versioning-evolution-design.md):

```
trigger_extraction()
  → query_recent_experiences(since: last_extraction)
  → group by skill_id                              ← 关键限制在这里
  → for each skill with > threshold experiences:
      extract_patterns(experiences, existing_patterns)
      → PatternAnalysisReport
```

**`group by skill_id`** 意味着：只有那些在执行时关联了某个 skill 的经验才会被分析。如果一个 agent 成功完成了一个复杂任务但没有使用任何 skill（因为还没有相关 skill），这条经验记录的 `skill_id` 为空，Pattern Extraction 会直接跳过它。

这就是差距：**y-agent 只能改进已有的 skill，不能从"没用 skill 但成功完成的执行"中发现新的可复用 workflow。**

举个具体场景：

```
场景: 用户三次要求 "部署服务到 staging 环境"
  每次 agent 都执行了类似的步骤:
    1. check_git_status → 确认分支干净
    2. run_tests → 确认测试通过
    3. build_docker_image → 构建镜像
    4. push_to_registry → 推送到注册中心
    5. kubectl_apply → 部署到 staging
    6. health_check → 验证部署成功

  三次都成功了，但因为没有对应的 skill，这三条 Experience Record 的 skill_id = null。
  Pattern Extraction 分组时跳过了它们。
  系统永远不会提议创建 "staging-deployment-workflow" skill。
```

**具体修改建议**:

在 Pattern Extraction 中新增一个独立阶段——**Skillless Experience Analysis**，专门处理 `skill_id = null` 的经验记录：

```
trigger_extraction()
  → query_recent_experiences(since: last_extraction)
  → group by skill_id
  → [现有逻辑] for each skill with > threshold experiences: ...

  → [新增阶段] filter experiences where skill_id is null AND outcome = Success
  → cluster by task_description similarity (embedding 聚类或 LLM 分组)
  → for each cluster with >= 3 experiences:
      extract_workflow_pattern(experiences)
        输入: 多条成功执行的 trajectory_summary + tool_calls + key_decisions
        输出: WorkflowSkillProposal
          - name: 从 task_description 共性中提炼
          - instructions: tool_call 序列模板化 (具体参数 → 占位符)
          - triggers: 从 task_description 中提取关键词
          - decision_points: key_decisions 的交集
          - success_conditions: 从成功执行中归纳
```

与现有 Pattern Extraction 的 5 种类型对比：

| 现有类型 | 前置条件 | 产出 |
|----------|---------|------|
| Edge Case | skill_id 非空 + 某 skill 的经验中出现异常场景 | 对该 skill 的子文档或规则修改 |
| Common Error | skill_id 非空 + 某 skill 的经验中出现重复错误 | 对该 skill 添加警告/负例 |
| Better Phrasing | skill_id 非空 + 用户纠正了 skill 的表述 | 对该 skill 的 root.md 措辞修改 |
| New Capability | skill_id 非空 + 某 skill 被用于预期外用途 | 考虑拆分新 skill |
| Obsolete Rule | skill_id 非空 + 某 skill 的规则不再适用 | 标记规则删除 |
| **Workflow Discovery (新增)** | **skill_id 为空** + 相似任务成功 >= 3 次 | **创建全新 workflow skill 提案** |

这是 y-agent 当前设计中唯一能"从无到有"创建 skill 的路径（现有 New Capability 是从已有 skill 的使用中拆分，不是凭空创建）。

**落地步骤**:

1. Experience Record 的 `skill_id` 允许为 null（当前设计已支持）
2. Pattern Extraction 新增 Skillless Experience Analysis 阶段
3. 聚类算法：最简单的实现是用 task_description 的 embedding 做余弦相似度聚类（阈值 > 0.8）
4. Workflow 模板化：LLM 将多条 tool_calls 序列对齐，提取出不变的步骤骨架 + 可变的参数位置
5. 产出的 WorkflowSkillProposal 走现有 Approval Gate（默认 Supervised）

### 4.2 中等价值借鉴

#### (E) Merge 决策的双通道评估

**AutoSkill 方案**: LLM judge（primary）+ 确定性评分（0.70 × semantic + 0.18 × signal + 0.12 × name，fallback），两个通道交叉验证。

**借鉴理由**: y-agent 的 Skill Refinement 设计中，merge 判断主要依赖人工审批。当策略为 auto-minor 或 auto-evaluated 时，缺少详细的自动 merge 判断逻辑。AutoSkill 的双通道方案提供了一个经过实践验证的参考。

**建议**: 在 skill-versioning-evolution-design.md 的 auto 审批路径中，引入类似的双通道 merge 判断。确定性评分部分可以复用 y-agent 已有的 Skill Registry 向量索引。

#### (F) Query Rewrite 提升 Skill 检索质量

**AutoSkill 方案**: 对话上下文中的用户消息常含有指代（"it""this"）和省略，LLM 改写为独立的检索查询。改写 prompt 区分 State A（延续同一任务）和 State B（话题切换），使用 topic anchor（domain + deliverable + operation）作为改写锚点。

**借鉴理由**: y-agent 的 Context Assembly Pipeline 有 compaction 和 memory recall，但在 skill 检索阶段没有专门的 query rewrite 步骤。如果直接用用户的最新消息做检索，指代和省略会严重降低检索质量。

**建议**: 在 y-agent Context Assembly Pipeline 的 Memory Recall 阶段之前，增加 optional 的 Query Rewrite 步骤。这可以作为 ContextMiddleware 链中的一个 middleware 实现。

#### (G) Extraction Gating 的启发式设计

**AutoSkill 方案**: 轻量启发式（ack 检测、关键词重叠、revision hints、显式话题切换标志）控制"是否尝试"，质量过滤留给 Extractor。

**借鉴理由**: y-agent 的 Experience Capture 是"每次执行都记录"，但 Pattern Extraction 需要判断"何时有足够多有意义的经验值得分析"。AutoSkill 的 gating 思路可以用于 y-agent 的 Pattern Extraction 调度——不是固定周期运行，而是在检测到有意义的经验模式变化时触发。

### 4.3 需谨慎对待的方面

#### (H) 全自动无审批的演进

AutoSkill 默认全自动——抽取、merge、version bump 不经人工确认。这适合低风险的对话辅助场景，但对 y-agent（agent 执行有副作用的任务）来说风险较高。

一个低质量 skill 被自动 merge 进来可能导致 agent 在后续任务中做出错误决策。y-agent 的分层审批模型（Supervised → Auto-minor → Auto-evaluated → Autonomous → Frozen）是更安全的设计。

**建议**: 保留 y-agent 现有的审批模型，但在 Auto-evaluated 策略中参考 AutoSkill 的 usage tracking 信号——如果一个 skill 的 usage_rate 持续高于阈值，可以放宽其 evolution proposal 的审批条件。

#### (I) 无 Token 预算的 Skill 格式

AutoSkill 的 SKILL.md 没有 token 预算限制——instructions 可以任意长。在注入时通过 `max_chars` 截断。这对于直接面向人类用户的系统可以接受（检索 top-1 通常足够），但对于需要同时加载多个 skill 的 agent 系统，缺少预算控制会导致上下文膨胀。

y-agent 的根文档 2000 tokens 限制和树形子文档按需加载是更好的设计，不需要修改。

#### (J) 单文件格式 vs 树形结构

AutoSkill 使用单文件 `SKILL.md`，简单直观但无法表达复杂 skill。y-agent 的 root.md + details/ 树形结构更适合复杂 agent 行为的表达，不需要修改。

---

## 五、对 y-agent 的具体修改建议

### 5.1 新增：Fast-Path Skill Extraction

**修改文档**: `skill-versioning-evolution-design.md`

在 Experience Capture → Pattern Extraction → Skill Refinement 的批次管线之外，新增 Fast-Path：

| 路径 | 触发条件 | 证据来源 | 审批 | 适用场景 |
|------|---------|---------|------|---------|
| Batch Path（现有） | 定时/按需 | 多条 Experience Records | 分层审批 | 高风险 agent 任务 |
| **Fast Path（新增）** | 每次交互后异步 | 单次对话窗口 | Auto-minor | 低风险对话辅助/代码生成 |

Fast-Path 的启用由 agent 配置决定（新增 `skill_evolution.fast_path: bool` 配置项），默认关闭。

### 5.2 新增：Evidence Provenance 标注

**修改文档**: `skill-versioning-evolution-design.md`, `memory-architecture-design.md`

Experience Record 新增 `evidence_entries` 字段，每条证据标注来源类别：

| 来源类别 | 定义 | Pattern Extraction 硬规则 |
|----------|------|--------------------------|
| `user_stated` | 用户明确说出的约束/偏好/流程 | 可直接作为 skill 更新依据 |
| `user_correction` | 用户对 agent 输出的纠正 | 最高优先级，可覆盖 user_stated |
| `task_outcome` | 任务成功/失败的客观结果 | 仅作统计信号，不直接作为 skill 内容 |
| `agent_observation` | Agent 自身总结的模式 | 必须有 user_stated 或 user_correction 佐证才可采信 |

不使用数值加权——在 Pattern Extraction 的 LLM prompt 中添加硬规则：来自 `agent_observation` 的模式如果没有用户证据佐证，直接丢弃。

### 5.3 新增：Skill Usage Audit

**修改文档**: `skills-knowledge-design.md`, `hooks-plugin-design.md`

在 Hook 系统的 PostTaskMiddleware 中新增 usage audit 逻辑（双通道：LLM 主通道 + 关键词回退）。audit 结果写入 Skill evaluation metrics：

```toml
[evaluation]
use_count = 142          # 现有: 关联任务数
success_rate = 0.85      # 现有: 任务成功率
injection_count = 89     # 新增: 被注入到上下文的次数
actual_usage_count = 67  # 新增: LLM 真正依赖该 skill 的次数
usage_rate = 0.753       # 新增: actual_usage_count / injection_count
```

LLM 审计的核心判断标准：**如果不注入这个 skill，LLM 也能生成质量相当的回复，则 used=false。**

`usage_rate < 0.1` 触发 Pattern Extraction 的 Obsolete Rule 检测（triggers/tags 定位不准，或 instructions 质量低）。

### 5.4 新增：Skillless Experience Analysis（Workflow Discovery）

**修改文档**: `skill-versioning-evolution-design.md`

在 Pattern Extraction 中新增独立阶段——处理 `skill_id = null` 的成功经验：

1. 筛选 `skill_id = null AND outcome = Success` 的经验
2. 按 `task_description` embedding 相似度聚类（阈值 > 0.8）
3. 聚类中经验数 >= 3 时，LLM 提取 workflow 模板（步骤骨架 + 参数占位符）
4. 产出 WorkflowSkillProposal，走现有 Approval Gate

这是 y-agent 当前设计中唯一能"从无到有"创建 skill 的路径。现有 New Capability 类型是从已有 skill 使用中拆分，不同于从无 skill 的成功执行中发现新 workflow。

### 5.5 可选：Context Pipeline 增加 Query Rewrite

**修改文档**: `context-session-design.md`

在 Context Assembly Pipeline 的 Memory Recall 阶段之前，增加 optional 的 Query Rewrite middleware（ContextMiddleware chain）。当用户消息包含指代或省略时，改写为独立的检索查询。

---

## 六、优先级与依赖关系

| 优先级 | 修改项 | 依赖 | 侵入程度 | 预期收益 |
|--------|--------|------|---------|---------|
| **P0** | Evidence Provenance 标注 | Experience Record 模型 | 低——新增字段 + LLM prompt 硬规则 | 防止 agent 自我强化偏差 |
| **P0** | Skill Usage Audit (双通道) | hooks-plugin PostTaskMiddleware | 低——新增 middleware + 指标字段 | 补全 skill 质量反馈环 |
| **P1** | Fast-Path Extraction | Skill Versioning, Experience Capture | 中——新增旁路通道 | 提升低风险场景的 skill 积累速度 |
| **P1** | Skillless Experience Analysis | Pattern Extraction | 中——新增分析阶段 + 聚类 | 从无到有发现可复用 workflow |
| **P2** | Query Rewrite middleware | ContextMiddleware chain | 低——新增可选 middleware | 提升 skill 检索精度 |
| **P2** | Merge 双通道判断 | Skill Refinement auto 路径 | 低——auto 审批路径增加判断逻辑 | 更可靠的自动 merge 决策 |

---

## 七、总结

AutoSkill 的核心价值在于一个工程化的实时 Skill 演进闭环：**对话 → 抽取 → add/merge/discard → 检索 → 注入 → 使用审计 → stale 剪枝**。这个闭环的每个环节都经过了精心设计：抽取时严格区分用户证据和 agent 生成，merge 判断使用 LLM + 确定性评分双通道，usage tracking 提供真实的 skill 效用信号。

对 y-agent 而言，最有价值的借鉴不是替换现有设计，而是补充现有设计中的四个缺口：

1. **证据可信度缺口**: Experience Record 需要 evidence provenance 标注 + Pattern Extraction 硬规则（agent_observation 必须有用户证据佐证），防止 agent 自我强化偏差
2. **反馈闭环缺口**: Skill 指标需要 injection_count / actual_usage_count / usage_rate，用 LLM 审计（"去掉这个 skill 回复质量是否不变"）判断 skill 是否真正被使用
3. **实时性缺口**: 批次演进管线之外需要 Fast-Path，让低风险场景的 skill 积累不必等待批次作业
4. **从无到有缺口**: Pattern Extraction 只改进已有 skill（按 skill_id 分组），需要新增 Skillless Experience Analysis 阶段，从 skill_id=null 的成功执行中聚类发现可复用 workflow
