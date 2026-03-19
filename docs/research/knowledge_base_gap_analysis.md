# Knowledge Base 功能差距分析

> 对比 [knowledge-base-design.md](file:///Users/gorgias/Projects/y-agent/docs/design/knowledge-base-design.md) 设计文档与 [y-knowledge](file:///Users/gorgias/Projects/y-agent/crates/y-knowledge) crate 当前实现的差距

## 总体评估

| 维度 | 设计覆盖 | 当前状态 |
|------|---------|---------|
| 设计文档 | ✅ 完整（1127 行，涵盖架构、流程、数据模型、检索算法） | — |
| 模块计划 | ✅ 完整（[y-knowledge.md](file:///Users/gorgias/Projects/y-agent/docs/plan/modules/y-knowledge.md)） | — |
| 代码实现 | 7 个源文件，~600 行代码 | **Phase 1 部分完成，约 15-20%** |

> [!CAUTION]
> 当前实现仅覆盖设计的骨架部分。核心组件（向量索引、源连接器、Embedding 集成、检索工具、上下文注入中间件）均未实现，知识库处于**不可用**状态。

---

## 1. 数据模型层

| 设计项 | 状态 | 说明 |
|--------|------|------|
| `KnowledgeEntry` (完整字段集) | 🔴 未实现 | 设计要求 18 个字段（id, workspace_id, collection, content, overview, summary, domains, source, quality_score 等），当前仅有简化的 [Chunk](file:///Users/gorgias/Projects/y-agent/crates/y-knowledge/src/chunking.rs#22-36) 结构（6 个字段） |
| `SourceRef` (来源溯源) | 🔴 未实现 | 设计要求 source_type, uri, content_hash, title, author, fetched_at, connector_id，当前 [ChunkMetadata](file:///Users/gorgias/Projects/y-agent/crates/y-knowledge/src/chunking.rs#39-49) 仅有 source (string) |
| `KnowledgeCollection` (知识集合) | 🔴 未实现 | 设计要求集合 CRUD、配置、统计，完全缺失 |
| `DomainTaxonomy` (层级域分类) | 🔴 未实现 | 设计要求树状层级域（如 testing/automation），当前只有单一 domain 字符串 |
| 知识条目状态机 | 🔴 未实现 | 设计定义了 Fetched→Parsed→Chunked→Classified→Filtered→Indexed→Active→Stale→Expired 全生命周期 |

---

## 2. 摄取管道 (Ingestion Pipeline)

| 设计项 | 状态 | 说明 |
|--------|------|------|
| Source Connectors (PDF/Web/API/Text/Markdown) | 🔴 未实现 | 设计定义了 `SourceConnector` trait 和 4 种连接器，无任何实现 |
| Document Parser | 🔴 未实现 | 无 PDF、HTML、Markdown 解析器 |
| Semantic Chunker (Heading/Paragraph/Sliding Window/LLM) | 🟡 部分 | 当前仅按 `\n\n` (L1) 和 `\n` (L2) 简单分割；missing: heading-based、sliding window（含 overlap）、LLM 辅助分割 |
| L0 Summary 生成 | 🟡 部分 | 当前 L0 只是截断前 N 字符；设计要求 LLM 生成 ~100 token 摘要 |
| L1 Overview 生成 | 🟡 部分 | 当前 L1 只是按双换行分割；设计要求 LLM 生成 ~500 token 要点概述 |
| Domain Classifier (规则 + LLM 辅助) | 🔴 未实现 | 设计要求两种模式：keyword 匹配 + LLM 辅助分类 |
| Quality Filter (最小长度/语言检测/去重/一致性) | 🔴 未实现 | 设计要求 5 项质量检查 |
| Ingestion Agent 编排 | 🔴 未实现 | Agent-driven 摄取流程完全缺失 |

---

## 3. 存储与索引 (Storage & Indexing)

| 设计项 | 状态 | 说明 |
|--------|------|------|
| Vector Index (HNSW/Qdrant) | 🔴 Placeholder | [indexer.rs](file:///Users/gorgias/Projects/y-agent/crates/y-knowledge/src/indexer.rs) 只有空的 [VectorIndexer](file:///Users/gorgias/Projects/y-agent/crates/y-knowledge/src/indexer.rs#11-12) struct，无实际功能 |
| Keyword Index (BM25 倒排索引) | 🔴 未实现 | 设计要求 BM25 评分的倒排索引 |
| Domain Index (前缀树) | 🔴 未实现 | 设计要求 prefix tree |
| Freshness Index (B-Tree on timestamp) | 🔴 未实现 | |
| Metadata Index (KV on collection/source_type/tags) | 🔴 未实现 | |
| Embedding 集成 | 🔴 未集成 | `y-core` 有 [EmbeddingProvider](file:///Users/gorgias/Projects/y-agent/crates/y-core/src/embedding.rs#53-75) trait 定义，但 [y-knowledge](file:///Users/gorgias/Projects/y-agent/crates/y-knowledge) **未依赖**也未使用 |
| Batch Embedding | 🔴 未实现 | |

---

## 4. 检索引擎 (Retrieval Engine)

| 设计项 | 状态 | 说明 |
|--------|------|------|
| Vector Search (语义搜索) | 🟡 Mock | [retrieval.rs](file:///Users/gorgias/Projects/y-agent/crates/y-knowledge/src/retrieval.rs) 使用纯内存 substring/keyword 匹配模拟，无向量搜索 |
| Keyword Search (BM25) | 🔴 未实现 | 当前只有单词逐一包含检查，非 BM25 |
| Hybrid Search (RRF 融合) | 🔴 未实现 | 设计要求 Reciprocal Rank Fusion，当前只是单路径 |
| Domain Filter | 🟢 基础 | 简单字符串精确匹配，不支持层级匹配 |
| Freshness Filter | 🟡 占位 | [RetrievalFilter](file:///Users/gorgias/Projects/y-agent/crates/y-knowledge/src/retrieval.rs#17-25) 有 `freshness_after` 字段，但未在搜索逻辑中使用 |
| Quality Boost | 🔴 未实现 | 设计要求 `quality_score ^ 0.5` 加权 |
| Freshness Boost (时间衰减) | 🔴 未实现 | 设计要求 `1 / (1 + decay_rate * days)` |
| Deep Retrieval (LLM sub-query 展开 + MMR) | 🔴 未实现 | Phase 2+ 特性 |
| Multi-Stage Pipeline (4 阶段) | 🔴 未实现 | 设计要求 Candidate Generation → Pre-Filtering → Fusion → Post-Ranking |
| 搜索策略配置 (6 种策略) | 🔴 未实现 | SemanticSearch, KeywordSearch, DomainScoped, CollectionScoped, Hybrid, Deep |

---

## 5. 上下文集成 (Context Integration)

| 设计项 | 状态 | 说明 |
|--------|------|------|
| `InjectKnowledge` ContextMiddleware (priority 350) | 🔴 未实现 | `y-core/hook.rs` 仅有注释提及 350 位，无实际中间件 |
| Domain-Triggered Retrieval | 🔴 未实现 | 自动域触发的知识注入 |
| Context Window Guard Knowledge 预算 (4000 tokens) | 🔴 未实现 | |
| Skill-Referenced Knowledge 解析 | 🔴 未实现 | Skill manifest 的 `knowledge_bases` 引用解析 |

---

## 6. 内置工具 (Built-in Tools)

| 设计项 | 状态 | 说明 |
|--------|------|------|
| `knowledge_search` Tool | 🔴 未实现 | 代码库中未找到任何此工具的注册 |
| `knowledge_lookup` Tool | 🔴 未实现 | |
| `knowledge_ingest` Tool | 🔴 未实现 | |

---

## 7. 知识维护 (Maintenance)

| 设计项 | 状态 | 说明 |
|--------|------|------|
| Re-ingestion (重新摄取) | 🔴 未实现 | |
| Staleness detection (过期检测) | 🔴 未实现 | |
| TTL Expiry (生存时间过期) | 🔴 未实现 | |
| Deduplication (去重：精确/近似/跨源) | 🔴 未实现 | |
| Collection CRUD 操作 | 🔴 未实现 | |

---

## 8. 可观测性 & 安全 (Observability & Security)

| 设计项 | 状态 | 说明 |
|--------|------|------|
| 摄取/检索/注入 metrics | 🔴 未实现 | |
| Hook Points (`kb_ingestion_completed`, `kb_knowledge_retrieved`) | 🔴 未实现 | |
| Event Bus Events | 🔴 未实现 | |
| Workspace 隔离 | 🔴 未实现 | |
| 内容安全过滤 | 🔴 未实现 | |
| 审计日志 | 🔴 未实现 | |

---

## 9. 外部集成

| 设计项 | 状态 | 说明 |
|--------|------|------|
| y-service 集成 (`KnowledgeService`) | 🔴 未实现 | 无其他 crate 依赖 [y-knowledge](file:///Users/gorgias/Projects/y-agent/crates/y-knowledge) |
| GUI 知识管理页面 | 🔴 未实现 | 无 GUI 组件 |
| CLI 命令 (`kb ingest`, `kb collection`, 等) | 🔴 未实现 | |

---

## 已完成的部分

| 组件 | 文件 | 完成度 |
|------|------|--------|
| `ChunkLevel` enum (L0/L1/L2) | [chunking.rs](file:///Users/gorgias/Projects/y-agent/crates/y-knowledge/src/chunking.rs) | ✅ |
| [Chunk](file:///Users/gorgias/Projects/y-agent/crates/y-knowledge/src/chunking.rs#22-36) struct (简化版) | [chunking.rs](file:///Users/gorgias/Projects/y-agent/crates/y-knowledge/src/chunking.rs) | 🟡 字段不完整 |
| [ChunkingStrategy](file:///Users/gorgias/Projects/y-agent/crates/y-knowledge/src/chunking.rs#52-55) (简单文本分割) | [chunking.rs](file:///Users/gorgias/Projects/y-agent/crates/y-knowledge/src/chunking.rs) | 🟡 缺 heading-based 等策略 |
| [KnowledgeConfig](file:///Users/gorgias/Projects/y-agent/crates/y-knowledge/src/config.rs#6-19) (token 限制) | [config.rs](file:///Users/gorgias/Projects/y-agent/crates/y-knowledge/src/config.rs) | 🟡 缺集合配置、刷新策略等 |
| `KnowledgeError` | [error.rs](file:///Users/gorgias/Projects/y-agent/crates/y-knowledge/src/error.rs) | ✅ |
| [ProgressiveLoader](file:///Users/gorgias/Projects/y-agent/crates/y-knowledge/src/progressive.rs#11-14) (级别升级 + 预算) | [progressive.rs](file:///Users/gorgias/Projects/y-agent/crates/y-knowledge/src/progressive.rs) | 🟢 核心逻辑完成 |
| [HybridRetriever](file:///Users/gorgias/Projects/y-agent/crates/y-knowledge/src/retrieval.rs#28-32) (内存子串匹配) | [retrieval.rs](file:///Users/gorgias/Projects/y-agent/crates/y-knowledge/src/retrieval.rs) | 🟡 仅为开发态 mock |
| Unit Tests (T-KB-001 ~ T-KB-003) | chunking.rs, progressive.rs, retrieval.rs | ✅ 12 个测试 |
| [EmbeddingProvider](file:///Users/gorgias/Projects/y-agent/crates/y-core/src/embedding.rs#53-75) trait | [embedding.rs](file:///Users/gorgias/Projects/y-agent/crates/y-core/src/embedding.rs) | ✅ trait 定义完成 |

---

## 按设计 Phased Implementation 对比

| Phase | 设计范围 | 当前状态 | 工作量估计 |
|-------|---------|---------|-----------|
| **Phase 1** | 数据模型、Knowledge Store（向量+元数据）、Collection CRUD、`knowledge_search`/`knowledge_lookup` 工具、PDF/Markdown 连接器、heading-based chunker、规则域分类 | **~15% 完成**（只有简化数据模型和基础 chunking） | 3-4 周 |
| **Phase 2** | Web 连接器、`knowledge_ingest` 工具、LLM 辅助分类、质量过滤+去重、`InjectKnowledge` 中间件、域分类管理 | **0% 完成** | 3-4 周 |
| **Phase 3** | 新鲜度管理（重摄取/过期检测/TTL）、集合配置、RRF 融合排序、Skill 引用知识解析、可观测性 | **0% 完成** | 2-3 周 |
| **Phase 4** | API 连接器、语义分块（LLM 辅助）、高级去重、性能优化、CLI 命令 | **0% 完成** | 2-3 周 |

---

## 10. MaxKB 可借鉴机制

> 来源：[MaxKB 架构分析](file:///Users/gorgias/.gemini/antigravity/brain/49363ce6-b852-4ac6-893e-9e7a88c1a374/maxkb_analysis.md)

### 分片策略

| MaxKB 机制 | 当前 y-agent 状态 | 建议 |
|-----------|-----------------|------|
| **句子边界分片**：按中文标点（`。！；`）+ 换行切割，默认 256 字符/块 | 🔴 当前按 `\n\n`/`\n` 简单分割，不识别句子边界 | **高优**：在 [ChunkingStrategy](file:///Users/gorgias/Projects/y-agent/crates/y-knowledge/src/chunking.rs#52-55) 中实现标点感知分片，参考 MaxKB 的 [MarkChunkHandle](file:///Users/gorgias/Projects/MaxKB/apps/common/chunk/impl/mark_chunk_handle.py#14-39) 正则方案 |
| **Paragraph + Chunk 两层结构**：段落保持完整语义单元，子分片(chunk)用于向量化 | 🟡 有 L0/L1/L2 但含义不同 | 参考此模式：L2 = Paragraph（完整段落），为每个 L2 生成多条子分片向量记录，检索后按 L2 段落去重返回 |
| **chunks ArrayField 缓存分片**：段落记录上缓存分片结果，向量重建无需重新分片 | 🔴 不存在 | 在 `KnowledgeEntry` 中增加 `chunks: Vec<String>` 字段，避免重复分片计算 |

### 检索策略

| MaxKB 机制 | 当前 y-agent 状态 | 建议 |
|-----------|-----------------|------|
| **Blend Search 加法融合**：`comprehensive_score = (1 - cosine_distance) + ts_rank_cd` | 🔴 设计要求 RRF 但未实现 | **高优**：先用加法融合作为 v1 快速落地，后续迭代到 RRF |
| **DISTINCT ON paragraph_id 去重**：子分片检索后按段落只保留最高分 | 🔴 不存在 | 实现检索结果的段落级去重聚合 |
| **similarity 阈值过滤**：低于阈值的结果直接丢弃（默认 0.65） | 🟡 [RetrievalFilter](file:///Users/gorgias/Projects/y-agent/crates/y-knowledge/src/retrieval.rs#17-25) 有 min_score 概念但未使用 | 在检索中启用最小相似度阈值 |
| **directly_return 高置信直接返回**：相似度 > 0.9 时跳过 LLM 直接引用 | 🔴 不存在 | 可选特性：高置信知识直接注入上下文，标注来源 |

### 中文分词

| MaxKB 机制 | 当前 y-agent 状态 | 建议 |
|-----------|-----------------|------|
| **jieba 全模式分词 → tsvector**：中文文本用 jieba 分词后存入 PostgreSQL tsvector | 🔴 设计提到 BM25 但未解决中文分词 | **高优**：Rust 生态可用 `jieba-rs` crate 解决中文分词，生成倒排索引 |
| **文本归一化**：embedding 前移除 emoji、规范化空白 | 🔴 不存在 | 在 embedding 管道入口增加 [normalize_for_embedding](file:///Users/gorgias/Projects/MaxKB/apps/knowledge/vector/base_vector.py#48-55) 预处理 |

### 摄取流程

| MaxKB 机制 | 当前 y-agent 状态 | 建议 |
|-----------|-----------------|------|
| **LLM 生成关联问题(FAQ)**：摄取时用 LLM 为每个段落生成「什么问题会命中这段内容」 | 🔴 不存在 | 整合到 L0 summary 生成流程：不仅生成摘要，还生成触发问题，向量化后增强检索召回 |
| **Celery 异步向量化 + QueueOnce 防重复**：按文档粒度异步并发 | 🔴 不存在 | y-agent 可用 tokio task + 去重锁实现类似机制 |
| **Per-knowledge HNSW 索引**：每个知识库单独创建带 WHERE 条件的 HNSW 索引 | 🔴 不存在 | Qdrant 可用 collection 或 payload index 实现类似分区 |

### 数据模型增强

| MaxKB 机制 | 当前 y-agent 状态 | 建议 |
|-----------|-----------------|------|
| **Problem (FAQ) 模型**：预设问答对，独立向量化，检索时与段落共同参与匹配 | 🔴 设计文档未覆盖 | 考虑在 `KnowledgeCollection` 中增加 FAQ 功能，用户可为高频问题创建精确问答对 |
| **hit_num 命中统计**：段落和问题记录命中次数 | 🔴 设计有 `access_count` 但未实现 | 在 `KnowledgeEntry` 中实现 `access_count` 自增，用于检索排序和管理分析 |
| **is_active 启用/禁用**：段落级别的激活控制 | 🔴 不存在 | 在 `KnowledgeEntry` 中增加 `is_active` 字段，支持不删除地禁用特定条目 |

---

## 建议优先级（含 MaxKB 借鉴）

> [!IMPORTANT]
> 以下按依赖关系和使用价值排列，🆕 标记受 MaxKB 启发的新增/调整项

1. **完善数据模型** — `KnowledgeEntry` 完整字段、`SourceRef`、`KnowledgeCollection`，🆕 增加 `chunks`、`is_active`、`hit_num` 字段
2. **实现 Qdrant 向量索引** — 将 [VectorIndexer](file:///Users/gorgias/Projects/y-agent/crates/y-knowledge/src/indexer.rs#11-12) 从 placeholder 升级为生产实现，复用 [EmbeddingProvider](file:///Users/gorgias/Projects/y-agent/crates/y-core/src/embedding.rs#53-75)
3. **🆕 文本归一化** — 在 embedding 管道入口增加预处理（emoji 移除、空白规范化）
4. **🆕 句子边界分片** — 替换当前 `\n\n`/`\n` 分割为标点感知分片（参考 MaxKB [MarkChunkHandle](file:///Users/gorgias/Projects/MaxKB/apps/common/chunk/impl/mark_chunk_handle.py#14-39)）
5. **实现 Source Connectors** — 至少 Markdown + Text
6. **LLM 辅助 L0/L1 生成** — 🆕 同时生成触发问题(FAQ)，增强检索召回
7. **🆕 中文分词支持** — 引入 `jieba-rs` 实现中文关键词索引
8. **🆕 Blend Search (加法融合)** — 先用 [(1-distance) + keyword_score](file:///Users/gorgias/Projects/MaxKB/apps/knowledge/models/knowledge.py#78-81) 作为 v1 hybrid retrieval，后续迭代到 RRF
9. **🆕 检索结果段落级去重** — 子分片向量检索后按段落 DISTINCT，返回完整段落
10. **实现 `knowledge_search` / `knowledge_lookup` 工具** — 让 Agent 能使用知识库
11. **实现 `InjectKnowledge` ContextMiddleware** — 自动知识注入
12. **Service 层集成** — 在 `y-service` 中创建 `KnowledgeService` 编排各组件
13. **GUI / CLI 支持** — 用户界面和命令行管理
