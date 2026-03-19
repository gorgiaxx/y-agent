# L1 Section-Level Chunk Generation Plan

## Background

The knowledge system defines three resolution levels:
- **L0**: Document summary (~200 tokens)
- **L1**: Section-level overviews (~500 tokens per section)
- **L2**: Paragraph-level granular chunks

Currently, only L2 is generated during ingestion. L0 (`entry.summary`) and L1 (`entry.overview`) remain `None`. The GUI hardcodes `l1_sections = Vec::new()`, so L1 is never displayed. The front-end rendering code and TypeScript types already exist and are gated on `l1_sections.length > 0`.

## Design Decisions

### D1: L1 Storage — New `l1_sections: Vec<L1Section>` field

Add a structured field to `KnowledgeEntry` for storing L1 section data:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct L1Section {
    pub index: usize,
    pub title: String,
    pub content: String,
}

// In KnowledgeEntry:
#[serde(default)]
pub l1_sections: Vec<L1Section>,
```

- `#[serde(default)]` ensures old `knowledge_entries.json` files deserialize without error
- Keep existing `overview: Option<String>` — fill it with concatenated L1 section text for backward compat with `ProgressiveLoader` and future modules

### D2: L0 — Call existing `chunk_l0()` during ingestion

`chunk_l0()` already truncates to `l0_max_tokens` (200). Call it during ingestion and store in `entry.summary`.

### D3: L1 Title Extraction — Heading-aware with fallback

- If section content starts with `# Heading` / `## Heading`: extract as title (strip `#` prefix)
- Otherwise: `"Section {index + 1}"`

### D4: L1 Indexing — Skip

L1 chunks are **not** indexed into `HybridRetriever` to avoid search redundancy with L2.

### D5: Overview field — Sync from L1

Fill `entry.overview` with a joined text of all L1 section titles, providing a fallback for modules (e.g. `ProgressiveLoader`) that may reference it.

---

## Proposed Changes

### y-knowledge data model

#### [MODIFY] [models.rs](file:///Users/gorgias/Projects/y-agent/crates/y-knowledge/src/models.rs)

1. Add `L1Section` struct (fields: `index`, `title`, `content`) with `Serialize`/`Deserialize`
2. Add `#[serde(default)] pub l1_sections: Vec<L1Section>` to `KnowledgeEntry`
3. Initialize as `Vec::new()` in `KnowledgeEntry::new()`
4. Add tests: serialization roundtrip, backward compat (old JSON missing `l1_sections`)

---

### y-knowledge chunking

#### [MODIFY] [chunking.rs](file:///Users/gorgias/Projects/y-agent/crates/y-knowledge/src/chunking.rs)

Add `pub fn extract_section_title(content: &str, index: usize) -> String`:
- Check if the first non-empty line begins with `#` → extract text after `#` chars, trim
- Fallback: return `"Section {index + 1}"`

Add tests:
- `test_extract_section_title_h1` — input `"# Hello\ncontent"` → `"Hello"`
- `test_extract_section_title_h2` — input `"## Sub\ncontent"` → `"Sub"`
- `test_extract_section_title_fallback` — input `"plain text"` → `"Section 1"`
- `test_extract_section_title_empty` — input `""` → `"Section 1"`

---

### y-knowledge ingestion pipeline

#### [MODIFY] [mod.rs](file:///Users/gorgias/Projects/y-agent/crates/y-knowledge/src/ingestion/mod.rs)

After L2 chunking (line ~123) and before `entry.transition(EntryState::Chunked)` (line ~135), add:

```rust
// Generate L0 summary
let l0_chunks = strategy.chunk(&entry.id.to_string(), &entry.content, ChunkLevel::L0, &metadata);
if let Some(l0) = l0_chunks.first() {
    entry.summary = Some(l0.content.clone());
}

// Generate L1 sections
let l1_chunks = strategy.chunk(&entry.id.to_string(), &entry.content, ChunkLevel::L1, &metadata);
entry.l1_sections = l1_chunks.iter().enumerate().map(|(i, chunk)| {
    L1Section {
        index: i,
        title: extract_section_title(&chunk.content, i),
        content: chunk.content.clone(),
    }
}).collect();

// Sync overview field for backward compat
entry.overview = if entry.l1_sections.is_empty() {
    None
} else {
    Some(entry.l1_sections.iter().map(|s| s.title.as_str()).collect::<Vec<_>>().join(" | "))
};
```

Update existing test assertions:
- `test_pipeline_basic_ingestion`: add assert `entry.summary.is_some()` + `!entry.l1_sections.is_empty()`

Add new tests:
- `test_pipeline_generates_l0_summary` — verify `entry.summary` is `Some` and within L0 token budget
- `test_pipeline_generates_l1_sections` — verify `entry.l1_sections` is non-empty and each section has title + content
- `test_pipeline_l1_section_titles` — markdown doc with headings → verify titles extracted correctly

---

### y-knowledge re-exports

#### [MODIFY] [lib.rs](file:///Users/gorgias/Projects/y-agent/crates/y-knowledge/src/lib.rs)

Add `L1Section` to line 54 re-exports:

```rust
pub use models::{EntryState, KnowledgeCollection, KnowledgeEntry, L1Section, SourceRef};
```

---

### y-service layer (verify only)

#### [VERIFY] [knowledge_service.rs](file:///Users/gorgias/Projects/y-agent/crates/y-service/src/knowledge_service.rs)

No code changes needed. Verify:
- `ingest()` passes data through (it stores the full entry → `l1_sections` included via serde automatically)
- `save_entries()` / `load_entries()` serialize new fields via serde
- `get_entry()` returns entry with populated `l1_sections`

> [!NOTE]
> `knowledge_service.rs:412` strips `entry.content` before persistence but does **not** strip `l1_sections` or `chunks` — L1 data will be persisted correctly.

---

### y-gui Tauri backend

#### [MODIFY] [knowledge.rs](file:///Users/gorgias/Projects/y-agent/crates/y-gui/src-tauri/src/commands/knowledge.rs)

Replace line 287:
```rust
let l1_sections = Vec::new(); // L1 not yet generated
```
With:
```rust
let l1_sections: Vec<SectionInfo> = entry.l1_sections.iter().map(|s| {
    SectionInfo {
        index: s.index,
        title: s.title.clone(),
        summary: s.content.clone(),  // L1 section content maps to SectionInfo.summary
    }
}).collect();
```

> [!NOTE]
> `SectionInfo.summary` (backend) maps to `KnowledgeSection.summary` (frontend) — naming is consistent.

---

### Files NOT changed (already ready)

| File | Reason |
|------|--------|
| [index.ts](file:///Users/gorgias/Projects/y-agent/crates/y-gui/src/types/index.ts) | `KnowledgeSection` type already defined (line 375) |
| [KnowledgePanel.tsx](file:///Users/gorgias/Projects/y-agent/crates/y-gui/src/components/KnowledgePanel.tsx) | L1 rendering implemented (line 296), gated on `l1_sections.length > 0` |
| [config.rs](file:///Users/gorgias/Projects/y-agent/crates/y-knowledge/src/config.rs) | `l1_max_tokens = 500` already configured (line 89) |
| [progressive.rs](file:///Users/gorgias/Projects/y-agent/crates/y-knowledge/src/progressive.rs) | Uses `ChunkingStrategy` at runtime, no persistence dependency |

---

## Verification Plan

### Automated Tests

```bash
# 1. Run y-knowledge unit tests (models + chunking + ingestion)
cargo test -p y-knowledge -- models::tests
cargo test -p y-knowledge -- chunking::tests::test_extract_section_title
cargo test -p y-knowledge -- ingestion::tests

# 2. Run y-service tests (verify serde passthrough)
cargo test -p y-service

# 3. Full workspace compile check (includes y-gui Tauri backend)
cargo check --workspace
```

### Manual Verification

1. **Import a Markdown doc with headings** in the GUI → navigate to entry detail → confirm L0 summary shows real content (not truncated L2), L1 section headers and summaries appear in the collapsible section, L2 chunks display as before
2. **Import a plain text file** → confirm L1 titles fallback to "Section 1", "Section 2", etc.
3. **Restart the app** → open an existing entry → confirm L1 data persists (no deserialization errors, L1 sections still visible)
4. **Open an entry ingested before this change** → confirm no crash, L1 section is simply not displayed (backward compat via `#[serde(default)]`)
