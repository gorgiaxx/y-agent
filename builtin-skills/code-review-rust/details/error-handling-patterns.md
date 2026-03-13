# Error Handling Best Practices in Rust

## Recommended Patterns

### Use `thiserror` for Library Crates

Define domain-specific error types with `thiserror::Error`. Each variant should carry enough context for the caller to act on.

```
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("record not found: {id}")]
    NotFound { id: String },

    #[error("connection failed: {source}")]
    Connection { #[from] source: io::Error },
}
```

### Use `anyhow` for Application Crates (CLI, binaries)

For top-level application code where error types do not need to be matched on by callers, `anyhow::Result` reduces boilerplate.

### Context Over Bare `?`

Always add context when propagating errors across module boundaries:

```
// Bad: bare ? loses context
let data = std::fs::read(path)?;

// Good: adds context
let data = std::fs::read(path)
    .with_context(|| format!("failed to read config from {}", path.display()))?;
```

## Anti-Patterns to Flag

| Anti-Pattern | Issue | Fix |
|-------------|-------|-----|
| `unwrap()` in library code | Panics on failure | Return `Result` |
| `Box<dyn Error>` everywhere | No pattern matching possible | Use typed errors |
| String-only errors | No structured handling | Use enum variants |
| Swallowing errors with `let _ = ...` | Silent failures | Log at minimum, or propagate |
| `panic!` for expected conditions | Crashes the program | Return `Err(...)` |
