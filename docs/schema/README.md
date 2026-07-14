# Runtime Schema Sources

Database schemas are documented by the files that the application actually
loads. A separate hand-maintained SQL design document is intentionally not kept.

## SQLite

- [`crates/y-storage/src/schema.sql`](../../crates/y-storage/src/schema.sql) is
  the embedded operational and diagnostics schema.
- [`crates/y-storage/src/migration.rs`](../../crates/y-storage/src/migration.rs)
  owns compatibility checks and schema initialization.
- `y-storage` tests define expected migration and persistence behavior.

SQLite uses WAL mode and stores local operational state such as sessions,
workflows, schedules, chat checkpoints, diagnostics, and provider metrics.

## Knowledge Vectors

`y-knowledge` owns vector indexing and retrieval. Qdrant support is optional and
feature-gated by `vector_qdrant`; local retrieval is the default path.

When the runtime schema changes, update the embedded schema, migration behavior,
tests, and affected public configuration in the same change.
