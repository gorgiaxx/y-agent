# Progressive Knowledge Retrieval Enhancement

借鉴 OpenViking 的 L0/L1/L2 渐进式检索设计，改进 y-agent 的知识注入流程。

## Background

当前 y-agent 的 chat 知识注入**仅使用 L2 chunks** — 直接将原始段落塞入 LLM 上下文。
OpenViking 的设计理念是**渐进式加载**：

| y-agent 当前 | OpenViking 设计 | 差异 |
|-------------|----------------|------|
| 搜索 L2 chunks → 直接注入 | 搜索用 L0 abstract → rerank 用 L1 → 注入用 L0/L1 | y-agent 浪费 token |
| 注入完整段落 | 注入 L0 摘要 + L1 导航信息 | y-agent 缺乏结构化 |
| 无渐进机制 | LLM 可通过工具按需读取 L2 | y-agent 不支持按需深入 |

## Design Decisions

### D1: 结构化上下文注入（核心改进）

改造 `KnowledgeContextProvider` 的注入格式，从「直接塞 L2 原文」改为「L0 摘要 + L1 节概览 + 按需提示」：

```
<knowledge_context>
The following knowledge is relevant to your query. Use KnowledgeSearch tool to get full details.

--- Knowledge Item 1 (relevance: 92%) ---
Source: Rust Error Handling Guide
Summary: Comprehensive guide covering Result type, ? operator, and custom error types.
Sections:
  1. Result Type Basics
  2. The ? Operator
  3. Custom Error Types with thiserror
  4. Error Propagation Patterns

--- Knowledge Item 2 (relevance: 78%) ---
Source: API Authentication
Summary: OAuth 2.0 and JWT token authentication for the REST API.
Sections:
  1. OAuth 2.0 Flow
  2. JWT Token Validation

</knowledge_context>
```

**优势**：
- 相同 token 预算下可以覆盖更多文档（L0 ~100 tokens vs L2 ~300 tokens）
- LLM 能看到文档结构，知道哪些 section 有用
- 配合 `KnowledgeSearch` tool 按需获取 L2 细节

### D2: 搜索结果携带 L0 摘要

`KnowledgeContextItem` 和 `SearchResultItem` 增加 `summary` 字段，
从 entry 的 L0 summary 或 L1 sections 提取。

### D3: 保持 L2 搜索索引不变

继续使用 L2 chunks 做向量/BM25 检索（保证精确匹配），但在注入时用 L0/L1 做呈现。
这与 OpenViking **"向量搜索用 L0/L2 定位，呈现用 L0+L1"** 的思路一致。

---

## Proposed Changes

### y-knowledge middleware

#### [MODIFY] [middleware.rs](file:///Users/gorgias/Projects/y-agent/crates/y-knowledge/src/middleware.rs)

1. `KnowledgeContextItem` — 增加 `summary: Option<String>` 和 `sections: Vec<String>` 字段
2. `InjectKnowledge::retrieve_for_context()` — 检索到 L2 chunks 后，查找对应 entry 的 L0 summary 和 L1 section titles，附加到返回结果
3. `InjectKnowledge` — 新增 `entry_metadata: HashMap<String, EntryMetadata>` 用于存储每个 document_id 对应的 L0/L1 信息
4. 新增 `register_entry_metadata()` 方法，在 ingestion/reindex 时调用
5. `format_chunk()` — 修改格式：
   - 如果有 L0 summary + L1 sections：输出结构化格式（L0 摘要 + L1 section 列表）
   - 如果没有 L0/L1（旧数据）：回退到当前的 L2 原文注入

---

### y-knowledge middleware — entry metadata

#### [MODIFY] [middleware.rs](file:///Users/gorgias/Projects/y-agent/crates/y-knowledge/src/middleware.rs)

新增结构：
```rust
/// Lightweight metadata for progressive context injection.
#[derive(Debug, Clone)]
pub struct EntryMetadata {
    pub title: String,
    pub summary: Option<String>,           // L0
    pub section_titles: Vec<String>,       // L1 titles only
}
```

---

### y-service ingestion — register metadata

#### [MODIFY] [knowledge_service.rs](file:///Users/gorgias/Projects/y-agent/crates/y-service/src/knowledge_service.rs)

1. `ingest()` — 在索引完 L2 chunks 到 retriever 后，调用 `register_entry_metadata()` 注册 L0/L1 信息
2. `reindex_all_entries()` — 同样在 reindex 时注册 metadata

---

### y-context knowledge provider — 结构化注入

#### [MODIFY] [knowledge_provider.rs](file:///Users/gorgias/Projects/y-agent/crates/y-context/src/knowledge_provider.rs)

修改 `format_knowledge_block()`:
- 对于有 L0/L1 的结果：输出 L0 summary + L1 section titles 列表
- 对于无 L0/L1 的结果（旧数据回退）：输出 L2 原文（保持现有行为）
- 添加全局提示语：`"Use KnowledgeSearch tool to get full details for specific sections."`

---

### y-tools KnowledgeSearch — L2 详情按需查询

#### [MODIFY] [knowledge_search.rs](file:///Users/gorgias/Projects/y-agent/crates/y-tools/src/builtin/knowledge_search.rs)

`format_results()` — 增加 `summary` 字段到输出 JSON（如果有 L0 summary）

---

## Files NOT Changed

| File | Reason |
|------|--------|
| `chunking.rs` | L0/L1 生成代码已在上轮实现 |
| `models.rs` | `L1Section` 已在上轮添加 |
| `ingestion/mod.rs` | L0/L1 已在 ingestion 时生成 |
| `KnowledgePanel.tsx` | 前端展示无需改动 |

---

## Verification Plan

### Automated Tests

```bash
# 1. Middleware tests — verify structured context format
cargo test -p y-knowledge -- middleware::tests

# 2. Context provider tests — verify L0/L1 injection
cargo test -p y-context -- knowledge_provider::tests

# 3. Full workspace compile
cargo check --workspace
```

### New Tests to Add

1. **`middleware.rs`**: `test_retrieve_with_entry_metadata` — register metadata, verify context items include summary
2. **`middleware.rs`**: `test_format_with_l0_l1_fallback` — verify fallback to L2 when no metadata registered
3. **`knowledge_provider.rs`**: `test_structured_knowledge_block` — verify output format includes sections

### Manual Verification

1. 启动 y-gui, 导入一个含 Markdown headings 的文档到知识库
2. 在 chat 中选择该知识库 collection, 提问相关内容
3. 查看 terminal 日志中 `knowledge retrieval: injected context` 的 tokens 数量
4. 确认 LLM 可以通过 `KnowledgeSearch` 工具获取更多细节
