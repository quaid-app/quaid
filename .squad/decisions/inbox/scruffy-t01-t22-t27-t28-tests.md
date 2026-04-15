# Scruffy findings — T01, T22, T27, T28 tests

## Completed

- Added `Page` serde round-trip unit coverage in `src/core/types.rs`.
- Added migrate idempotency regression coverage for the "one file changed" SHA-256 branch in `src/core/migrate.rs`.
- Added `src/lib.rs` to expose `core`, `commands`, and `mcp` to integration tests.
- Added `tests/roundtrip_raw.rs` for byte-exact canonical export.
- Added `tests/roundtrip_semantic.rs` for corpus import/export/re-import verification.
- Added MCP server tests for tools capability smoke coverage, `brain_get` missing-slug error code, and stale `brain_put` OCC error code/data.
- Updated `openspec/changes/p1-core-storage-cli/tasks.md` to mark T01, T22, T27, and T28 complete.

## Decisions / patterns discovered

1. `Page` does not have a dedicated `tags` field in the current tree; tags that matter for serde round-trip coverage are presently represented in `frontmatter`.
2. The raw round-trip test must use a constructed canonical fixture because the shipped fixtures are representative ingest fixtures, not byte-exact canonical render fixtures.
3. The semantic round-trip assertion should compare normalized exported markdown hashes rather than original fixture bytes: the first export can preserve CRLF bytes from imported fixture content, while the canonical re-import/export path normalizes line endings to LF.
4. `src/lib.rs` is the minimal structural fix needed to unblock integration tests without changing the existing binary module layout.

## Validation

- `cargo check`
- `cargo test`
