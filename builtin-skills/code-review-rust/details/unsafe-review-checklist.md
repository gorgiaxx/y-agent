# Unsafe Code Review Checklist

Every `unsafe` block in Rust must satisfy all of the following:

## Mandatory Requirements

- [ ] **SAFETY comment** — A `// SAFETY:` comment immediately above the `unsafe` block explaining why the invariants are upheld
- [ ] **Minimal scope** — The `unsafe` block contains only the operations that require it, nothing else
- [ ] **No undefined behavior** — The code does not invoke any of Rust's UB categories (see below)

## Undefined Behavior Categories to Check

1. **Dangling references** — Ensure all references point to valid, live data
2. **Aliasing violations** — No `&mut T` and `&T` to the same data at the same time
3. **Uninitialized memory** — Never read from `MaybeUninit` before initialization
4. **Invalid values** — A `bool` must be 0 or 1; an enum must be a defined variant
5. **Data races** — Concurrent unsynchronized writes to the same memory
6. **Null pointer dereference** — Validate raw pointers before dereferencing
7. **Misaligned access** — Pointer alignment requirements are met

## FFI-Specific Checks

- [ ] All inputs from C are validated before use
- [ ] Null pointers are checked at the FFI boundary
- [ ] Ownership transfer is documented (who frees what)
- [ ] String encoding assumptions are explicit (UTF-8 vs. arbitrary bytes)
- [ ] Lifetime of borrowed data outlives the FFI call

## Review Questions

Ask yourself for each `unsafe` block:

1. Can this be rewritten in safe Rust? If yes, do so.
2. Is there a safe abstraction in `std` or a well-audited crate that does the same thing?
3. What invariants must the caller maintain for this to be sound?
4. Are those invariants documented and enforced by the type system where possible?
