### Leela SG-6 Final Fixes — 2026-04-15

**Author:** Leela (Lead)
**Status:** Implemented — pending Nibbler re-review
**Commit:** `ba5fb20` on `phase1/p1-core-storage-cli`

---

## Context

Nibbler rejected `src/mcp/server.rs` twice. Fry is locked out under the reviewer rejection protocol after authoring both the original and the first revision. Leela took direct ownership of the two remaining blockers from Nibbler's second rejection.

---

## Fix 1: OCC create-path guard

**Blocker:** When `brain_put` received `expected_version: Some(n)` for a page that did not exist, the code silently created the page at version 1, ignoring the supplied version. This violates the OCC contract — a client supplying `expected_version` is asserting knowledge of current state; if that state doesn't exist, the call must fail.

**Change:** Added a guard at the top of the `None =>` branch in the `match existing_version` block in `src/mcp/server.rs`. When `input.expected_version` is `Some(n)` and `existing_version` is `None`, the handler returns:
- Error code: `-32009`
- Message: `"conflict: page does not exist at version {n}"`
- Data: `{ "current_version": null }`

**Test added:** `brain_put_rejects_create_with_expected_version_when_page_does_not_exist` — verifies error code `-32009` and `current_version: null` data.

---

## Fix 2: Bounded result materialization

**Blocker:** `search_fts()` materialized every matching row into a `Vec` with no SQL `LIMIT` before returning. `hybrid_search()` consumed that full result set before merging and truncating. The handler-level `results.truncate(limit)` in server.rs was present but ineffective — the DB already did a full table scan and all rows were in memory.

**Change:** Added `limit: usize` parameter to both `search_fts` (in `src/core/fts.rs`) and `hybrid_search` (in `src/core/search.rs`):

- `search_fts`: appends `LIMIT ?n` to the SQL query, pushing the bound into SQLite so only `limit` rows are ever transferred from the DB engine.
- `hybrid_search`: passes `limit` down to `search_fts` and calls `merged.truncate(limit)` after the set-union/RRF merge step.

All callers updated:
- `src/mcp/server.rs`: `brain_query` and `brain_search` compute `limit` (clamped to `MAX_LIMIT`) before the call and pass it in. The now-redundant post-call `truncate` removed.
- `src/commands/search.rs`: passes `limit as usize` to `search_fts`.
- `src/commands/query.rs`: passes `limit as usize` to `hybrid_search`.
- All tests in `src/core/fts.rs` and `src/core/search.rs`: pass `1000` as limit (exceeds any test fixture size; does not change test semantics).

---

## Verification

- `cargo clippy -- -D warnings`: clean
- `cargo test`: 152 unit tests + 2 integration tests pass (was 151; +1 new test for Fix 1)
- Fry's 5 fixes from the previous revision remain intact and untouched
