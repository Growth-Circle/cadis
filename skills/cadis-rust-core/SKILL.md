---
name: cadis-rust-core
description: Use when implementing or reviewing CADIS Rust workspace structure, cadisd daemon, cadis CLI, session core, event bus, store, or core crate boundaries.
---

# CADIS Rust Core

## Read First

- `docs/00_PROJECT_CHARTER.md`
- `docs/04_TRD.md`
- `docs/05_ARCHITECTURE.md`
- `docs/06_IMPLEMENTATION_PLAN.md`
- `docs/11_DECISIONS.md`

## Rules

- Keep core Rust-first.
- Keep `cadisd` as runtime authority.
- Do not add UI framework dependencies to core crates.
- Do not add Node.js to daemon runtime.
- Keep `unsafe` forbidden unless a decision record explicitly allows it.
- Prefer small crates with clear ownership.

## Expected Crate Order

1. `cadis-protocol`
2. `cadis-core`
3. `cadis-daemon`
4. `cadis-cli`
5. `cadis-store`
6. `cadis-policy`

## Validation

When Rust code exists, run:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

If a crate changes public behavior, update the implementation plan and checklist.

