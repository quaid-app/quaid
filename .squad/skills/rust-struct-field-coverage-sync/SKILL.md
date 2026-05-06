# Rust Struct Field Coverage Sync

When a shared Rust struct gains a field, coverage honesty depends on two follow-ups landing immediately:

1. Update every hand-built fixture/initializer in unit and integration tests in the same change.
2. Add a serde back-compat test proving legacy payloads without the new field still deserialize to the intended default.

## Why

- Rust catches missing fixture fields at compile time, but coverage runs often discover the drift later than a normal targeted test pass.
- A small back-compat serde test protects persisted JSON/fixture shapes during refactors without overreaching into integration behavior.

## Quaid fit

- `Page` is constructed manually in multiple test seams (`tests\assertions.rs`, core helper tests, MCP/output tests).
- New optional metadata like `superseded_by` should preserve legacy payload reads while keeping compile-time fixture drift loud.
