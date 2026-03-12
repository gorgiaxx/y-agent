# Fix Skills Design-Implementation Gaps

Fix 6 identified discrepancies between design documents and the actual `y-core`/`y-skills` implementation.

## Proposed Changes

### y-core (core types)

#### [MODIFY] [skill.rs](file:///Users/gorgias/Projects/y-agent/crates/y-core/src/skill.rs)

1. **Gap 3: Classification type enum** — Move `SkillClassificationType` from `y-skills::classifier` to `y-core::skill`. Change `SkillClassification.skill_type` from `String` to `SkillClassificationType`.

2. **Gap 4: Duplicate tags** — Remove `tags` from `SkillClassification`. Tags remain on `SkillManifest` only.

3. **Gap 2: SubDocumentRef path** — Add `path: String` field for the file path, keep `id` as unique identifier.

4. **Gap 6: Skill state** — Move `SkillState` enum from `y-skills::state` to `y-core::skill`. Add `state: Option<SkillState>` to `SkillManifest`.

5. **Gap 6b: SkillManifest root_path** — Add `root_path: Option<String>` field to `SkillManifest` alongside `root_content` for design-aligned file path reference.

---

### y-skills (downstream updates)

#### [MODIFY] [classifier.rs](file:///Users/gorgias/Projects/y-agent/crates/y-skills/src/classifier.rs)
- Re-export `SkillClassificationType` from `y-core` instead of defining it locally.

#### [MODIFY] [state.rs](file:///Users/gorgias/Projects/y-agent/crates/y-skills/src/state.rs)
- Re-export `SkillState` from `y-core` instead of defining it locally.

#### [MODIFY] [manifest.rs](file:///Users/gorgias/Projects/y-agent/crates/y-skills/src/manifest.rs)
- Update `NestedClassification.skill_type` parsing to produce the enum.
- Remove `tags` field from `NestedClassification`.
- Update `SubDocumentRef` construction to include `path`.
- Set `state` and `root_path` on constructed manifests.

#### [MODIFY] [store.rs](file:///Users/gorgias/Projects/y-agent/crates/y-skills/src/store.rs)
- Update test helpers to include new `SubDocumentRef.path` and `state` fields.

#### [MODIFY] [registry.rs](file:///Users/gorgias/Projects/y-agent/crates/y-skills/src/registry.rs)
- Update test helpers to include new fields.

#### [MODIFY] [config.rs](file:///Users/gorgias/Projects/y-agent/crates/y-skills/src/config.rs)
- **Gap 5**: Change `store_path` default from `.y-agent/skills` to `skills` (relative to data dir), add doc comment noting it's resolved against the XDG data directory at runtime.

#### [MODIFY] [lib.rs](file:///Users/gorgias/Projects/y-agent/crates/y-skills/src/lib.rs)
- Update re-exports to include `SkillClassificationType` and `SkillState` from `y-core`.

---

### y-service

#### [MODIFY] [skill_ingestion.rs](file:///Users/gorgias/Projects/y-agent/crates/y-service/src/skill_ingestion.rs)
- Update `SkillClassification` construction to use enum instead of string.
- Add `path` when constructing `SubDocumentRef`.
- Set `state` and `root_path` on output manifests.

---

### y-cli

#### [MODIFY] [skills.rs](file:///Users/gorgias/Projects/y-agent/crates/y-cli/src/commands/skills.rs)
- Update `cls.skill_type` display (now an enum, use `Display` impl or format).

---

## Verification Plan

### Automated Tests

All existing tests must still pass after changes. Run:

```bash
# Core crate
cargo test -p y-core -- skill

# Skills crate (all features)
cargo test -p y-skills --all-features

# Service crate  
cargo test -p y-service --lib -- skill

# Full workspace build
cargo build
```

No new tests needed — existing tests in `manifest.rs` (7 tests), `classifier.rs` (4 tests), `state.rs` (5 tests), `store.rs`, `registry.rs`, and `validator.rs` already cover the affected code paths. The changes are structural (type narrowing, field additions), so passing existing tests validates correctness.
