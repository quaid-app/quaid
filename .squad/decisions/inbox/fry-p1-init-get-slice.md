# Phase 1 Init + Get Slice — T05, T07

**By:** Fry
**Date:** 2026-04-14

## What

Implemented `src/commands/init.rs` (T05) and `src/commands/get.rs` (T07) — the first two usable CLI commands.

### init.rs decisions
1. **Existence check before `db::open`** — `Path::exists()` check before calling `db::open()` prevents re-initializing an existing database. The spec says "print a warning and exit with code 0 without reinitialising the schema", so we short-circuit before touching SQLite.
2. **No schema migration on existing DBs** — `init` is strictly for creation. If the file exists, we don't verify schema version or attempt migration. This keeps init simple and predictable.

### get.rs decisions
1. **`get_page` extracted as public helper** — The page-loading logic is in `get_page(db, slug) -> Result<Page>` rather than inlined in `run()`. This lets `put` (T06) and other commands reuse it for OCC version lookups without circular module dependencies.
2. **Frontmatter stored as JSON** — Schema stores `frontmatter` as a JSON string. `get_page` deserializes it with `serde_json::from_str` and falls back to empty map on malformed JSON (defensive, not panic).
3. **`--json` output** — When `--json` is passed, the full `Page` struct is serialized to JSON via `serde_json::to_string_pretty`. Otherwise, `render_page` produces canonical markdown.

### Wiring (main.rs)
- No changes needed. `main.rs` already dispatches `init` before `db::open()` and passes `cli.json` to `get`. The scaffold was correctly set up in Sprint 0.

## Tests added
- **init:** 3 tests (creation, idempotent re-run, nonexistent parent rejection)
- **get:** 4 tests (data round-trip, markdown render, not-found error, frontmatter deserialization)

## Gate status
- `cargo fmt --check` ✓
- `cargo clippy -- -D warnings` ✓
- `cargo test` — 48/48 pass (41 baseline + 7 new)

## Routing
- **Bender:** `get_page` is now available for integration testing. Round-trip tests can insert a page via SQL, then call `get_page` and verify against `render_page`.
- **T06 (put):** Can import `get_page` to read current version for OCC checks — no DB query duplication needed.
- **Scope held:** No put/list/stats work touched. T06–T12 remain in their current task states.
